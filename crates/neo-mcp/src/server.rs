//! JSON-RPC transport — read from stdin, dispatch, write to stdout.

use std::io::{self, BufRead, Write};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::state::McpState;
use crate::tools;
use crate::McpError;

/// A JSON-RPC 2.0 request.
#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

/// A JSON-RPC 2.0 response.
#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

/// JSON-RPC error object.
#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
}

/// Main stdio loop — reads line-delimited JSON-RPC, dispatches, responds.
pub fn stdio_loop(state: &mut McpState) -> Result<(), McpError> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let req: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                write_error(&mut stdout, Value::Null, -32700, &e.to_string())?;
                continue;
            }
        };

        let id = req.id.clone().unwrap_or(Value::Null);
        match dispatch(state, &req) {
            Ok(result) => write_result(&mut stdout, id, result)?,
            Err(e) => write_error(&mut stdout, id, error_code(&e), &e.to_string())?,
        }
    }

    Ok(())
}

/// Dispatch a request to the correct handler.
fn dispatch(state: &mut McpState, req: &JsonRpcRequest) -> Result<Value, McpError> {
    match req.method.as_str() {
        "initialize" => handle_initialize(state),
        "tools/list" => {
            require_init(state)?;
            Ok(tools::list_tools())
        }
        "tools/call" => {
            require_init(state)?;
            handle_tool_call(state, &req.params)
        }
        other => Err(McpError::UnknownMethod(other.to_string())),
    }
}

/// Handle `initialize` — mark session as ready.
fn handle_initialize(state: &mut McpState) -> Result<Value, McpError> {
    state.initialized = true;
    Ok(serde_json::json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {
            "tools": {}
        },
        "serverInfo": {
            "name": "neo-mcp",
            "version": env!("CARGO_PKG_VERSION")
        }
    }))
}

/// Handle `tools/call` — route to the correct tool handler.
fn handle_tool_call(state: &mut McpState, params: &Value) -> Result<Value, McpError> {
    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::InvalidParams("missing tool name".into()))?;

    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or(Value::Object(serde_json::Map::new()));

    tools::call_tool(name, args, state)
}

/// Require that `initialize` was called.
fn require_init(state: &McpState) -> Result<(), McpError> {
    if state.initialized {
        Ok(())
    } else {
        Err(McpError::NotInitialized)
    }
}

/// Write a success response to stdout.
fn write_result(out: &mut impl Write, id: Value, result: Value) -> Result<(), McpError> {
    let resp = JsonRpcResponse {
        jsonrpc: "2.0".into(),
        id,
        result: Some(result),
        error: None,
    };
    let line = serde_json::to_string(&resp)?;
    writeln!(out, "{line}")?;
    out.flush()?;
    Ok(())
}

/// Write an error response to stdout.
fn write_error(out: &mut impl Write, id: Value, code: i64, message: &str) -> Result<(), McpError> {
    let resp = JsonRpcResponse {
        jsonrpc: "2.0".into(),
        id,
        result: None,
        error: Some(JsonRpcError {
            code,
            message: message.to_string(),
        }),
    };
    let line = serde_json::to_string(&resp)?;
    writeln!(out, "{line}")?;
    out.flush()?;
    Ok(())
}

/// Map McpError to JSON-RPC error code.
fn error_code(err: &McpError) -> i64 {
    match err {
        McpError::Json(_) => -32700,
        McpError::UnknownMethod(_) => -32601,
        McpError::UnknownTool(_) => -32602,
        McpError::InvalidParams(_) => -32602,
        McpError::NotInitialized => -32002,
        McpError::Engine(_) => -32000,
        McpError::Io(_) => -32000,
    }
}
