//! Interaction coverage beyond a plain click: keys, hover, drag, native
//! controls, shadow DOM and iframes, blocking dialogs, device emulation.
//!
//! Split out of a single 2700-line `tool_impls.rs`: at 67 tools that file had
//! stopped being navigable, and a reviewer could not tell which tools a change
//! touched.

use async_trait::async_trait;
use serde_json::{Map, Value};

use crate::page;
use crate::tools::{ParamSpec, ParamType, Tool, ToolCtx, ToolError, ToolOutput, ToolSpec};

use super::{arg_bool, arg_f64, arg_str, resolve_target, target_params, verified};

pub struct PressTool;

#[async_trait]
impl Tool for PressTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "press",
            description: "Press a key or shortcut on the focused element: Enter, Tab, Escape, Backspace, Delete, Arrow*, Home, End, PageUp, PageDown, Space, or a single character. Combine with modifiers for shortcuts (e.g. key='a', modifiers=['ctrl']).",
            params: vec![
                ParamSpec::new("key", ParamType::String, "Key name or single character").required(),
                ParamSpec::new("modifiers", ParamType::Array, "Any of alt, ctrl, meta, shift"),
                ParamSpec::new("budget_s", ParamType::Number, "Seconds to wait for the page to react (default 3)"),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let key = arg_str(args, "key")
            .ok_or_else(|| ToolError::Argument("press: key must be a string".into()))?;
        let modifiers: Vec<String> = args
            .get("modifiers")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default();
        let tab = ctx.browser.tab().await?;
        let tab_ref = &tab;
        verified(
            &tab,
            "press",
            arg_f64(args, "budget_s", 3.0),
            || async move { page::press_key(tab_ref, key, &modifiers).await },
        )
        .await
    }
}

pub struct HoverTool;

#[async_trait]
impl Tool for HoverTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "hover",
            description: "Move the real cursor over an element without clicking. Needed for menus and tooltips that only appear on a trusted mouseover — a JS-dispatched event is not isTrusted and many libraries check.",
            params: {
                // Declared because the implementation reads it; a param the code
                // honours but the schema omits is rejected by validate_args.
                let mut p = target_params();
                p.push(ParamSpec::new("budget_s", ParamType::Number, "Seconds to wait for the page to react (default 2)"));
                p
            },
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let tab = ctx.browser.tab().await?;
        let id = resolve_target(&tab, args, "hover").await?;
        let tab_ref = &tab;
        verified(
            &tab,
            "hover",
            arg_f64(args, "budget_s", 2.0),
            || async move { page::hover(tab_ref, id).await },
        )
        .await
    }
}

pub struct ClickVariantTool;

#[async_trait]
impl Tool for ClickVariantTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "click_variant",
            description: "Double-click or right-click an element with real mouse events, using the same scroll and overlay checks as `click`.",
            params: {
                let mut p = target_params();
                p.push(ParamSpec::new("kind", ParamType::String, "double | right (default double)")
                    .with_enum(&["double", "right"]));
                p.push(ParamSpec::new("budget_s", ParamType::Number, "Seconds to wait for the page to react (default 4)"));
                p
            },
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let kind = arg_str(args, "kind").unwrap_or("double");
        let (button, count) = match kind {
            "right" => ("right", 1),
            "double" => ("left", 2),
            other => {
                return Err(ToolError::Argument(format!(
                    "click_variant: kind must be double or right, got {other:?}"
                )))
            }
        };
        let tab = ctx.browser.tab().await?;
        let id = resolve_target(&tab, args, "click_variant").await?;
        let tab_ref = &tab;
        verified(
            &tab,
            "click_variant",
            arg_f64(args, "budget_s", 4.0),
            || async move { page::click_variant(tab_ref, id, button, count).await },
        )
        .await
    }
}

pub struct SetControlTool;

#[async_trait]
impl Tool for SetControlTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "set_control",
            description: "Set a checkbox, radio, <select> or contenteditable. Goes through the framework-visible property setter, so React/Vue state updates too — assigning `.checked` directly changes the pixels while the app keeps the old value.",
            params: vec![
                ParamSpec::new("selector", ParamType::String, "CSS selector of the control").required(),
                ParamSpec::new("value", ParamType::String, "For select: option value or visible text. For checkbox/radio: true|false. For contenteditable: the text").required(),
                ParamSpec::new("budget_s", ParamType::Number, "Seconds to wait for the page to react (default 3)"),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let selector = arg_str(args, "selector")
            .ok_or_else(|| ToolError::Argument("set_control: selector must be a string".into()))?;
        let value = arg_str(args, "value")
            .ok_or_else(|| ToolError::Argument("set_control: value must be a string".into()))?;
        let tab = ctx.browser.tab().await?;
        let tab_ref = &tab;
        verified(
            &tab,
            "set_control",
            arg_f64(args, "budget_s", 3.0),
            || async move { page::set_control(tab_ref, selector, value).await },
        )
        .await
    }
}

pub struct DragTool;

#[async_trait]
impl Tool for DragTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "drag",
            description: "Drag one element onto another with real mouse events, including the intermediate moves HTML5 drag-and-drop and JS drag libraries require (a press-then-release does nothing).",
            params: vec![
                ParamSpec::new("from_selector", ParamType::String, "CSS selector of the element to drag").required(),
                ParamSpec::new("to_selector", ParamType::String, "CSS selector of the drop target").required(),
                ParamSpec::new("budget_s", ParamType::Number, "Seconds to wait for the page to react (default 4)"),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let from = arg_str(args, "from_selector")
            .ok_or_else(|| ToolError::Argument("drag: from_selector must be a string".into()))?;
        let to = arg_str(args, "to_selector")
            .ok_or_else(|| ToolError::Argument("drag: to_selector must be a string".into()))?;
        let tab = ctx.browser.tab().await?;
        let from_id = page::backend_node_for_css(&tab, from)
            .await?
            .ok_or_else(|| ToolError::Failed(format!("drag: no element matches {from:?}")))?;
        let to_id = page::backend_node_for_css(&tab, to)
            .await?
            .ok_or_else(|| ToolError::Failed(format!("drag: no element matches {to:?}")))?;
        let tab_ref = &tab;
        verified(
            &tab,
            "drag",
            arg_f64(args, "budget_s", 4.0),
            || async move { page::drag_and_drop(tab_ref, from_id, to_id).await },
        )
        .await
    }
}

// --- B3 frames/shadow/dialogs/emulation + D2 bridge ----------------------------

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
