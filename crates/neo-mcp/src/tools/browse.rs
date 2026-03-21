//! `browse` tool — navigate to URL and return structured page data.
//!
//! One tool call = navigate + extract. The AI gets everything in one response.

use serde_json::Value;

use crate::state::McpState;
use crate::McpError;

use super::ToolDef;

/// Tool definition for `tools/list`.
pub(crate) fn definition() -> ToolDef {
    ToolDef {
        name: "browse",
        description: "Navigate to URL and return structured page data",
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "URL to navigate to"
                },
                "extract": {
                    "type": "boolean",
                    "description": "Include WOM extraction (default true)",
                    "default": true
                }
            },
            "required": ["url"]
        }),
    }
}

/// Execute the `browse` tool.
pub fn call(args: Value, state: &mut McpState) -> Result<Value, McpError> {
    let url = args
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::InvalidParams("missing 'url'".into()))?;

    let do_extract = args
        .get("extract")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let page = state.engine.navigate(url)?;

    let wom = if do_extract {
        Some(serde_json::to_value(&page.wom)?)
    } else {
        None
    };

    Ok(serde_json::json!({
        "url": page.url,
        "title": page.title,
        "state": page.state,
        "render_ms": page.render_ms,
        "page_type": page.wom.page_type,
        "summary": page.wom.summary,
        "wom": wom,
        "errors": page.errors,
    }))
}
