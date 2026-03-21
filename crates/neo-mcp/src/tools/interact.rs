//! `interact` tool — click, type, fill form, submit on the current page.

use serde_json::Value;
use std::collections::HashMap;

use crate::state::McpState;
use crate::McpError;

use super::ToolDef;

/// Tool definition for `tools/list`.
pub(crate) fn definition() -> ToolDef {
    ToolDef {
        name: "interact",
        description: "Interact with the current page (click, type, fill, submit)",
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["click", "type", "fill_form", "submit", "press_key"],
                    "description": "Interaction type"
                },
                "target": {
                    "type": "string",
                    "description": "Element target (CSS selector, text, aria-label)"
                },
                "text": {
                    "type": "string",
                    "description": "Text to type (for action=type)"
                },
                "fields": {
                    "type": "object",
                    "description": "Field name→value map (for action=fill_form)",
                    "additionalProperties": { "type": "string" }
                },
                "key": {
                    "type": "string",
                    "description": "Key to press (for action=press_key): Enter, Tab, Escape, etc."
                }
            },
            "required": ["action"]
        }),
    }
}

/// Execute the `interact` tool.
pub fn call(args: Value, state: &mut McpState) -> Result<Value, McpError> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::InvalidParams("missing 'action'".into()))?;

    match action {
        "click" => call_click(&args, state),
        "type" => call_type(&args, state),
        "fill_form" => call_fill_form(&args, state),
        "submit" => call_submit(&args, state),
        "press_key" => call_press_key(&args, state),
        other => Err(McpError::InvalidParams(format!("unknown action: {other}"))),
    }
}

fn call_click(args: &Value, state: &mut McpState) -> Result<Value, McpError> {
    let target = require_str(args, "target")?;
    let result = state.engine.click(target)?;
    Ok(serde_json::to_value(result)?)
}

fn call_type(args: &Value, state: &mut McpState) -> Result<Value, McpError> {
    let target = require_str(args, "target")?;
    let text = require_str(args, "text")?;
    state.engine.type_text(target, text)?;
    Ok(serde_json::json!({ "ok": true }))
}

fn call_fill_form(args: &Value, state: &mut McpState) -> Result<Value, McpError> {
    let fields_val = args
        .get("fields")
        .ok_or_else(|| McpError::InvalidParams("missing 'fields'".into()))?;

    let fields: HashMap<String, String> = serde_json::from_value(fields_val.clone())?;
    state.engine.fill_form(&fields)?;
    Ok(serde_json::json!({ "ok": true, "filled": fields.len() }))
}

fn call_submit(args: &Value, state: &mut McpState) -> Result<Value, McpError> {
    let target = args.get("target").and_then(|v| v.as_str());
    let result = state.engine.submit(target)?;
    Ok(serde_json::to_value(result)?)
}

fn call_press_key(args: &Value, state: &mut McpState) -> Result<Value, McpError> {
    let target = require_str(args, "target")?;
    let key = require_str(args, "key")?;
    state.engine.press_key(target, key)?;
    Ok(serde_json::json!({ "ok": true, "key": key }))
}

/// Extract a required string field from args.
fn require_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, McpError> {
    args.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::InvalidParams(format!("missing '{key}'")))
}
