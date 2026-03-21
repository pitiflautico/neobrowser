//! neo-runtime — JavaScript runtime for executing web page scripts.
//!
//! Wraps deno_core (V8) to execute JavaScript from web pages.
//! This is how NeoRender runs React, Vue, Angular — any SPA.
//! Provides fetch/timer/storage ops, ES module loading, and V8 bytecode caching.

pub mod code_cache;
pub mod mock;
pub mod modules;
pub mod ops;
pub mod scheduler;
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

    /// Number of pending async tasks (promises, timers, fetches).
    fn pending_tasks(&self) -> usize;

    /// Inject HTML into the DOM (parse and set as document).
    /// Also loads bootstrap.js which sets up browser globals (fetch, timers, etc.).
    fn set_document_html(&mut self, html: &str, url: &str) -> Result<(), RuntimeError>;

    /// Export the current DOM state as HTML string.
    /// Returns the outerHTML of document.documentElement after JS execution.
    fn export_html(&mut self) -> Result<String, RuntimeError> {
        self.eval("globalThis.__neorender_export ? __neorender_export() : ''")
    }
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
