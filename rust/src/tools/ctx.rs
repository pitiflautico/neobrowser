//! The context every tool receives, and the trait they implement.
//!
//! The context carries the browser, the trace and the policy decision, so a tool never
//! reaches for global state — which is what makes two HTTP sessions genuinely independent
//! rather than accidentally sharing a browser.

//! Tool registry, schemas, argument validation, and the `Tool` trait.
//!
//! Mirrors the Python `TOOLS` dict + `_validate_args` + `dispatch_tool`, but with
//! typed specs and a trait-object registry so each tool is a self-contained unit
//! that the MCP layer (see `mcp.rs`) can list and call generically.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Map, Value};

use super::catalogue::Registry;
use super::result::{ToolError, ToolOutput};
use super::spec::ToolSpec;
use crate::browser::Browser;

/// Shared context handed to every tool call.
#[derive(Clone)]
pub struct ToolCtx {
    pub browser: Arc<Browser>,
    /// The tool registry, so meta-tools (replay) can re-invoke other tools.
    pub registry: Arc<Registry>,
    /// Resolved once at startup: the policy evaluated before every dispatch. Held
    /// here rather than read from the environment per call so a session cannot have
    /// its rules changed underneath it mid-run.
    pub policy: Arc<crate::policy::Policy>,
    /// This session's trace. Shared so tools can add evidence to the same timeline
    /// the dispatch layer is already writing to.
    pub trace: Arc<crate::trace::Trace>,
    /// The Chrome bridge, when enabled. `None` is the ordinary case, so the bridge
    /// tools can report "not enabled" with instructions rather than failing opaquely.
    pub bridge: Option<Arc<crate::bridge::Bridge>>,
}

/// A callable tool.
#[async_trait]
pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;
    async fn call(&self, ctx: &ToolCtx, args: &Map<String, Value>)
        -> Result<ToolOutput, ToolError>;
}
