//! MockRuntime — test double for JsRuntime trait.
//!
//! No real V8 — for testing engine/interact layers without heavy deps.
//! Configurable return values and call recording.

use crate::{JsRuntime, RuntimeError};
use std::collections::HashMap;

/// Mock JavaScript runtime for testing.
///
/// Records all calls and returns configurable values.
/// No V8 or deno_core dependency — fast to construct.
pub struct MockRuntime {
    /// Configured return values for eval() keyed by input code.
    eval_results: HashMap<String, String>,
    /// Default return value for eval() when no specific match.
    default_eval: String,
    /// Recorded eval() calls.
    pub eval_calls: Vec<String>,
    /// Recorded load_module() calls.
    pub module_calls: Vec<String>,
    /// Recorded set_document_html() calls: (html, url).
    pub html_calls: Vec<(String, String)>,
    /// Simulated pending task count.
    pub pending: usize,
    /// If set, eval() returns this error.
    pub eval_error: Option<String>,
}

impl Default for MockRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl MockRuntime {
    /// Create a new mock runtime with no configured results.
    pub fn new() -> Self {
        Self {
            eval_results: HashMap::new(),
            default_eval: "undefined".to_string(),
            eval_calls: Vec::new(),
            module_calls: Vec::new(),
            html_calls: Vec::new(),
            pending: 0,
            eval_error: None,
        }
    }

    /// Configure a return value for a specific eval input.
    pub fn on_eval(&mut self, code: &str, result: &str) {
        self.eval_results
            .insert(code.to_string(), result.to_string());
    }

    /// Set the default return value for eval().
    pub fn set_default_eval(&mut self, result: &str) {
        self.default_eval = result.to_string();
    }
}

impl JsRuntime for MockRuntime {
    fn eval(&mut self, code: &str) -> Result<String, RuntimeError> {
        self.eval_calls.push(code.to_string());
        if let Some(err) = &self.eval_error {
            return Err(RuntimeError::Eval(err.clone()));
        }
        let result = self
            .eval_results
            .get(code)
            .cloned()
            .unwrap_or_else(|| self.default_eval.clone());
        Ok(result)
    }

    fn execute(&mut self, code: &str) -> Result<(), RuntimeError> {
        self.eval_calls.push(code.to_string());
        if let Some(err) = &self.eval_error {
            return Err(RuntimeError::Eval(err.clone()));
        }
        Ok(())
    }

    fn load_module(&mut self, url: &str) -> Result<(), RuntimeError> {
        self.module_calls.push(url.to_string());
        Ok(())
    }

    fn run_until_settled(&mut self, _timeout_ms: u64) -> Result<(), RuntimeError> {
        self.pending = 0;
        Ok(())
    }

    fn pending_tasks(&self) -> usize {
        self.pending
    }

    fn set_document_html(&mut self, html: &str, url: &str) -> Result<(), RuntimeError> {
        self.html_calls.push((html.to_string(), url.to_string()));
        Ok(())
    }
}
