//! neo-engine — the AI browser engine orchestrator.
//!
//! Single entry point for agents: navigate, interact, extract, trace.
//! Wires together HTTP, DOM, JS runtime, interaction, extraction, and tracing
//! into a coherent navigation lifecycle.

pub mod config;
pub mod lifecycle;
pub mod mock;
pub mod session;

pub use config::EngineConfig;
pub use lifecycle::Lifecycle;
pub use mock::MockBrowserEngine;
pub use session::NeoSession;

use neo_extract::WomDocument;
use neo_interact::{ClickResult, SubmitResult};
use neo_trace::ExecutionSummary;
use neo_types::{PageState, TraceEntry};
use std::collections::HashMap;

/// Errors from the engine layer.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// HTTP layer failure.
    #[error("http: {0}")]
    Http(#[from] neo_http::HttpError),

    /// DOM parsing or query failure.
    #[error("dom: {0}")]
    Dom(#[from] neo_dom::DomError),

    /// JavaScript runtime failure.
    #[error("runtime: {0}")]
    Runtime(#[from] neo_runtime::RuntimeError),

    /// Interaction failure (element not found, not interactive, etc.).
    #[error("interact: {0}")]
    Interact(#[from] neo_interact::InteractError),

    /// Invalid URL provided.
    #[error("invalid url: {0}")]
    InvalidUrl(String),

    /// Navigation timed out.
    #[error("navigation timeout after {0}ms")]
    Timeout(u64),

    /// Engine is in an invalid state for the requested operation.
    #[error("invalid state: expected {expected:?}, got {actual:?}")]
    InvalidState {
        expected: PageState,
        actual: PageState,
    },
}

/// The AI browser engine -- the single entry point for agents.
pub trait BrowserEngine {
    /// Navigate to URL. Returns structured page data.
    fn navigate(&mut self, url: &str) -> Result<PageResult, EngineError>;

    /// Navigate back in history.
    fn back(&mut self) -> Result<PageResult, EngineError>;

    /// Navigate forward in history.
    fn forward(&mut self) -> Result<PageResult, EngineError>;

    /// Get the navigation history as a list of URLs.
    fn history(&self) -> Vec<String>;

    /// Current page state in the lifecycle.
    fn page_state(&self) -> PageState;

    /// Execute JavaScript and return result.
    fn eval(&mut self, js: &str) -> Result<String, EngineError>;

    /// Click an element.
    fn click(&mut self, target: &str) -> Result<ClickResult, EngineError>;

    /// Type text into an element.
    fn type_text(&mut self, target: &str, text: &str) -> Result<(), EngineError>;

    /// Fill a form with multiple fields.
    fn fill_form(&mut self, fields: &HashMap<String, String>) -> Result<(), EngineError>;

    /// Submit the current form.
    fn submit(&mut self, target: Option<&str>) -> Result<SubmitResult, EngineError>;

    /// Extract WOM (what the AI sees).
    fn extract(&self) -> Result<WomDocument, EngineError>;

    /// Get execution trace.
    fn trace(&self) -> Vec<TraceEntry>;

    /// Get execution summary.
    fn summary(&self) -> ExecutionSummary;
}

/// Result of a page navigation (engine-level, wraps neo-types::PageResult).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PageResult {
    /// Final URL after redirects.
    pub url: String,
    /// Document title.
    pub title: String,
    /// Final lifecycle state.
    pub state: PageState,
    /// Total render time in milliseconds.
    pub render_ms: u64,
    /// Extracted WOM document.
    pub wom: WomDocument,
    /// Errors encountered during navigation.
    pub errors: Vec<String>,
    /// URLs visited during redirect chain (empty if no redirects).
    #[serde(default)]
    pub redirect_chain: Vec<String>,
}
