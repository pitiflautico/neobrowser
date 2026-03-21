//! neo-mcp — MCP (Model Context Protocol) server for NeoRender.
//!
//! JSON-RPC over stdio. AI agents send tool calls, neo-mcp routes them
//! to a [`BrowserEngine`] and returns structured results.
//!
//! Tools: `browse`, `interact`, `extract`, `trace`.

pub mod mock;
pub mod server;
pub mod state;
pub mod tools;

use neo_engine::BrowserEngine;

/// Errors from the MCP server layer.
#[derive(Debug, thiserror::Error)]
pub enum McpError {
    /// JSON parse/serialize failure.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    /// IO failure (stdin/stdout).
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// Engine returned an error.
    #[error("engine: {0}")]
    Engine(#[from] neo_engine::EngineError),

    /// Unknown method in JSON-RPC request.
    #[error("unknown method: {0}")]
    UnknownMethod(String),

    /// Unknown tool name.
    #[error("unknown tool: {0}")]
    UnknownTool(String),

    /// Invalid tool parameters.
    #[error("invalid params: {0}")]
    InvalidParams(String),

    /// Server not yet initialized.
    #[error("server not initialized — call initialize first")]
    NotInitialized,
}

/// Run the MCP server on stdin/stdout with the given engine.
///
/// Blocks until stdin closes or a fatal error occurs.
pub fn run_server(engine: Box<dyn BrowserEngine>) -> Result<(), McpError> {
    let mut state = state::McpState::new(engine);
    server::stdio_loop(&mut state)
}
