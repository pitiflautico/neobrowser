//! DenoRuntime — V8-backed implementation of JsRuntime trait.
//!
//! Creates a deno_core::JsRuntime with browser polyfills (linkedom DOM),
//! ES module support via NeoModuleLoader, and V8 bytecode caching.

use crate::code_cache::V8CodeCache;
use crate::modules::{NeoModuleLoader, ScriptStoreHandle};
use crate::ops;
use crate::scheduler::{SchedulerConfig, TaskTracker, TimerBudget};
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
    /// Task tracker for pending async work.
    pub(crate) tracker: TaskTracker,
    /// Timer budget for per-page tick limits.
    pub(crate) timer_budget: TimerBudget,
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

        let code_cache = config
            .cache_dir
            .as_ref()
            .and_then(|dir| V8CodeCache::new(dir).ok())
            .map(Rc::new);

        let loader = NeoModuleLoader {
            store: store.clone(),
            code_cache,
            page_origin: page_origin.clone(),
        };

        let mut runtime = deno_core::JsRuntime::new(RuntimeOptions {
            extensions: vec![neo_runtime_ext::init_ops()],
            module_loader: Some(Rc::new(loader)),
            ..Default::default()
        });

        let tracker = TaskTracker::new();
        let timer_budget = TimerBudget::new(scheduler_config.timer_budget);

        // Put HttpClient and other state in OpState for ops.
        {
            let op_state = runtime.op_state();
            let mut state = op_state.borrow_mut();
            if let Some(client) = http_client {
                state.put(ops::SharedHttpClient(client));
            }
            state.put(ops::ConsoleBuffer::default());
            state.put(ops::StorageState::default());
            // Scheduler state — shared with ops via Arc atomics.
            state.put(tracker.clone());
            state.put(timer_budget.clone());
            state.put(ops::OpsSchedulerConfig {
                interval_max_ticks: scheduler_config.interval_max_ticks as u32,
            });
        }

        // Node.js polyfills required by linkedom (Buffer, process, atob/btoa).
        let node_polyfills: &str = include_str!("../../../js/node_polyfills.js");
        runtime
            .execute_script("<neorender:node_polyfills>", node_polyfills.to_string())
            .map_err(|e| {
                RuntimeError::Init(format!("node polyfills: {}", first_line(&e.to_string())))
            })?;

        // Load linkedom DOM implementation (included at compile time).
        let linkedom_js: &str = include_str!("../../../js/linkedom.js");
        runtime
            .execute_script("<neorender:linkedom>", linkedom_js.to_string())
            .map_err(|e| {
                RuntimeError::Init(format!("linkedom load: {}", first_line(&e.to_string())))
            })?;

        let tokio_rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| RuntimeError::Init(e.to_string()))?;

        Ok(Self {
            runtime,
            store,
            page_origin,
            tracker,
            timer_budget,
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
}

// JsRuntime trait impl is in v8_runtime_impl.rs
