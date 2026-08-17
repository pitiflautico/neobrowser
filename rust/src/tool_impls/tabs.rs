//! Multi-tab management over one shared Chrome.
//!
//! Split out of a single 2700-line `tool_impls.rs`: at 67 tools that file had
//! stopped being navigable, and a reviewer could not tell which tools a change
//! touched.

use async_trait::async_trait;
use serde_json::{Map, Value};

use crate::tools::{ParamSpec, ParamType, Tool, ToolCtx, ToolError, ToolOutput, ToolSpec};

// --- multi-tab management ------------------------------------------------------

pub struct NewTabTool;

#[async_trait]
impl Tool for NewTabTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec { name: "new_tab", description: "Open a new browser tab on the shared Chrome and make it active. Returns its index/id.", params: vec![] }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        _args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput::text(ctx.browser.new_tab().await?))
    }
}

pub struct ListTabsTool;

#[async_trait]
impl Tool for ListTabsTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "list_tabs",
            description: "List open tabs with index, url, title, and which is active.",
            params: vec![],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        _args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput::text(ctx.browser.list_tabs().await?))
    }
}

pub struct SwitchTabTool;

#[async_trait]
impl Tool for SwitchTabTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "switch_tab",
            description: "Make the tab at the given index active (subsequent tools act on it).",
            params: vec![
                ParamSpec::new("index", ParamType::Integer, "Tab index from list_tabs").required(),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let index = args
            .get("index")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| ToolError::Argument("switch_tab: index must be an integer".into()))?;
        if index < 0 {
            return Err(ToolError::Argument("switch_tab: index must be >= 0".into()));
        }
        Ok(ToolOutput::text(
            ctx.browser.switch_tab(index as usize).await?,
        ))
    }
}

pub struct CloseTabTool;

#[async_trait]
impl Tool for CloseTabTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "close_tab",
            description: "Close the tab at the given index (cannot close the last remaining tab).",
            params: vec![
                ParamSpec::new("index", ParamType::Integer, "Tab index from list_tabs").required(),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let index = args
            .get("index")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| ToolError::Argument("close_tab: index must be an integer".into()))?;
        if index < 0 {
            return Err(ToolError::Argument("close_tab: index must be >= 0".into()));
        }
        Ok(ToolOutput::text(
            ctx.browser.close_tab(index as usize).await?,
        ))
    }
}
