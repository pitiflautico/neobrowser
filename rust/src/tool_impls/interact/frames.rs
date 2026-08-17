//! Tools for the page's structure and its environment: frames, dialogs, emulation.
//!
//! Dialogs get a tool because a native `alert()` blocks everything — including this tool's own
//! next command — so there has to be a way to answer one deliberately rather than deadlock.

use async_trait::async_trait;
use serde_json::{Map, Value};

use crate::tools::{ParamSpec, ParamType, Tool, ToolCtx, ToolError, ToolOutput, ToolSpec};

use super::super::{arg_bool, arg_f64, arg_str, verified};

pub struct PierceTool;

#[async_trait]
impl Tool for PierceTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "pierce",
            description: "Reach an element inside shadow DOM or a same-origin iframe, where an ordinary selector returns null. Walks open shadow roots and reachable frames, and reports the path it found the element through. Cross-origin frames are skipped and listed by `list_frames` instead of silently missed.",
            params: vec![
                ParamSpec::new("selector", ParamType::String, "CSS selector to find, at any depth").required(),
                ParamSpec::new("action", ParamType::String, "read | click | fill (default read)")
                    .with_enum(&["read", "click", "fill"]),
                ParamSpec::new("value", ParamType::String, "Value, for action=fill"),
                ParamSpec::new("budget_s", ParamType::Number, "Seconds to wait for the page to react on click/fill (default 3)"),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let selector = arg_str(args, "selector")
            .ok_or_else(|| ToolError::Argument("pierce: selector must be a string".into()))?;
        let action = arg_str(args, "action").unwrap_or("read");
        let value = arg_str(args, "value").unwrap_or("");
        let tab = ctx.browser.tab().await?;
        // A read is not a mutation, so it does not go through the verified envelope —
        // there is nothing to verify, and wrapping it would only add noise.
        if action == "read" {
            return Ok(ToolOutput::text(
                crate::frames::pierce(&tab, selector, action, value).await?,
            ));
        }
        let tab_ref = &tab;
        verified(
            &tab,
            "pierce",
            arg_f64(args, "budget_s", 3.0),
            || async move { crate::frames::pierce(tab_ref, selector, action, value).await },
        )
        .await
    }
}

pub struct ListFramesTool;

#[async_trait]
impl Tool for ListFramesTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "list_frames",
            description: "List every frame in the page with its URL and whether JS can reach into it. A cross-origin frame is reported as such, so an element you cannot find is explainable instead of a mystery — navigate to that frame's URL directly.",
            params: vec![],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        _args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let tab = ctx.browser.tab().await?;
        Ok(ToolOutput::text(crate::frames::list_frames(&tab).await?))
    }
}

pub struct DialogTool;

#[async_trait]
impl Tool for DialogTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "dialog",
            description: "Answer a blocking alert/confirm/prompt/beforeunload. A JavaScript dialog freezes the renderer, so every other call hangs until it is answered — if the browser seems dead after a click, this is the first thing to try.",
            params: vec![
                ParamSpec::new("action", ParamType::String, "accept | dismiss (default dismiss)")
                    .with_enum(&["accept", "dismiss"]),
                ParamSpec::new("prompt_text", ParamType::String, "Text to enter, for a prompt() dialog"),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        use crate::frames::DialogAction;
        let action = match arg_str(args, "action").unwrap_or("dismiss") {
            "accept" => DialogAction::Accept,
            "dismiss" => DialogAction::Dismiss,
            other => {
                return Err(ToolError::Argument(format!(
                    "dialog: action must be accept or dismiss, got {other:?}"
                )))
            }
        };
        let tab = ctx.browser.tab().await?;
        Ok(ToolOutput::text(
            crate::frames::handle_dialog(&tab, action, arg_str(args, "prompt_text")).await?,
        ))
    }
}

pub struct EmulateTool;

#[async_trait]
impl Tool for EmulateTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "emulate",
            description: "Override geolocation, viewport size, or grant browser permissions — what a location-gated or mobile-only flow needs before it will render at all. Pass latitude+longitude, width+height, and/or permissions (geolocation, camera, microphone, notifications, clipboard).",
            params: vec![
                ParamSpec::new("latitude", ParamType::Number, "Latitude to report"),
                ParamSpec::new("longitude", ParamType::Number, "Longitude to report"),
                ParamSpec::new("width", ParamType::Integer, "Viewport width in CSS pixels"),
                ParamSpec::new("height", ParamType::Integer, "Viewport height in CSS pixels"),
                ParamSpec::new("mobile", ParamType::Boolean, "Emulate a mobile device (sets a 2x scale factor; default false)"),
                ParamSpec::new("permissions", ParamType::Array, "Permissions to grant"),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let permissions: Vec<String> = args
            .get("permissions")
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
            crate::frames::emulate(
                &tab,
                args.get("latitude").and_then(Value::as_f64),
                args.get("longitude").and_then(Value::as_f64),
                args.get("width").and_then(Value::as_i64),
                args.get("height").and_then(Value::as_i64),
                arg_bool(args, "mobile", false),
                &permissions,
            )
            .await?,
        ))
    }
}
