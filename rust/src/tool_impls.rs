//! Concrete tool implementations and the registry builder.
//!
//! Phase 2 shipped `status`. Phase 3 adds the core browser verbs:
//! navigate, read, screenshot, find, click, type. Phases 5–6 add the rest.

use serde_json::{Map, Value};
use std::sync::Arc;

use crate::page;
use crate::tools::{ParamSpec, ParamType, Registry, Tool, ToolError, ToolOutput};

// --- small typed arg accessors -------------------------------------------------

fn arg_str<'a>(args: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| v.as_str())
}
/// Wrap a raw mutating operation in the verified-action envelope.
///
/// The raw `ops::*` functions report what they attempted; this reports what the page
/// did about it. Every mutating tool goes through here so the observe → act → verify
/// discipline cannot be forgotten in one place and present in another.
async fn verified<F, Fut>(
    tab: &crate::cdp::CdpClient,
    action: &str,
    budget_s: f64,
    op: F,
) -> Result<ToolOutput, ToolError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<String, crate::cdp::CdpError>>,
{
    let budget = crate::action::Budget::from_secs(budget_s);
    let result = crate::action::perform(tab, action, budget, op).await;
    Ok(ToolOutput::text(result.to_string_pretty()))
}

fn arg_f64(args: &Map<String, Value>, key: &str, default: f64) -> f64 {
    args.get(key).and_then(|v| v.as_f64()).unwrap_or(default)
}
fn arg_i64(args: &Map<String, Value>, key: &str, default: i64) -> i64 {
    args.get(key).and_then(|v| v.as_i64()).unwrap_or(default)
}
fn arg_bool(args: &Map<String, Value>, key: &str, default: bool) -> bool {
    args.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
}

pub mod browse;
pub mod debug;
pub mod fetch;
pub mod interact;
pub mod session;
pub mod tabs;
pub mod websearch;
/// Resolve a target given as `ref`, `backend_node_id` or `selector`, in that order of
/// preference. Shared by the interaction tools so they all accept the same addressing.
async fn resolve_target(
    tab: &crate::cdp::CdpClient,
    args: &Map<String, Value>,
    tool: &str,
) -> Result<i64, ToolError> {
    if let Some(r) = arg_str(args, "ref") {
        return crate::observe::resolve(tab, r).await?.ok_or_else(|| {
            ToolError::Failed(format!(
                "{tool}: no element currently matches the reference `{r}`. Re-run `observe`"
            ))
        });
    }
    if let Some(id) = args.get("backend_node_id").and_then(Value::as_i64) {
        return Ok(id);
    }
    if let Some(sel) = arg_str(args, "selector") {
        return page::backend_node_for_css(tab, sel)
            .await?
            .ok_or_else(|| ToolError::Failed(format!("{tool}: no element matches {sel:?}")));
    }
    Err(ToolError::Argument(format!(
        "{tool}: provide ref (preferred), backend_node_id, or selector"
    )))
}

fn target_params() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new(
            "ref",
            ParamType::String,
            "Stable reference from `observe` (preferred)",
        ),
        ParamSpec::new(
            "backend_node_id",
            ParamType::Integer,
            "backendNodeId from `find`",
        ),
        ParamSpec::new("selector", ParamType::String, "CSS selector"),
    ]
}

pub mod bridge;
pub mod playbook;

// Re-exported flat so `tool_impls::NavigateTool` keeps working and the
// registry list below reads as one inventory rather than nine.
pub use bridge::*;
pub use browse::*;
pub use debug::*;
pub use fetch::*;
pub use interact::*;
pub use playbook::*;
pub use session::*;
pub use tabs::*;
pub use websearch::*;

pub fn build_registry() -> Registry {
    let mut r = Registry::new();
    for t in tool_list() {
        r.register(t);
    }
    r
}

/// All registered tools. Kept as a list so tests can assert coverage/parity.
pub fn tool_list() -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(StatusTool),
        Arc::new(ObserveTool),
        Arc::new(RevokeSessionTool),
        Arc::new(PressTool),
        Arc::new(HoverTool),
        Arc::new(ClickVariantTool),
        Arc::new(SetControlTool),
        Arc::new(DragTool),
        Arc::new(PerfTraceTool),
        Arc::new(ComputedStyleTool),
        Arc::new(HarExportTool),
        Arc::new(TraceBundleTool),
        Arc::new(LoginFlowTool),
        Arc::new(ExtractPaginatedTool),
        Arc::new(ProfileModeTool),
        Arc::new(PierceTool),
        Arc::new(ListFramesTool),
        Arc::new(DialogTool),
        Arc::new(EmulateTool),
        Arc::new(BridgeStatusTool),
        Arc::new(BridgeCdpTool),
        Arc::new(CpuProfileTool),
        Arc::new(HeapStatsTool),
        Arc::new(SourceMapTool),
        Arc::new(HarImportTool),
        Arc::new(NavigateTool),
        Arc::new(ReadTool),
        Arc::new(ScreenshotTool),
        Arc::new(FindTool),
        Arc::new(ClickTool),
        Arc::new(TypeTool),
        Arc::new(JsTool),
        Arc::new(PageInfoTool),
        Arc::new(AnalyzeTool),
        Arc::new(FillTool),
        Arc::new(FormFillTool),
        Arc::new(SubmitTool),
        Arc::new(FindAndClickTool),
        Arc::new(DismissOverlayTool),
        Arc::new(ExtractTool),
        Arc::new(ExtractTableTool),
        Arc::new(ScrollTool),
        Arc::new(WaitTool),
        Arc::new(PaginateTool),
        Arc::new(ConsoleLogsTool),
        Arc::new(NetworkLogTool),
        Arc::new(MetricsTool),
        Arc::new(DebugTool),
        Arc::new(SaveCookiesTool),
        Arc::new(RestoreCookiesTool),
        Arc::new(SaveSessionTool),
        Arc::new(SessionInfoTool),
        Arc::new(LoginTool),
        Arc::new(BrowseTool),
        Arc::new(UploadTool),
        Arc::new(DownloadTool),
        Arc::new(SearchTool),
        Arc::new(SearchImagesTool),
        Arc::new(SearchVideosTool),
        Arc::new(SearchTwitterVideosTool),
        Arc::new(RecordTaskTool),
        Arc::new(StopRecordingTool),
        Arc::new(ReplayTool),
        Arc::new(NewTabTool),
        Arc::new(ListTabsTool),
        Arc::new(SwitchTabTool),
        Arc::new(CloseTabTool),
    ]
}

/// Regression guard: the registry must expose the full Python-parity tool set.
#[cfg(test)]
mod tests {
    use super::*;

    const EXPECTED: &[&str] = &[
        // Verified-observation + vault tools (Rust-only)
        "observe",
        "revoke_session",
        "press",
        "hover",
        "click_variant",
        "set_control",
        "drag",
        "perf_trace",
        "computed_style",
        "har_export",
        "trace_bundle",
        "login_flow",
        "extract_paginated",
        "profile_mode",
        "pierce",
        "list_frames",
        "dialog",
        "emulate",
        "bridge_status",
        "bridge_cdp",
        "cpu_profile",
        "heap_stats",
        "source_map",
        "har_import",
        // 39 Python-parity tools
        "status",
        "navigate",
        "read",
        "screenshot",
        "find",
        "click",
        "type",
        "js",
        "page_info",
        "analyze",
        "fill",
        "form_fill",
        "submit",
        "find_and_click",
        "dismiss_overlay",
        "extract",
        "extract_table",
        "scroll",
        "wait",
        "paginate",
        "console_logs",
        "network_log",
        "metrics",
        "debug",
        "save_cookies",
        "restore_cookies",
        "save_session",
        "session_info",
        "login",
        "browse",
        "upload",
        "download",
        "search",
        "search_images",
        "search_videos",
        "search_twitter_videos",
        "record_task",
        "stop_recording",
        "replay",
        // Rust additions: real multi-tab support
        "new_tab",
        "list_tabs",
        "switch_tab",
        "close_tab",
    ];

    #[test]
    fn registry_has_full_tool_parity() {
        let reg = build_registry();
        let names: std::collections::HashSet<&str> =
            tool_list().iter().map(|t| t.spec().name).collect();
        for name in EXPECTED {
            assert!(names.contains(name), "missing tool: {name}");
        }
        assert_eq!(names.len(), EXPECTED.len(), "tool count mismatch");
        assert_eq!(reg.len(), EXPECTED.len());
    }

    #[test]
    fn no_duplicate_tool_names() {
        let list = tool_list();
        let unique: std::collections::HashSet<&str> = list.iter().map(|t| t.spec().name).collect();
        assert_eq!(unique.len(), list.len(), "duplicate tool name registered");
    }
}
