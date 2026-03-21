//! `trace` tool — execution trace and summary for AI decision-making.

use serde_json::Value;

use crate::state::McpState;
use crate::McpError;

use super::ToolDef;

/// Tool definition for `tools/list`.
pub(crate) fn definition() -> ToolDef {
    ToolDef {
        name: "trace",
        description: "Get execution trace or summary",
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "kind": {
                    "type": "string",
                    "enum": ["summary", "full", "last_action"],
                    "description": "What trace data to return"
                }
            },
            "required": ["kind"]
        }),
    }
}

/// Execute the `trace` tool.
pub fn call(args: Value, state: &mut McpState) -> Result<Value, McpError> {
    let kind = args
        .get("kind")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::InvalidParams("missing 'kind'".into()))?;

    match kind {
        "summary" => {
            let summary = state.engine.summary();
            Ok(serde_json::to_value(summary)?)
        }
        "full" => {
            let entries = state.engine.trace();
            Ok(serde_json::to_value(entries)?)
        }
        "last_action" => {
            let entries = state.engine.trace();
            let last = entries.last().cloned();
            Ok(serde_json::to_value(last)?)
        }
        other => Err(McpError::InvalidParams(format!("unknown kind: {other}"))),
    }
}
