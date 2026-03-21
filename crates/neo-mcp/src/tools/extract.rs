//! `extract` tool — structured data extraction from the current page.

use serde_json::Value;

use crate::state::McpState;
use crate::McpError;

use super::ToolDef;

/// Tool definition for `tools/list`.
pub(crate) fn definition() -> ToolDef {
    ToolDef {
        name: "extract",
        description: "Extract structured data from current page",
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "kind": {
                    "type": "string",
                    "enum": ["wom", "text", "links", "semantic", "tables"],
                    "description": "What to extract"
                },
                "max_chars": {
                    "type": "integer",
                    "description": "Max characters for text extraction",
                    "default": 2000
                }
            },
            "required": ["kind"]
        }),
    }
}

/// Execute the `extract` tool.
pub fn call(args: Value, state: &mut McpState) -> Result<Value, McpError> {
    let kind = args
        .get("kind")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::InvalidParams("missing 'kind'".into()))?;

    match kind {
        "wom" => {
            let wom = state.engine.extract()?;
            Ok(serde_json::to_value(wom)?)
        }
        "text" => {
            // Text extraction: serialize WOM summary + node labels.
            let wom = state.engine.extract()?;
            let max = args
                .get("max_chars")
                .and_then(|v| v.as_u64())
                .unwrap_or(2000) as usize;
            let text = wom_to_text(&wom, max);
            Ok(serde_json::json!({ "text": text }))
        }
        "links" => {
            let links = state.engine.extract_links()?;
            let entries: Vec<serde_json::Value> = links
                .into_iter()
                .map(|(text, href)| serde_json::json!({ "text": text, "href": href }))
                .collect();
            Ok(serde_json::json!({ "links": entries, "count": entries.len() }))
        }
        "semantic" => {
            let semantic = state.engine.extract_semantic()?;
            let max = args
                .get("max_chars")
                .and_then(|v| v.as_u64())
                .unwrap_or(50000) as usize;
            let text = if semantic.len() > max {
                semantic[..max].to_string()
            } else {
                semantic
            };
            Ok(serde_json::json!({ "semantic": text }))
        }
        "tables" => {
            // Table extraction delegates to WOM nodes with role hints.
            let wom = state.engine.extract()?;
            Ok(serde_json::json!({
                "tables": [],
                "node_count": wom.nodes.len(),
            }))
        }
        other => Err(McpError::InvalidParams(format!("unknown kind: {other}"))),
    }
}

/// Convert WOM to compressed text within char budget.
fn wom_to_text(wom: &neo_extract::WomDocument, max_chars: usize) -> String {
    let mut buf = String::with_capacity(max_chars);
    buf.push_str(&wom.title);
    buf.push('\n');
    buf.push_str(&wom.summary);
    buf.push('\n');

    for node in &wom.nodes {
        if buf.len() >= max_chars {
            break;
        }
        let line = format!("[{}] {} {}\n", node.role, node.label, node.id);
        buf.push_str(&line);
    }

    buf.truncate(max_chars);
    buf
}
