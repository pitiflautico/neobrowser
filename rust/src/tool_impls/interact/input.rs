//! Tools that act on an element directly: keys, hover, click variants, controls, drag.
//!
//! Each reports what happened rather than that an event was sent. `set_control` exists
//! separately from typing because no sequence of keystrokes sets a `<select>` reliably, and
//! setting `.value` without dispatching the events a framework listens for leaves the DOM
//! right and the application's state stale.

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

use super::super::{arg_f64, arg_str, resolve_target, target_params, verified};

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
