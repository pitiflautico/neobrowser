//! `wait` tool — wait for an element or text to appear on the page.

use serde_json::Value;

use crate::state::McpState;
use crate::McpError;

use super::ToolDef;

/// Tool definition for `tools/list`.
pub(crate) fn definition() -> ToolDef {
    ToolDef {
        name: "wait",
        description: "Wait for an element (CSS selector) or visible text to appear on the page",
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "selector": {
                    "type": "string",
                    "description": "CSS selector to wait for (alternative to text)"
                },
                "text": {
                    "type": "string",
                    "description": "Visible text content to wait for (alternative to selector)"
                },
                "timeout_ms": {
                    "type": "integer",
                    "description": "Timeout in milliseconds (default 5000)",
                    "default": 5000
                }
            }
        }),
    }
}

/// Execute the `wait` tool.
pub fn call(args: Value, state: &mut McpState) -> Result<Value, McpError> {
    let selector = args.get("selector").and_then(|v| v.as_str());
    let text = args.get("text").and_then(|v| v.as_str());

    let timeout_ms = args
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(5000) as u32;

    match (selector, text) {
        (Some(sel), _) => {
            // Wait for CSS selector.
            let found = state.engine.wait_for(sel, timeout_ms)?;
            Ok(serde_json::json!({ "found": found, "selector": sel }))
        }
        (None, Some(txt)) => {
            // Wait for visible text content.
            let found = state.engine.wait_for_text(txt, timeout_ms)?;
            Ok(serde_json::json!({ "found": found, "text": txt }))
        }
        (None, None) => Err(McpError::InvalidParams(
            "either 'selector' or 'text' is required".into(),
        )),
    }
}
