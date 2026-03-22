//! neo-runtime — JavaScript runtime for executing web page scripts.
//!
//! Wraps deno_core (V8) to execute JavaScript from web pages.
//! This is how NeoRender runs React, Vue, Angular — any SPA.
//! Provides fetch/timer/storage ops, ES module loading, and V8 bytecode caching.

pub mod code_cache;
pub mod imports;
pub mod mock;
pub mod modules;
pub mod ops;
pub mod scheduler;
pub mod trace;
pub mod v8;
mod v8_runtime_impl;

use std::path::PathBuf;

/// Errors from the JavaScript runtime.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    /// JavaScript evaluation error.
    #[error("eval error: {0}")]
    Eval(String),

    /// Module loading or resolution failed.
    #[error("module error: {0}")]
    Module(String),

    /// Event loop timed out waiting for tasks to settle.
    #[error("event loop timeout after {timeout_ms}ms ({pending} tasks pending)")]
    Timeout {
        /// Configured timeout.
        timeout_ms: u64,
        /// Tasks still pending when timeout hit.
        pending: usize,
    },

    /// V8 engine initialization failure.
    #[error("v8 init error: {0}")]
    Init(String),

    /// DOM injection failed.
    #[error("dom error: {0}")]
    Dom(String),

    /// I/O error (cache, filesystem).
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Opaque handle for terminating a V8 isolate from another thread.
///
/// This is a thread-safe handle that can be sent to a watchdog thread.
/// Calling [`terminate`](RuntimeHandle::terminate) will cause the V8
/// isolate to throw an uncatchable exception on the next JS operation.
pub struct RuntimeHandle {
    pub(crate) inner: deno_core::v8::IsolateHandle,
}

// SAFETY: v8::IsolateHandle is designed for cross-thread termination.
unsafe impl Send for RuntimeHandle {}
unsafe impl Sync for RuntimeHandle {}

impl RuntimeHandle {
    /// Terminate the V8 isolate. Causes the currently executing script
    /// to throw an uncatchable exception.
    pub fn terminate(&self) -> bool {
        self.inner.terminate_execution()
    }

    /// Cancel a previous termination request. Must be called from the
    /// isolate's owning thread after handling the termination.
    pub fn cancel_terminate(&self) {
        self.inner.cancel_terminate_execution();
    }
}

/// JavaScript runtime for executing web page scripts.
pub trait JsRuntime: Send {
    /// Evaluate an expression and return result as string.
    /// The code is wrapped in `try { String(...) } catch(...)` — use for expressions only.
    fn eval(&mut self, code: &str) -> Result<String, RuntimeError>;

    /// Execute a script (statements). Does not return a value.
    /// Use this for inline `<script>` tags which contain statements, not expressions.
    fn execute(&mut self, code: &str) -> Result<(), RuntimeError>;

    /// Load and execute an ES module by URL.
    fn load_module(&mut self, url: &str) -> Result<(), RuntimeError>;

    /// Run the event loop until settled or timeout.
    fn run_until_settled(&mut self, timeout_ms: u64) -> Result<(), RuntimeError>;

    /// Pump the event loop once without waiting for new events.
    ///
    /// Returns `true` if there was work to do (microtasks, macrotasks),
    /// `false` if the loop was idle. Used for aggressive SPA hydration
    /// drainage after script execution — React schedules mount via
    /// `queueMicrotask` / `Promise.then` which need explicit pumping.
    fn pump_event_loop(&mut self) -> Result<bool, RuntimeError> {
        Ok(false)
    }

    /// Number of pending async tasks (promises, timers, fetches).
    fn pending_tasks(&self) -> usize;

    /// Reset timer and callback budgets (call between script exec and settle
    /// so React scheduler gets fresh budget for hydration).
    fn reset_budgets(&mut self) {}

    /// Execute JS that returns a Promise, then drive the event loop until
    /// the promise resolves. Unlike eval() which is fire-and-forget for
    /// async code, this actually WAITS for the promise to complete.
    /// Uses deno_core's with_event_loop_promise internally.
    fn eval_promise(&mut self, _code: &str, _timeout_ms: u64) -> Result<String, RuntimeError> {
        Err(RuntimeError::Eval("eval_promise not implemented".into()))
    }

    /// Inject HTML into the DOM (parse and set as document).
    /// Also loads bootstrap.js which sets up browser globals (fetch, timers, etc.).
    fn set_document_html(&mut self, html: &str, url: &str) -> Result<(), RuntimeError>;

    /// Export the current DOM state as HTML string.
    /// Returns the outerHTML of document.documentElement after JS execution.
    fn export_html(&mut self) -> Result<String, RuntimeError> {
        self.eval("globalThis.__neorender_export ? __neorender_export() : ''")
    }

    // ─── Module store access (for pre-fetch pipeline) ───

    /// Insert a pre-fetched module into the script store.
    fn insert_module(&mut self, url: &str, source: &str);

    /// Check if a module URL is already in the store.
    fn has_module(&self, url: &str) -> bool;

    /// Mark a module URL for stubbing (heavy, non-essential).
    fn mark_stub(&mut self, url: &str);

    /// Get the source code for a module URL (if pre-fetched).
    fn get_module_source(&self, url: &str) -> Option<String>;

    /// List all module URLs currently in the store.
    fn module_urls(&self) -> Vec<String>;

    /// Get a thread-safe handle for V8 isolate termination.
    ///
    /// Returns `None` for mock runtimes that don't have a real V8 isolate.
    /// The watchdog uses this to kill runaway scripts from another thread.
    fn isolate_handle(&mut self) -> Option<RuntimeHandle> {
        None
    }

    /// Drain pending navigation requests from the browser shim.
    ///
    /// Returns JSON strings describing form submits, location changes, etc.
    /// Default: no navigation interception (mock runtimes).
    fn drain_navigation_requests(&mut self) -> Vec<String> {
        vec![]
    }

    /// Get the cookie string for the current page (document.cookie equivalent).
    fn get_cookies(&mut self) -> String {
        String::new()
    }

    /// Set a cookie from a Set-Cookie string.
    fn set_cookie(&mut self, _cookie_str: &str) {}

    /// Set the import map for bare specifier resolution.
    fn set_import_map(&mut self, _map: modules::ImportMap) {}
}

/// Configuration for creating a runtime instance.
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    /// Maximum time for `run_until_settled` in milliseconds.
    pub settle_timeout_ms: u64,
    /// Maximum time for a single script execution in milliseconds.
    pub script_timeout_ms: u64,
    /// Directory for V8 bytecode cache. None disables caching.
    pub cache_dir: Option<PathBuf>,
    /// Path to linkedom JS bundle for DOM polyfill.
    pub linkedom_path: Option<PathBuf>,
    /// Path to bootstrap JS that wires up globals.
    pub bootstrap_path: Option<PathBuf>,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            settle_timeout_ms: 5000,
            script_timeout_ms: 3000,
            cache_dir: None,
            linkedom_path: None,
            bootstrap_path: None,
        }
    }
}
