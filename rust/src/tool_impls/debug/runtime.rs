//! Tools that answer "what is this page doing right now": console, network, metrics.
//!
//! These are the first thing to reach for when something did not work, because the useful
//! answer is usually already recorded — a failed request, a thrown exception — and invisible
//! in a screenshot.

use async_trait::async_trait;
use serde_json::{Map, Value};

use crate::ops;
use crate::tools::{ParamSpec, ParamType, Tool, ToolCtx, ToolError, ToolOutput, ToolSpec};

use super::super::{arg_i64, arg_str};

// --- console_logs --------------------------------------------------------------

pub struct ConsoleLogsTool;

#[async_trait]
impl Tool for ConsoleLogsTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "console_logs",
            description: "Get captured browser console entries (log/info/warning/error/exception).",
            params: vec![
                ParamSpec::new(
                    "level",
                    ParamType::String,
                    "Filter by level: log, info, warning, error (default: all)",
                ),
                ParamSpec::new(
                    "limit",
                    ParamType::Integer,
                    "Max entries to return (default 50)",
                ),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let level = arg_str(args, "level");
        let limit = arg_i64(args, "limit", 50).max(0) as usize;
        Ok(ToolOutput::text(
            ctx.browser.console_logs(level, limit).await?,
        ))
    }
}

// --- network_log ---------------------------------------------------------------

pub struct NetworkLogTool;

#[async_trait]
impl Tool for NetworkLogTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "network_log",
            description: "Get captured network requests with status, duration, and size.",
            params: vec![
                ParamSpec::new(
                    "url_pattern",
                    ParamType::String,
                    "Filter by URL substring (default: all)",
                ),
                ParamSpec::new("limit", ParamType::Integer, "Max entries (default 50)"),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let pattern = arg_str(args, "url_pattern");
        let limit = arg_i64(args, "limit", 50).max(0) as usize;
        Ok(ToolOutput::text(
            ctx.browser.network_log(pattern, limit).await?,
        ))
    }
}

// --- metrics -------------------------------------------------------------------

pub struct MetricsTool;

#[async_trait]
impl Tool for MetricsTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "metrics",
            description: "Get Chrome performance metrics: JSHeapUsedSize, Nodes, Documents, etc.",
            params: vec![ParamSpec::new(
                "key",
                ParamType::String,
                "Return only this metric (default: all)",
            )],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let key = arg_str(args, "key");
        Ok(ToolOutput::text(ctx.browser.metrics(key).await?))
    }
}

// --- debug ---------------------------------------------------------------------

pub struct DebugTool;

#[async_trait]
impl Tool for DebugTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "debug",
            description: "Control an in-page console interceptor: start (install), flush (default, drain captured logs), stop (remove).",
            params: vec![ParamSpec::new("action", ParamType::String, "start, flush (default), or stop").with_enum(&["start", "flush", "stop"])],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let action = arg_str(args, "action").unwrap_or("flush");
        let tab = ctx.browser.tab().await?;
        Ok(ToolOutput::text(ops::debug(&tab, action).await?))
    }
}
