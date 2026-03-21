//! `wait` tool — wait for an element to appear on the page.

use serde_json::Value;

use crate::state::McpState;
use crate::McpError;

use super::ToolDef;

/// Tool definition for `tools/list`.
pub(crate) fn definition() -> ToolDef {
    ToolDef {
        name: "wait",
        description: "Wait for an element to appear on the page",
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "selector": {
                    "type": "string",
                    "description": "CSS selector to wait for"
                },
                "timeout_ms": {
                    "type": "integer",
                    "description": "Timeout in milliseconds (default 5000)",
                    "default": 5000
                }
            },
            "required": ["selector"]
        }),
    }
}

/// Execute the `wait` tool.
pub fn call(args: Value, state: &mut McpState) -> Result<Value, McpError> {
    let selector = args
        .get("selector")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::InvalidParams("missing 'selector'".into()))?;

    let timeout_ms = args
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(5000) as u32;

    let found = state.engine.wait_for(selector, timeout_ms)?;
    Ok(serde_json::json!({ "found": found, "selector": selector }))
}
