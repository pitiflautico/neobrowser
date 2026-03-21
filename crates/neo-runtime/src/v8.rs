//! DenoRuntime — V8-backed implementation of JsRuntime trait.
//!
//! Creates a deno_core::JsRuntime with browser polyfills (linkedom DOM),
//! ES module support via NeoModuleLoader, and V8 bytecode caching.

use crate::code_cache::V8CodeCache;
use crate::modules::{NeoModuleLoader, ScriptStoreHandle};
use crate::ops;
use crate::scheduler::{SchedulerConfig, TaskTracker, TimerBudget};
use crate::{JsRuntime as JsRuntimeTrait, RuntimeConfig, RuntimeError};
use deno_core::{PollEventLoopOptions, RuntimeOptions};
use neo_http::HttpClient;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

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
    runtime: deno_core::JsRuntime,
    /// Shared script store for module loading.
    store: ScriptStoreHandle,
    /// Task tracker for pending async work.
    tracker: TaskTracker,
    /// Timer budget for per-page tick limits.
    timer_budget: TimerBudget,
    /// Tokio runtime for blocking on async ops.
    tokio_rt: tokio::runtime::Runtime,
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

        let code_cache = config
            .cache_dir
            .as_ref()
            .and_then(|dir| V8CodeCache::new(dir).ok())
            .map(Rc::new);

        let loader = NeoModuleLoader {
            store: store.clone(),
            code_cache,
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

impl JsRuntimeTrait for DenoRuntime {
    fn eval(&mut self, code: &str) -> Result<String, RuntimeError> {
        let wrapped = format!(
            "try {{ String({}) }} catch(__e) {{ 'Error: ' + __e.message }}",
            code
        );
        let result = self
            .runtime
            .execute_script("<eval>", wrapped)
            .map_err(|e| RuntimeError::Eval(first_line(&e.to_string())))?;

        let scope = &mut self.runtime.handle_scope();
        let local = deno_core::v8::Local::new(scope, result);
        if let Some(s) = local.to_string(scope) {
            Ok(s.to_rust_string_lossy(scope))
        } else {
            Ok("undefined".to_string())
        }
    }

    fn execute(&mut self, code: &str) -> Result<(), RuntimeError> {
        let wrapped = format!("try {{ {} }} catch(__e) {{ /* non-fatal */ }}", code);
        self.runtime
            .execute_script("<script>", wrapped)
            .map_err(|e| RuntimeError::Eval(first_line(&e.to_string())))?;
        Ok(())
    }

    fn load_module(&mut self, url: &str) -> Result<(), RuntimeError> {
        let specifier = deno_core::ModuleSpecifier::parse(url)
            .map_err(|e| RuntimeError::Module(e.to_string()))?;

        self.tokio_rt.block_on(async {
            let mod_id = self
                .runtime
                .load_main_es_module(&specifier)
                .await
                .map_err(|e| RuntimeError::Module(first_line(&e.to_string())))?;

            let eval = self.runtime.mod_evaluate(mod_id);

            self.runtime
                .run_event_loop(PollEventLoopOptions::default())
                .await
                .map_err(|e| {
                    RuntimeError::Module(format!("event loop: {}", first_line(&e.to_string())))
                })?;

            eval.await
                .map_err(|e| RuntimeError::Module(first_line(&e.to_string())))?;

            Ok(())
        })
    }

    fn run_until_settled(&mut self, timeout_ms: u64) -> Result<(), RuntimeError> {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);

        self.tokio_rt.block_on(async {
            loop {
                // Run V8 event loop for up to 100ms per iteration.
                let loop_timeout = Duration::from_millis(100)
                    .min(deadline.saturating_duration_since(Instant::now()));

                match tokio::time::timeout(
                    loop_timeout,
                    self.runtime.run_event_loop(PollEventLoopOptions::default()),
                )
                .await
                {
                    Ok(Ok(())) => {
                        // Event loop completed — no more pending work in V8.
                        // Check our tracker too (might have timer ops queued).
                        if self.tracker.is_settled() {
                            return Ok(());
                        }
                        // Tracker still has pending work but V8 event loop
                        // thinks it's done — this means timers are processing.
                        // Continue looping to let microtasks drain.
                    }
                    Ok(Err(e)) => {
                        // Non-fatal event loop errors (React internals, etc.)
                        eprintln!(
                            "[neo-runtime] event loop error (non-fatal): {}",
                            first_line(&e.to_string())
                        );
                        return Ok(());
                    }
                    Err(_) => {
                        // Timeout on this iteration — check overall deadline.
                    }
                }

                // Check: overall timeout?
                if Instant::now() >= deadline {
                    let pending = self.tracker.pending();
                    if pending > 0 {
                        return Err(RuntimeError::Timeout {
                            timeout_ms,
                            pending,
                        });
                    }
                    return Ok(());
                }
            }
        })
    }

    fn pending_tasks(&self) -> usize {
        self.tracker.pending()
    }

    fn set_document_html(&mut self, html: &str, url: &str) -> Result<(), RuntimeError> {
        // Reset timer budget for new page.
        self.timer_budget.reset();
        self.tracker.reset();

        let escaped = html
            .replace('\\', "\\\\")
            .replace('`', "\\`")
            .replace("${", "\\${");
        let escaped_url = url.replace('\'', "\\'");
        let js = format!(
            "globalThis.__neorender_html = `{}`;\
             globalThis.__neorender_url = '{}';",
            escaped, escaped_url
        );
        self.runtime
            .execute_script("<set_document_html>", js)
            .map_err(|e| RuntimeError::Dom(first_line(&e.to_string())))?;

        // Load bootstrap.js — parses HTML via linkedom, sets up browser globals
        // (fetch, timers, console, DOM constructors, etc.).
        let bootstrap_js: &str = include_str!("../../../js/bootstrap.js");
        self.runtime
            .execute_script("<neorender:bootstrap>", bootstrap_js.to_string())
            .map_err(|e| RuntimeError::Dom(format!("bootstrap: {}", first_line(&e.to_string()))))?;

        // Set location to match the page URL.
        let loc_js = format!(
            "try {{\
                const __u = new URL('{}');\
                location.href = __u.href;\
                location.protocol = __u.protocol;\
                location.host = __u.host;\
                location.hostname = __u.hostname;\
                location.port = __u.port;\
                location.pathname = __u.pathname;\
                location.search = __u.search;\
                location.hash = __u.hash;\
                location.origin = __u.origin;\
             }} catch(e) {{}}",
            escaped_url
        );
        self.runtime
            .execute_script("<set_location>", loc_js)
            .map_err(|e| RuntimeError::Dom(first_line(&e.to_string())))?;

        Ok(())
    }

    fn export_html(&mut self) -> Result<String, RuntimeError> {
        self.eval("globalThis.__neorender_export ? __neorender_export() : ''")
    }
}

/// Extract the first line of an error message.
fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or(s).to_string()
}
