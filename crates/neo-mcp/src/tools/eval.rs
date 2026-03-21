//! `eval` tool — execute arbitrary JavaScript on the current page.

use serde_json::Value;

use crate::state::McpState;
use crate::McpError;

use super::ToolDef;

/// Tool definition for `tools/list`.
pub(crate) fn definition() -> ToolDef {
    ToolDef {
        name: "eval",
        description: "Execute arbitrary JavaScript on the current page and return the result",
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "code": {
                    "type": "string",
                    "description": "JavaScript code to execute"
                }
            },
            "required": ["code"]
        }),
    }
}

/// Execute the `eval` tool.
pub fn call(args: Value, state: &mut McpState) -> Result<Value, McpError> {
    let code = args
        .get("code")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::InvalidParams("missing 'code'".into()))?;

    let result = state.engine.eval(code)?;
    Ok(serde_json::json!({ "result": result }))
}
