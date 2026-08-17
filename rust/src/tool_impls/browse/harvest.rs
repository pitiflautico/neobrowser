//! Tools for getting structured data out of a page, and for advancing to the next one.

//! The core loop: navigate, observe, act, verify, extract.
//!
//! Split out of a single 2700-line `tool_impls.rs`: at 67 tools that file had
//! stopped being navigable, and a reviewer could not tell which tools a change
//! touched.

use async_trait::async_trait;
use serde_json::{Map, Value};

use crate::ops;
use crate::tools::{ParamSpec, ParamType, Tool, ToolCtx, ToolError, ToolOutput, ToolSpec};

use super::super::{arg_i64, arg_str};

// --- status --------------------------------------------------------------------

pub struct PaginateTool;

#[async_trait]
impl Tool for PaginateTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "paginate",
            description: "Advance to the next page: click a given selector or auto-detect a next/more control, then force a frame.",
            params: vec![ParamSpec::new("selector", ParamType::String, "Optional CSS selector of the next control")],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let selector = arg_str(args, "selector");
        let tab = ctx.browser.tab().await?;
        Ok(ToolOutput::text(ops::paginate(&tab, selector).await?))
    }
}

// --- console_logs --------------------------------------------------------------

// --- extract -------------------------------------------------------------------

pub struct ExtractTool;

#[async_trait]
impl Tool for ExtractTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "extract",
            description: "Extract structured data: 'links' (default) returns anchors as text+href; 'tables' returns table outerHTML.",
            params: vec![ParamSpec::new("what", ParamType::String, "What to extract: links (default) or tables").with_enum(&["links", "tables"])],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let what = arg_str(args, "what").unwrap_or("links");
        let tab = ctx.browser.tab().await?;
        Ok(ToolOutput::text(ops::extract(&tab, what).await?))
    }
}

// --- extract_table -------------------------------------------------------------

// --- extract_table -------------------------------------------------------------

pub struct ExtractTableTool;

#[async_trait]
impl Tool for ExtractTableTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "extract_table",
            description: "Parse a table into an array of {header: cell} row objects.",
            params: vec![
                ParamSpec::new(
                    "selector",
                    ParamType::String,
                    "CSS selector for the table(s) (default: table)",
                ),
                ParamSpec::new(
                    "index",
                    ParamType::Integer,
                    "Which matched table to parse (default 0)",
                ),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let selector = arg_str(args, "selector").unwrap_or("table");
        let index = arg_i64(args, "index", 0);
        let tab = ctx.browser.tab().await?;
        Ok(ToolOutput::text(
            ops::extract_table(&tab, selector, index).await?,
        ))
    }
}

// --- scroll --------------------------------------------------------------------
