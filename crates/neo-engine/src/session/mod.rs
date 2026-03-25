//! NeoSession — the main `BrowserEngine` implementation.
//!
//! Wires HTTP, DOM, JS runtime, interaction, extraction, and tracing
//! into the navigation lifecycle.

pub mod bot_detection;
pub mod browser_impl;
mod hydration;
mod pipeline;
mod pipeline_phases;
mod prefetch;
mod script_exec;
mod script_parts;
mod scripts;
mod stub;

use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

use neo_dom::DomEngine;
use neo_extract::{Extractor, WomDocument};
use neo_http::{CookieStore, HttpCache, HttpClient};
use neo_interact::Interactor;
use neo_runtime::JsRuntime;
use neo_trace::Tracer;
use neo_types::NetworkLogEntry;
use std::collections::HashSet;

use crate::config::EngineConfig;
use crate::lifecycle::Lifecycle;
use crate::pipeline::PipelineContext;

/// An entry in the navigation history.
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    /// Page URL.
    pub url: String,
    /// Page title.
    pub title: String,
    /// Timestamp in milliseconds since UNIX epoch.
    pub timestamp: u64,
}

/// Factory for creating fresh V8 runtimes on cross-origin navigation.
///
/// Captures a closure that knows how to build a new `DenoRuntime` with the
/// same HTTP client, cookie store, and configuration used for the initial one.
pub struct RuntimeFactory {
    creator: Box<dyn Fn() -> Result<Box<dyn JsRuntime>, String> + Send>,
}

impl RuntimeFactory {
    /// Create a new factory from a closure.
    pub fn new<F>(f: F) -> Self
    where
        F: Fn() -> Result<Box<dyn JsRuntime>, String> + Send + 'static,
    {
        Self {
            creator: Box::new(f),
        }
    }

    /// Build a fresh runtime using the captured configuration.
    pub fn create_runtime(&self) -> Result<Box<dyn JsRuntime>, String> {
        (self.creator)()
    }
}

/// The main browser engine session.
///
/// Holds all subsystem trait objects and orchestrates the full
/// navigate -> parse -> execute -> extract pipeline.
pub struct NeoSession {
    pub(crate) http: Box<dyn HttpClient>,
    pub(crate) dom: Arc<Mutex<Box<dyn DomEngine>>>,
    pub(crate) runtime: Option<Box<dyn JsRuntime>>,
    pub(crate) interactor: Box<dyn Interactor>,
    pub(crate) extractor: Box<dyn Extractor>,
    pub(crate) tracer: Box<dyn Tracer>,
    pub(crate) lifecycle: Lifecycle,
    pub(crate) config: EngineConfig,
    pub(crate) history_stack: Vec<HistoryEntry>,
    pub(crate) history_index: isize,
    pub(crate) network_log: Vec<NetworkLogEntry>,
    /// Cached WOM from last navigation.
    pub(crate) last_wom: Option<WomDocument>,
    /// Cookie store for cross-navigation persistence.
    pub(crate) cookie_store: Option<Box<dyn CookieStore>>,
    /// HTTP response cache (disk-backed).
    pub(crate) http_cache: Option<Box<dyn HttpCache>>,
    /// Pipeline context for the current navigation (created per navigate()).
    pub(crate) pipeline_ctx: Option<PipelineContext>,
    /// Monotonically increasing page ID, incremented on every navigate().
    pub(crate) page_id: Arc<AtomicU64>,
    /// Current page origin for cross-origin isolation (e.g., "https://example.com").
    pub(crate) current_origin: String,
    /// Factory for creating fresh V8 runtimes on cross-origin navigation.
    pub(crate) runtime_factory: Option<RuntimeFactory>,
    /// Domains detected as Cloudflare-protected (need Chrome transport for API calls).
    pub(crate) cloudflare_domains: HashSet<String>,
    /// Accumulated trace events from script execution and module loading.
    pub(crate) trace_events: Vec<neo_runtime::TraceEvent>,
}

impl NeoSession {
    /// Create a new session from subsystem implementations.
    ///
    /// The DOM is wrapped in `Arc<Mutex<...>>` internally. Use
    /// [`new_shared`] if you need to share the DOM with the interactor.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        http: Box<dyn HttpClient>,
        dom: Box<dyn DomEngine>,
        runtime: Option<Box<dyn JsRuntime>>,
        interactor: Box<dyn Interactor>,
        extractor: Box<dyn Extractor>,
        tracer: Box<dyn Tracer>,
        lifecycle_tracer: Box<dyn Tracer>,
        config: EngineConfig,
    ) -> Self {
        Self {
            http,
            dom: Arc::new(Mutex::new(dom)),
            runtime,
            interactor,
            extractor,
            tracer,
            lifecycle: Lifecycle::new(lifecycle_tracer),
            config,
            history_stack: Vec::new(),
            history_index: -1,
            network_log: Vec::new(),
            last_wom: None,
            cookie_store: None,
            http_cache: None,
            pipeline_ctx: None,
            page_id: Arc::new(AtomicU64::new(0)),
            current_origin: String::new(),
            runtime_factory: None,
            cloudflare_domains: HashSet::new(),
            trace_events: Vec::new(),
        }
    }

    /// Create a session with a shared DOM reference.
    ///
    /// The same `Arc<Mutex<...>>` can be given to a [`DomInteractor`]
    /// so that interactions mutate the same DOM the session reads from.
    #[allow(clippy::too_many_arguments)]
    pub fn new_shared(
        http: Box<dyn HttpClient>,
        dom: Arc<Mutex<Box<dyn DomEngine>>>,
        runtime: Option<Box<dyn JsRuntime>>,
        interactor: Box<dyn Interactor>,
        extractor: Box<dyn Extractor>,
        tracer: Box<dyn Tracer>,
        lifecycle_tracer: Box<dyn Tracer>,
        config: EngineConfig,
    ) -> Self {
        Self {
            http,
            dom,
            runtime,
            interactor,
            extractor,
            tracer,
            lifecycle: Lifecycle::new(lifecycle_tracer),
            config,
            history_stack: Vec::new(),
            history_index: -1,
            network_log: Vec::new(),
            last_wom: None,
            cookie_store: None,
            http_cache: None,
            pipeline_ctx: None,
            page_id: Arc::new(AtomicU64::new(0)),
            current_origin: String::new(),
            runtime_factory: None,
            cloudflare_domains: HashSet::new(),
            trace_events: Vec::new(),
        }
    }

    /// Pipeline context from the most recent navigation (if any).
    pub fn pipeline_context(&self) -> Option<&PipelineContext> {
        self.pipeline_ctx.as_ref()
    }

    /// Attach a cookie store for cross-navigation cookie persistence.
    pub fn with_cookie_store(mut self, store: Box<dyn CookieStore>) -> Self {
        self.cookie_store = Some(store);
        self
    }

    /// Attach an HTTP cache for conditional requests and freshness.
    pub fn with_http_cache(mut self, cache: Box<dyn HttpCache>) -> Self {
        self.http_cache = Some(cache);
        self
    }

    /// Attach a runtime factory for cross-origin V8 isolation.
    ///
    /// When navigating cross-origin, the session will destroy the current
    /// runtime and create a fresh one using this factory.
    pub fn with_runtime_factory(mut self, factory: RuntimeFactory) -> Self {
        self.runtime_factory = Some(factory);
        self
    }

    /// Import cookies into the cookie store.
    ///
    /// No-op if no cookie store is attached.
    pub fn import_cookies(&self, cookies: &[neo_types::Cookie]) {
        if let Some(ref store) = self.cookie_store {
            store.import(cookies);
        }
    }

    /// Navigation history as URL list.
    pub fn history_urls(&self) -> Vec<String> {
        self.history_stack.iter().map(|e| e.url.clone()).collect()
    }

    /// Full history stack.
    pub fn history_stack(&self) -> &[HistoryEntry] {
        &self.history_stack
    }

    /// Network log of all requests made.
    pub fn network_log(&self) -> &[NetworkLogEntry] {
        &self.network_log
    }
}
