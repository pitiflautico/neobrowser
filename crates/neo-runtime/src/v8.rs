//! DenoRuntime — V8-backed implementation of JsRuntime trait.
//!
//! Creates a deno_core::JsRuntime with browser polyfills (linkedom DOM),
//! ES module support via NeoModuleLoader, and V8 bytecode caching.

use crate::code_cache::V8CodeCache;
use crate::modules::{ImportMapHandle, ModuleTracker, NeoModuleLoader, ScriptStoreHandle};
use crate::ops;
use crate::scheduler::{FetchBudget, SchedulerConfig, TaskTracker, TimerBudget, TimerState};
use crate::v8_runtime_impl::first_line;
use crate::{RuntimeConfig, RuntimeError};
use deno_core::RuntimeOptions;
use neo_http::HttpClient;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

// ─── Extension declaration ───

deno_core::extension!(
    neo_runtime_ext,
    ops = [
        ops::op_fetch,
        ops::op_timer,
        ops::op_timer_register,
        ops::op_timer_fire,
        ops::op_scheduler_config,
        ops::op_storage_get,
        ops::op_storage_set,
        ops::op_storage_remove,
        ops::op_console_log,
        ops::op_navigation_request,
        ops::op_cookie_get,
        ops::op_cookie_set,
        ops::op_yield,
        ops::op_sleep_ms,
    ],
);

// ─── DenoRuntime ───

/// V8-backed JavaScript runtime using deno_core.
pub struct DenoRuntime {
    /// The underlying deno_core runtime.
    pub(crate) runtime: deno_core::JsRuntime,
    /// Shared script store for module loading.
    pub(crate) store: ScriptStoreHandle,
    /// Shared page origin for module resolution (R7d).
    pub(crate) page_origin: crate::modules::PageOriginHandle,
    /// Shared import map for bare specifier resolution.
    pub(crate) import_map: ImportMapHandle,
    /// Task tracker for pending async work.
    pub(crate) tracker: TaskTracker,
    /// Timer budget for per-page tick limits.
    pub(crate) timer_budget: TimerBudget,
    /// Fetch budget for per-page concurrency and timeout limits.
    pub(crate) fetch_budget: FetchBudget,
    /// Module lifecycle tracker for quiescence detection.
    pub(crate) module_tracker: ModuleTracker,
    /// Tokio runtime for blocking on async ops.
    pub(crate) tokio_rt: tokio::runtime::Runtime,
}

// SAFETY: DenoRuntime is only used from a single thread at a time.
// deno_core::JsRuntime is !Send due to V8 isolate, but we ensure
// single-threaded access through our API.
unsafe impl Send for DenoRuntime {}

impl DenoRuntime {
    /// Create a new V8 runtime with the given configuration.
    pub fn new(config: &RuntimeConfig) -> Result<Self, RuntimeError> {
        Self::new_inner(config, None, SchedulerConfig::default())
    }

    /// Create a new V8 runtime with an HttpClient for op_fetch.
    ///
    /// The HttpClient is stored in OpState so that JavaScript `fetch()`
    /// calls route through the real HTTP layer.
    pub fn new_with_http(
        config: &RuntimeConfig,
        http_client: Arc<dyn HttpClient>,
    ) -> Result<Self, RuntimeError> {
        Self::new_inner(config, Some(http_client), SchedulerConfig::default())
    }

    /// Create a new V8 runtime with custom scheduler configuration.
    pub fn new_with_scheduler(
        config: &RuntimeConfig,
        http_client: Option<Arc<dyn HttpClient>>,
        scheduler_config: SchedulerConfig,
    ) -> Result<Self, RuntimeError> {
        Self::new_inner(config, http_client, scheduler_config)
    }

    fn new_inner(
        config: &RuntimeConfig,
        http_client: Option<Arc<dyn HttpClient>>,
        scheduler_config: SchedulerConfig,
    ) -> Result<Self, RuntimeError> {
        let store = Rc::new(RefCell::new(crate::modules::ScriptStore::default()));
        let page_origin = Rc::new(RefCell::new(String::new()));
        let import_map: ImportMapHandle = Rc::new(RefCell::new(None));

        let code_cache = config
            .cache_dir
            .as_ref()
            .and_then(|dir| V8CodeCache::new(dir).ok())
            .map(Rc::new);

        let module_tracker = ModuleTracker::new();

        let loader = NeoModuleLoader {
            store: store.clone(),
            code_cache,
            page_origin: page_origin.clone(),
            import_map: import_map.clone(),
            http_client: http_client.clone(),
            on_demand_count: RefCell::new(0),
            module_tracker: module_tracker.clone(),
        };

        // Create tokio runtime BEFORE JsRuntime so that deno_core's WebTimers
        // have access to the tokio reactor from the very first execute_script call.
        let tokio_rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| RuntimeError::Init(e.to_string()))?;
        let _guard = tokio_rt.enter();

        let mut runtime = deno_core::JsRuntime::new(RuntimeOptions {
            extensions: vec![neo_runtime_ext::init_ops()],
            module_loader: Some(Rc::new(loader)),
            ..Default::default()
        });

        let tracker = TaskTracker::new();
        let timer_budget = TimerBudget::new(scheduler_config.timer_budget);
        let fetch_budget = FetchBudget::default();

        // Put HttpClient and other state in OpState for ops.
        {
            let op_state = runtime.op_state();
            let mut state = op_state.borrow_mut();
            if let Some(client) = http_client {
                state.put(ops::SharedHttpClient(client));
            }
            state.put(ops::ConsoleBuffer::default());
            state.put(ops::StorageState::default());
            state.put(ops::NavigationQueue::default());
            state.put(ops::CookieState::default());
            // Scheduler state — shared with ops via Arc atomics.
            state.put(tracker.clone());
            state.put(timer_budget.clone());
            state.put(fetch_budget.clone());
            state.put(ops::OpsSchedulerConfig {
                interval_max_ticks: scheduler_config.interval_max_ticks as u32,
            });
            state.put(TimerState::new());
        }

        // Node.js polyfills required by DOM implementations (Buffer, process, atob/btoa).
        let node_polyfills: &str = include_str!("../../../js/node_polyfills.js");
        runtime
            .execute_script("<neorender:node_polyfills>", node_polyfills.to_string())
            .map_err(|e| {
                RuntimeError::Init(format!("node polyfills: {}", first_line(&e.to_string())))
            })?;

        // Pre-polyfills for happy-dom (globals it expects before loading).
        let pre_happydom: &str = include_str!("../../../js/pre-happydom.js");
        runtime
            .execute_script("<neorender:pre_happydom>", pre_happydom.to_string())
            .map_err(|e| {
                RuntimeError::Init(format!("pre-happydom: {}", first_line(&e.to_string())))
            })?;

        // Load happy-dom DOM implementation (replaces linkedom).
        let happydom_js: &str = include_str!("../../../js/happy-dom.bundle.js");
        runtime
            .execute_script("<neorender:happydom>", happydom_js.to_string())
            .map_err(|e| {
                let full = e.to_string();
                // Print full error for debugging, return first line for error type
                eprintln!("[happy-dom init error] {}", &full[..full.len().min(500)]);
                RuntimeError::Init(format!("happy-dom load: {}", first_line(&full)))
            })?;

        // Export happy-dom classes to globalThis so bootstrap.js and page scripts find them.
        let export_js = r#"
            if (typeof happydom !== 'undefined') {
                const _hd = happydom;
                const _exports = [
                    'EventTarget','Node','Element','HTMLElement','Text','Comment','DocumentFragment',
                    'Event','CustomEvent','MouseEvent','KeyboardEvent','FocusEvent','InputEvent',
                    'UIEvent','ErrorEvent','SubmitEvent','WheelEvent','AnimationEvent',
                    'MutationObserver','IntersectionObserver','ResizeObserver',
                    'DOMParser','XMLSerializer','Range','Selection',
                    'NodeList','HTMLCollection','DOMTokenList','NamedNodeMap',
                    'Attr','CSSStyleDeclaration','CSSStyleSheet',
                    'Blob','File','FileReader','FormData',
                    'Headers','Response','URL','URLSearchParams','MediaQueryList','Storage',
                    'HTMLDivElement','HTMLSpanElement','HTMLInputElement','HTMLButtonElement',
                    'HTMLAnchorElement','HTMLFormElement','HTMLSelectElement','HTMLOptionElement',
                    'HTMLTextAreaElement','HTMLImageElement','HTMLScriptElement','HTMLStyleElement',
                    'HTMLLinkElement','HTMLMetaElement','HTMLIFrameElement','HTMLTemplateElement',
                    'HTMLTableElement','HTMLTableRowElement','HTMLTableCellElement',
                    'HTMLLabelElement','HTMLCanvasElement','HTMLVideoElement','HTMLAudioElement',
                    'SVGElement','SVGSVGElement','HTMLDialogElement',
                    'Document','Window','BrowserWindow',
                ];
                for (const name of _exports) {
                    if (_hd[name] && !globalThis[name]) globalThis[name] = _hd[name];
                }
            }
        "#;
        runtime
            .execute_script("<neorender:happydom_exports>", export_js.to_string())
            .map_err(|e| {
                RuntimeError::Init(format!("happy-dom exports: {}", first_line(&e.to_string())))
            })?;

        // Critical polyfills that must be set AFTER all other init.
        // deno_core 0.311's V8 Promise doesn't have .finally (removed from snapshot).
        // Must be set here because happy-dom or other init may reset Promise.prototype.
        let polyfills = r#"
            if (typeof Promise.prototype.finally !== 'function') {
                Promise.prototype.finally = function(onFinally) {
                    return this.then(
                        function(v) { return Promise.resolve(onFinally()).then(function() { return v; }); },
                        function(r) { return Promise.resolve(onFinally()).then(function() { throw r; }); }
                    );
                };
            }
        "#;
        runtime
            .execute_script("<neorender:critical_polyfills>", polyfills.to_string())
            .map_err(|e| {
                RuntimeError::Init(format!("polyfills: {}", first_line(&e.to_string())))
            })?;

        // Load turbo-stream decoder AFTER happy-dom exports (needs Blob, ReadableStream).
        let turbo_stream_js: &str = include_str!("../../../js/turbo-stream.bundle.js");
        runtime
            .execute_script("<neorender:turbo_stream>", turbo_stream_js.to_string())
            .map_err(|e| {
                eprintln!("[turbo-stream init] {}", &e.to_string()[..e.to_string().len().min(200)]);
                RuntimeError::Init(format!("turbo-stream: {}", first_line(&e.to_string())))
            })?;

        Ok(Self {
            runtime,
            store,
            page_origin,
            import_map,
            tracker,
            timer_budget,
            fetch_budget,
            module_tracker,
            tokio_rt,
        })
    }

    /// Access the shared script store to add pre-fetched scripts.
    pub fn script_store(&self) -> &ScriptStoreHandle {
        &self.store
    }

    /// Access the task tracker.
    pub fn tracker(&self) -> &TaskTracker {
        &self.tracker
    }

    /// Access the timer budget.
    pub fn timer_budget(&self) -> &TimerBudget {
        &self.timer_budget
    }

    /// Access the fetch budget.
    pub fn fetch_budget(&self) -> &FetchBudget {
        &self.fetch_budget
    }

    /// Access the module tracker for lifecycle instrumentation.
    pub fn module_tracker(&self) -> &ModuleTracker {
        &self.module_tracker
    }
}

// JsRuntime trait impl is in v8_runtime_impl.rs
