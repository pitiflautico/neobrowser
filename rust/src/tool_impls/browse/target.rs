//! Tools for reaching an element: find-and-click, dismiss what covers it, wait, scroll.
//!
//! These are grouped because the reason a click misses is almost always obstruction rather
//! than a wrong selector.

use async_trait::async_trait;
use serde_json::{Map, Value};

use crate::ops;
use crate::tools::{ParamSpec, ParamType, Tool, ToolCtx, ToolError, ToolOutput, ToolSpec};

use super::super::{arg_bool, arg_f64, arg_i64, arg_str, verified};

pub struct FindAndClickTool;

#[async_trait]
impl Tool for FindAndClickTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "find_and_click",
            description: "Click the nth VISIBLE clickable element whose text or aria-label contains the given text. Hidden and collapsed matches (closed accordion steps, header panels duplicating a body form) are skipped and counted in matched_total vs matched_visible, so a multi-step form can't silently submit the wrong step.",
            params: vec![
                ParamSpec::new("text", ParamType::String, "Visible text or label to search for").required(),
                ParamSpec::new("role", ParamType::String, "Optional ARIA role to narrow the search"),
                ParamSpec::new("nth", ParamType::Integer, "Which match to click (0-based, default 0)"),
                ParamSpec::new("budget_s", ParamType::Number, "Seconds to wait for the page to react before reporting 'uncertain' (default 5)"),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let text = arg_str(args, "text")
            .ok_or_else(|| ToolError::Argument("find_and_click: text must be a string".into()))?;
        let role = arg_str(args, "role").unwrap_or("");
        let nth = arg_i64(args, "nth", 0);
        let tab = ctx.browser.tab().await?;
        verified(
            &tab,
            "find_and_click",
            arg_f64(args, "budget_s", 5.0),
            || ops::find_and_click(&tab, text, role, nth),
        )
        .await
    }
}

// --- dismiss_overlay -----------------------------------------------------------

pub struct DismissOverlayTool;

#[async_trait]
impl Tool for DismissOverlayTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "dismiss_overlay",
            description: "Detect and dismiss cookie/GDPR/newsletter overlays by clicking accept/close inside them. force=true also tries Escape + backdrop.",
            params: vec![ParamSpec::new("force", ParamType::Boolean, "Also try Escape + backdrop click (default false)")],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let force = arg_bool(args, "force", false);
        let tab = ctx.browser.tab().await?;
        Ok(ToolOutput::text(ops::dismiss_overlay(&tab, force).await?))
    }
}

// --- wait ----------------------------------------------------------------------

pub struct WaitTool;

#[async_trait]
impl Tool for WaitTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "wait",
            description:
                "Wait for a fixed time, or (if selector given) poll until it appears, up to ms.",
            params: vec![
                ParamSpec::new(
                    "ms",
                    ParamType::Integer,
                    "Milliseconds to wait / poll timeout (default 1000)",
                ),
                ParamSpec::new(
                    "selector",
                    ParamType::String,
                    "Optional CSS selector to wait for",
                ),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let ms = arg_i64(args, "ms", 1000);
        if ms < 0 {
            return Err(ToolError::Argument("wait: ms must be >= 0".into()));
        }
        let ms = ms.min(ops::MAX_WAIT.as_millis() as i64);
        let selector = arg_str(args, "selector");
        let tab = ctx.browser.tab().await?;
        Ok(ToolOutput::text(ops::wait(&tab, ms, selector).await?))
    }
}

// --- scroll --------------------------------------------------------------------

pub struct ScrollTool;

#[async_trait]
impl Tool for ScrollTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "scroll",
            description: "Scroll the viewport (up/down/top/bottom), then force a frame so load-on-scroll content paints.",
            params: vec![
                ParamSpec::new("direction", ParamType::String, "up, down (default), top, or bottom").with_enum(&["up", "down", "top", "bottom"]),
                ParamSpec::new("amount", ParamType::Integer, "Pixels to scroll for up/down (default 500)"),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let direction = arg_str(args, "direction").unwrap_or("down");
        let amount = arg_i64(args, "amount", 500);
        let tab = ctx.browser.tab().await?;
        Ok(ToolOutput::text(
            ops::scroll(&tab, direction, amount).await?,
        ))
    }
}

// --- wait ----------------------------------------------------------------------
