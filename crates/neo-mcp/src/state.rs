//! MCP session state — holds the engine and initialization flag.

use neo_engine::BrowserEngine;

/// Session state for one MCP connection.
pub struct McpState {
    /// The browser engine backing all tool calls.
    pub engine: Box<dyn BrowserEngine>,
    /// Whether `initialize` has been called.
    pub initialized: bool,
}

impl McpState {
    /// Create a new session wrapping the given engine.
    pub fn new(engine: Box<dyn BrowserEngine>) -> Self {
        Self {
            engine,
            initialized: false,
        }
    }
}
