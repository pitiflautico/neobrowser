//! Tools for finding an element, and for describing the page as a model can act on.
//!
//! `observe` returns stable references (`role:name#nth`) rather than node ids, because a
//! node id is invalidated by any re-render between observing and acting — and a stale id
//! does not fail, it addresses something else.

//! The core loop: navigate, observe, act, verify, extract.
//!
//! Split out of a single 2700-line `tool_impls.rs`: at 67 tools that file had
//! stopped being navigable, and a reviewer could not tell which tools a change
//! touched.

use async_trait::async_trait;
use serde_json::{json, Map, Value};

use crate::page;
use crate::tools::{ParamSpec, ParamType, Tool, ToolCtx, ToolError, ToolOutput, ToolSpec};

use super::super::{arg_bool, arg_i64, arg_str};

// --- status --------------------------------------------------------------------

pub struct FindTool;

#[async_trait]
impl Tool for FindTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "find",
            description: "Find a UI element by natural-language intent (accessibility tree + heuristics). Returns a backendNodeId for use with click.",
            params: vec![ParamSpec::new(
                "intent",
                ParamType::String,
                "What to find, e.g. 'send button', 'message input box'",
            )
            .required()],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let intent = arg_str(args, "intent")
            .ok_or_else(|| ToolError::Argument("find: intent must be a string".into()))?;
        let tab = ctx.browser.tab().await?;
        match page::find(&tab, intent).await? {
            Some(n) => Ok(ToolOutput::text(
                json!({
                    "found": true,
                    "backend_node_id": n.backend_node_id,
                    "role": n.role,
                    "name": n.name,
                })
                .to_string(),
            )),
            None => Ok(ToolOutput::text(
                json!({ "found": false, "backend_node_id": null }).to_string(),
            )),
        }
    }
}

// --- revoke_session ------------------------------------------------------------

// --- observe -------------------------------------------------------------------

pub struct ObserveTool;

#[async_trait]
impl Tool for ObserveTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "observe",
            description: "Accessibility snapshot with STABLE element references you can pass to click/type. Prefer this over `find` for multi-step work: a reference like `button:Continue#0` survives a re-render, while a backendNodeId does not. Pass diff=true to get only what changed since the last observe, which is usually a few lines instead of the whole tree.",
            params: vec![
                ParamSpec::new("mode", ParamType::String, "interactive (default, actionable elements only) | visible (adds static text) | full (everything, for debugging)"),
                ParamSpec::new("diff", ParamType::Boolean, "Return only added/removed/changed elements since the previous observe (default false)"),
                ParamSpec::new("budget_chars", ParamType::Integer, "Maximum characters of listing to return (default 4000). Truncation is reported, never silent"),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        use crate::observe::{self, SnapshotMode};

        let mode = arg_str(args, "mode")
            .and_then(SnapshotMode::parse)
            .unwrap_or(SnapshotMode::Interactive);
        let want_diff = arg_bool(args, "diff", false);
        let budget_chars = arg_i64(args, "budget_chars", 4000).clamp(200, 200_000) as usize;

        let tab = ctx.browser.tab().await?;
        // Force a frame first: in headless the compositor is idle, so deferred
        // content would be missing from the tree we are about to read.
        page::nudge_frame(&tab).await;
        let snap = observe::snapshot(&tab, mode).await?;

        let previous = ctx.browser.take_snapshot().await;
        let mut out = snap.to_json(budget_chars);
        if want_diff {
            match &previous {
                Some(prev) => {
                    let d = observe::diff(prev, &snap);
                    out = json!({
                        "mode": mode.as_str(),
                        "url": snap.url,
                        "elements": snap.nodes.len(),
                        "diff": d.to_json(),
                    });
                }
                None => {
                    // Nothing to diff against yet. Returning the full snapshot with a
                    // note is more useful than an empty diff that reads as "no
                    // changes" when the truth is "no baseline".
                    out["warnings"] = json!([
                        "no_previous_snapshot: returned a full snapshot because there was \
                         nothing to diff against; the next observe(diff=true) will diff"
                    ]);
                }
            }
        }
        ctx.browser.store_snapshot(snap).await;
        Ok(ToolOutput::text(out.to_string()))
    }
}

// --- click ---------------------------------------------------------------------

// --- click ---------------------------------------------------------------------
