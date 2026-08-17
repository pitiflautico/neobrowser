//! Tools for mapping runtime locations back to source, and for exporting a trace bundle.

use async_trait::async_trait;
use serde_json::{Map, Value};

use crate::tools::{ParamSpec, ParamType, Tool, ToolCtx, ToolError, ToolOutput, ToolSpec};

use super::super::{arg_i64, arg_str};

pub struct SourceMapTool;

#[async_trait]
impl Tool for SourceMapTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "source_map",
            description: "Trace a minified stack frame to its original file, line and column by decoding the script's source map. Pass the line/column from the stack trace (1-based, as stacks report them). Reports null with an explanation when no mapping covers that position, rather than guessing a nearby line.",
            params: vec![
                ParamSpec::new("script_url", ParamType::String, "URL of the minified script").required(),
                ParamSpec::new("line", ParamType::Integer, "Line in the minified file"),
                ParamSpec::new("column", ParamType::Integer, "Column in the minified file"),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let url = arg_str(args, "script_url")
            .ok_or_else(|| ToolError::Argument("source_map: script_url must be a string".into()))?;
        let tab = ctx.browser.tab().await?;
        Ok(ToolOutput::text(
            crate::devtools::resolve_source(
                &tab,
                url,
                arg_i64(args, "line", 0).max(0) as u32,
                arg_i64(args, "column", 0).max(0) as u32,
            )
            .await?,
        ))
    }
}

pub struct TraceBundleTool;

#[async_trait]
impl Tool for TraceBundleTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "trace_bundle",
            description: "The evidence bundle for this session: the ordered timeline of tool calls, policy decisions and walls, with secrets redacted. Shareable as-is; also written to disk on exit for `neobrowser trace open <id>`.",
            params: vec![],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        _args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput::text(ctx.trace.bundle().to_string()))
    }
}
