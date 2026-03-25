//! `browse` tool — navigate to URL and return AI-oriented compact view.

use serde_json::Value;

use crate::state::McpState;
use crate::McpError;

use super::ToolDef;

pub(crate) fn definition() -> ToolDef {
    ToolDef {
        name: "browse",
        description: "Navigate to URL. Returns compact AI view: title, forms, buttons, links, text. Use 'extract' for full data.",
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "URL to navigate to"
                }
            },
            "required": ["url"]
        }),
    }
}

pub fn call(args: Value, state: &mut McpState) -> Result<Value, McpError> {
    let url = args
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::InvalidParams("missing 'url'".into()))?;

    let page = state.engine.navigate(url)?;
    let view = super::view::render_page(&page.url, &page.title, page.render_ms, &page.wom, &page.errors);

    Ok(serde_json::json!(view))
}
