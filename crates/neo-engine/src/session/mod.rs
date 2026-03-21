//! NeoSession — the main `BrowserEngine` implementation.
//!
//! Wires HTTP, DOM, JS runtime, interaction, extraction, and tracing
//! into the navigation lifecycle.

mod browser_impl;
mod hydration;
mod pipeline;
mod prefetch;
mod script_exec;
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
