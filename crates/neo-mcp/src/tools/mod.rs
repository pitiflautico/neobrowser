//! Tool registry — definitions and dispatch for all MCP tools.

pub mod browse;
pub mod extract;
pub mod interact;
pub mod search;
pub mod trace;

use serde_json::Value;

use crate::state::McpState;
use crate::McpError;

/// Tool definition for `tools/list` response.
pub(crate) struct ToolDef {
    name: &'static str,
    description: &'static str,
    schema: Value,
}

/// Return the `tools/list` response with all tool definitions.
pub fn list_tools() -> Value {
    let tools: Vec<ToolDef> = vec![
        browse::definition(),
        interact::definition(),
        extract::definition(),
        search::definition(),
        trace::definition(),
    ];

    let entries: Vec<Value> = tools
        .into_iter()
        .map(|t| {
            serde_json::json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": t.schema,
            })
        })
        .collect();

    serde_json::json!({ "tools": entries })
}

/// Dispatch a tool call by name.
pub fn call_tool(name: &str, args: Value, state: &mut McpState) -> Result<Value, McpError> {
    match name {
        "browse" => browse::call(args, state),
        "interact" => interact::call(args, state),
        "extract" => extract::call(args, state),
        "search" => search::call(args, state),
        "trace" => trace::call(args, state),
        other => Err(McpError::UnknownTool(other.to_string())),
    }
}
