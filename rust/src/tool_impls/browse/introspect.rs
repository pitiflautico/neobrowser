//! Tools that ask the page about itself: run script, summarise, analyse.

//! The core loop: navigate, observe, act, verify, extract.
//!
//! Split out of a single 2700-line `tool_impls.rs`: at 67 tools that file had
//! stopped being navigable, and a reviewer could not tell which tools a change
//! touched.

use async_trait::async_trait;
use serde_json::{Map, Value};

use crate::ops;
use crate::tools::{ParamSpec, ParamType, Tool, ToolCtx, ToolError, ToolOutput, ToolSpec};

use super::super::arg_str;

// --- status --------------------------------------------------------------------

pub struct JsTool;

#[async_trait]
impl Tool for JsTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "js",
            description: "Execute JavaScript in the page. Use a return statement to get a value; `await` is supported.",
            params: vec![ParamSpec::new("code", ParamType::String, "JavaScript code to execute. Must use a return statement to return a value.").required()],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let code = arg_str(args, "code")
            .ok_or_else(|| ToolError::Argument("js: code must be a string".into()))?;
        let tab = ctx.browser.tab().await?;
        Ok(ToolOutput::text(ops::eval_js(&tab, code).await?))
    }
}

// --- page_info -----------------------------------------------------------------

// --- page_info -----------------------------------------------------------------

pub struct PageInfoTool;

#[async_trait]
impl Tool for PageInfoTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec { name: "page_info", description: "Summarize the page: url, title, interactive-element count, forms, and overlay presence.", params: vec![] }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        _args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let tab = ctx.browser.tab().await?;
        Ok(ToolOutput::text(ops::page_info(&tab).await?))
    }
}

// --- analyze -------------------------------------------------------------------

// --- analyze -------------------------------------------------------------------

pub struct AnalyzeTool;

#[async_trait]
impl Tool for AnalyzeTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec { name: "analyze", description: "Structured page analysis: forms with fields+labels, buttons, overlays, and the active element.", params: vec![] }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        _args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let tab = ctx.browser.tab().await?;
        Ok(ToolOutput::text(ops::analyze(&tab).await?))
    }
}

// --- fill ----------------------------------------------------------------------

// --- fill ----------------------------------------------------------------------
