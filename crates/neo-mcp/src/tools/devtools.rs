//! `devtools` tool — browser DevTools: network traffic, console logs, errors, cookies.
//!
//! Exposes the JS-side `__neo_network`, `__neo_console`, `__neo_errors`, and `__neo_cookies`
//! inspectors for debugging and security testing.

use serde_json::Value;

use crate::state::McpState;
use crate::McpError;

use super::ToolDef;

pub(crate) fn definition() -> ToolDef {
    ToolDef {
        name: "devtools",
        description: "Browser DevTools: network traffic, console logs, errors, cookies. Panels: network, network_all, network_failed, console, console_errors, errors, errors_all, cookies, all. Optional filter for network (URL substring) or console (text search).",
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "panel": {
                    "type": "string",
                    "enum": ["network", "network_all", "network_failed", "console", "console_errors", "errors", "errors_all", "cookies", "all"],
                    "description": "Which DevTools panel to show"
                },
                "filter": {
                    "type": "string",
                    "description": "Filter pattern (network: URL substring, console: text search)"
                }
            },
            "required": ["panel"]
        }),
    }
}

pub fn call(args: Value, state: &mut McpState) -> Result<Value, McpError> {
    let panel = args
        .get("panel")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::InvalidParams("missing 'panel'".into()))?;

    let filter = args
        .get("filter")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Validate panel
    match panel {
        "network" | "network_all" | "network_failed" | "console" | "console_errors"
        | "errors" | "errors_all" | "cookies" | "all" => {}
        other => {
            return Err(McpError::InvalidParams(format!("unknown panel: {other}")));
        }
    }

    // Build JS call — filter is escaped for safety
    let escaped_filter = filter.replace('\\', "\\\\").replace('\'', "\\'");
    let js = format!("__neo_devtools('{}', '{}')", panel, escaped_filter);

    let result_str = state.engine.eval(&js)?;

    // Parse the JSON string returned by JS, then return as structured Value
    match serde_json::from_str::<Value>(&result_str) {
        Ok(parsed) => Ok(parsed),
        Err(_) => {
            // If it's not valid JSON, return as raw string
            Ok(serde_json::json!({ "raw": result_str }))
        }
    }
}
