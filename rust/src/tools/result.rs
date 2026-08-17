//! What a tool returns, and how it fails.
//!
//! `ToolError` distinguishes kinds of failure that a caller must treat differently — a policy
//! refusal is not a transport error, and neither is a page that simply did not have the
//! element. Flattening them into one string means an agent cannot tell "retry" from "never".

//! Tool registry, schemas, argument validation, and the `Tool` trait.
//!
//! Mirrors the Python `TOOLS` dict + `_validate_args` + `dispatch_tool`, but with
//! typed specs and a trait-object registry so each tool is a self-contained unit
//! that the MCP layer (see `mcp.rs`) can list and call generically.

use thiserror::Error;

/// What a tool produces on success.
#[derive(Debug, Clone)]
pub enum ToolOutput {
    Text(String),
    Image { data: String, mime: String },
}

impl ToolOutput {
    pub fn text(s: impl Into<String>) -> Self {
        ToolOutput::Text(s.into())
    }
}

/// Tool failure. `Argument` is a caller error (bad params); `Failed` is a runtime
/// fault. Both surface to the model as MCP `isError` text, never a crash.
#[derive(Debug, Error)]
pub enum ToolError {
    #[error("{0}")]
    Argument(String),
    #[error("{0}")]
    Failed(String),
}

impl From<crate::cdp::CdpError> for ToolError {
    fn from(e: crate::cdp::CdpError) -> Self {
        ToolError::Failed(e.to_string())
    }
}

impl From<crate::chrome::ChromeError> for ToolError {
    fn from(e: crate::chrome::ChromeError) -> Self {
        ToolError::Failed(e.to_string())
    }
}
