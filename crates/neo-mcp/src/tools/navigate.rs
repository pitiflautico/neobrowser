//! `navigate` tool — lightweight in-session navigation.

use serde_json::Value;

use crate::state::McpState;
use crate::McpError;

use super::ToolDef;

pub(crate) fn definition() -> ToolDef {
    ToolDef {
        name: "navigate",
        description: "Navigate within session: go to URL, back, forward, or reload. Returns compact AI view.",
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "URL to navigate to"
                },
                "action": {
                    "type": "string",
                    "enum": ["back", "forward", "reload"],
                    "description": "Navigation action (alternative to url)"
                }
            }
        }),
    }
}

pub fn call(args: Value, state: &mut McpState) -> Result<Value, McpError> {
    let url = args.get("url").and_then(|v| v.as_str());
    let action = args.get("action").and_then(|v| v.as_str());

    let page = match (url, action) {
        (Some(u), _) => state.engine.navigate(u)?,
        (None, Some("back")) => state.engine.back()?,
        (None, Some("forward")) => state.engine.forward()?,
        (None, Some("reload")) => {
            let current = state.engine.current_url().unwrap_or_default();
            if current.is_empty() {
                return Err(McpError::InvalidParams("no page loaded to reload".into()));
            }
            state.engine.navigate(&current)?
        }
        (None, Some(other)) => {
            return Err(McpError::InvalidParams(format!(
                "unknown action: {other}. Use back, forward, or reload"
            )));
        }
        (None, None) => {
            return Err(McpError::InvalidParams(
                "either 'url' or 'action' is required".into(),
            ));
        }
    };

    let view = super::view::render_page(&page.url, &page.title, page.render_ms, &page.wom, &page.errors);
    Ok(serde_json::json!(view))
}
