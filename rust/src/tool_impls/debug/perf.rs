//! Tools that answer "why is this slow, and what is it holding": traces, styles, HAR, heap.
//!
//! HAR import and export are here together on purpose: a captured network log is only useful
//! if it can leave, and only trustworthy if it can come back and be compared.

use async_trait::async_trait;
use serde_json::{Map, Value};

use crate::page;
use crate::reach;
use crate::tools::{ParamSpec, ParamType, Tool, ToolCtx, ToolError, ToolOutput, ToolSpec};

use super::super::{arg_i64, arg_str};

pub struct PerfTraceTool;

#[async_trait]
impl Tool for PerfTraceTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "perf_trace",
            description: "Web Vitals (LCP, CLS, TTFB), navigation timing, the slowest resources, and JS heap use — with an `insights` list naming which numbers exceed Google's published thresholds rather than leaving you to read a timing table.",
            params: vec![],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        _args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let tab = ctx.browser.tab().await?;
        Ok(ToolOutput::text(crate::devtools::perf_trace(&tab).await?))
    }
}

pub struct ComputedStyleTool;

#[async_trait]
impl Tool for ComputedStyleTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "computed_style",
            description: "Resolved CSS for one element, its box, and — when it is not visible — an explicit `hidden_because` list (display:none / visibility:hidden / opacity:0 / zero-sized). Answers 'why can't I click this'.",
            params: vec![
                ParamSpec::new("selector", ParamType::String, "CSS selector").required(),
                ParamSpec::new("properties", ParamType::Array, "Specific CSS properties; omit for a useful default set"),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let selector = arg_str(args, "selector").ok_or_else(|| {
            ToolError::Argument("computed_style: selector must be a string".into())
        })?;
        let properties: Vec<String> = args
            .get("properties")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default();
        let tab = ctx.browser.tab().await?;
        Ok(ToolOutput::text(
            crate::devtools::computed_style(&tab, selector, &properties).await?,
        ))
    }
}

pub struct HarExportTool;

#[async_trait]
impl Tool for HarExportTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "har_export",
            description: "Export captured network activity as a HAR 1.2 document, readable by DevTools and any HTTP analysis tool. URLs are redacted, since a HAR is routinely attached to a bug report.",
            params: vec![
                ParamSpec::new("limit", ParamType::Integer, "Maximum requests to include (default 200)"),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let limit = arg_i64(args, "limit", 200).clamp(1, 5000) as usize;
        let tab = ctx.browser.tab().await?;
        let entries = ctx.browser.network_entries(None, limit).await;
        let url = page::current_url(&tab).await.unwrap_or_default();
        Ok(ToolOutput::text(
            crate::devtools::to_har(&entries, &url).to_string(),
        ))
    }
}

pub struct HarImportTool;

#[async_trait]
impl Tool for HarImportTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "har_import",
            description: "Read a HAR document (from DevTools, a colleague, a bug report) and summarise its failures and slowest requests. Summarised rather than echoed, since a HAR is megabytes; URLs are redacted, since someone else's HAR is full of their tokens.",
            params: vec![
                ParamSpec::new("path", ParamType::String, "Path to a .har file").required(),
            ],
        }
    }
    async fn call(
        &self,
        _ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let path = arg_str(args, "path")
            .ok_or_else(|| ToolError::Argument("har_import: path must be a string".into()))?;
        // Routed through the same allowlist as `upload`: reading an arbitrary path
        // because the argument is called `path` instead of `files` would be a hole in
        // exactly the control that exists to stop it.
        let resolved = reach::resolve_upload_path(path).map_err(ToolError::Failed)?;
        let text = std::fs::read_to_string(&resolved)
            .map_err(|e| ToolError::Failed(format!("har_import: {e}")))?;
        Ok(ToolOutput::text(
            crate::devtools::from_har(&text).to_string(),
        ))
    }
}

// --- B3 frames/shadow/dialogs/emulation + D2 bridge ----------------------------

pub struct CpuProfileTool;

#[async_trait]
impl Tool for CpuProfileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "cpu_profile",
            description: "Sample the JS main thread and report the functions that burned the time (self-time, ranked). Interact with the page while this runs. Returns the top 20 rather than the raw sample tree, which is tens of thousands of nodes.",
            params: vec![ParamSpec::new("duration_ms", ParamType::Integer, "Sampling window, 100–30000 (default 3000)")],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let tab = ctx.browser.tab().await?;
        let ms = arg_i64(args, "duration_ms", 3000).clamp(100, 30_000) as u64;
        Ok(ToolOutput::text(
            crate::devtools::cpu_profile(&tab, ms).await?,
        ))
    }
}

pub struct HeapStatsTool;

#[async_trait]
impl Tool for HeapStatsTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "heap_stats",
            description: "JS heap size plus DOM node, listener, document and frame counts. Call it twice around a repeated interaction: counts that grow every cycle and never come back down is the signature of a leak.",
            params: vec![],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        _args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let tab = ctx.browser.tab().await?;
        Ok(ToolOutput::text(crate::devtools::heap_stats(&tab).await?))
    }
}
