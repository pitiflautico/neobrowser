//! neo-chrome — Chrome CDP fallback for NeoRender.
//!
//! Launches Chrome, connects via WebSocket CDP, provides navigate/eval/close.
//! Used when the V8 engine can't handle a site (WAF, captcha, Turnstile).
//! This is the fallback, not the primary engine.

pub mod cdp;
pub mod emulation;
pub mod fetch_proxy;
pub mod input;
pub mod launcher;
pub mod mock;
pub mod mouse;
pub mod observation;
pub mod navigation;
pub mod performance;
pub mod session;
pub mod sync_session;

use neo_types::PageResult;

/// Errors from the Chrome CDP layer.
#[derive(Debug, thiserror::Error)]
pub enum ChromeError {
    /// Chrome binary not found on the system.
    #[error("Chrome not found")]
    NotFound,
    /// Failed to establish CDP WebSocket connection.
    #[error("CDP connection failed: {0}")]
    ConnectionFailed(String),
    /// A CDP command returned an error.
    #[error("CDP command failed: {method} — {error}")]
    CommandFailed {
        /// The CDP method that failed.
        method: String,
        /// The error message from Chrome.
        error: String,
    },
    /// Operation timed out waiting for Chrome.
    #[error("timeout: {0}")]
    Timeout(String),
    /// Chrome process exited unexpectedly.
    #[error("Chrome process died")]
    ProcessDied,
    /// I/O error during process or file operations.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// JSON serialization/deserialization error.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

/// Result alias for ChromeError.
pub type Result<T> = std::result::Result<T, ChromeError>;

/// Trait for Chrome session implementations (real and mock).
pub trait ChromeSessionTrait: Send + Sync {
    /// Navigate to a URL and return page analysis.
    fn navigate(
        &mut self,
        url: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<PageResult>> + Send + '_>>;

    /// Evaluate JavaScript and return the string result.
    fn eval(
        &self,
        js: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send + '_>>;
}
