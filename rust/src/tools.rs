//! Tool registry, schemas, argument validation, and the `Tool` trait.
//!
//! Mirrors the Python `TOOLS` dict + `_validate_args` + `dispatch_tool`, but with
//! typed specs and a trait-object registry so each tool is a self-contained unit
//! that the MCP layer (see `mcp.rs`) can list and call generically.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Map, Value};
use thiserror::Error;

use crate::browser::Browser;

/// JSON-Schema-ish parameter type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamType {
    String,
    Number,
    Integer,
    Boolean,
    Object,
    Array,
}

impl ParamType {
    pub fn as_json(self) -> &'static str {
        match self {
            ParamType::String => "string",
            ParamType::Number => "number",
            ParamType::Integer => "integer",
            ParamType::Boolean => "boolean",
            ParamType::Object => "object",
            ParamType::Array => "array",
        }
    }
}

/// A single tool parameter.
#[derive(Debug, Clone)]
pub struct ParamSpec {
    pub name: &'static str,
    pub ty: ParamType,
    pub description: &'static str,
    pub required: bool,
    pub enum_values: &'static [&'static str],
}

impl ParamSpec {
    pub const fn new(name: &'static str, ty: ParamType, description: &'static str) -> Self {
        Self {
            name,
            ty,
            description,
            required: false,
            enum_values: &[],
        }
    }
    pub const fn required(mut self) -> Self {
        self.required = true;
        self
    }
    pub const fn with_enum(mut self, values: &'static [&'static str]) -> Self {
        self.enum_values = values;
        self
    }
}

/// A tool's public contract (name, one-line description, ordered params).
#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub params: Vec<ParamSpec>,
}

impl ToolSpec {
    /// Emit the MCP `inputSchema` object.
    pub fn input_schema(&self) -> Value {
        let mut properties = Map::new();
        let mut required = Vec::new();
        for p in &self.params {
            let mut prop = Map::new();
            prop.insert("type".into(), Value::String(p.ty.as_json().into()));
            prop.insert("description".into(), Value::String(p.description.into()));
            if !p.enum_values.is_empty() {
                prop.insert(
                    "enum".into(),
                    Value::Array(
                        p.enum_values
                            .iter()
                            .map(|s| Value::String((*s).into()))
                            .collect(),
                    ),
                );
            }
            properties.insert(p.name.into(), Value::Object(prop));
            if p.required {
                required.push(Value::String(p.name.into()));
            }
        }
        serde_json::json!({
            "type": "object",
            "properties": Value::Object(properties),
            "required": Value::Array(required),
        })
    }

    /// The full MCP tool descriptor for `tools/list`.
    pub fn descriptor(&self) -> Value {
        serde_json::json!({
            "name": self.name,
            "description": self.description,
            "inputSchema": self.input_schema(),
        })
    }

    /// Reject unknown and missing-required arguments. Messages match the Python
    /// `_validate_args` so the model gets the same corrective feedback.
    pub fn validate_args(&self, args: &Map<String, Value>) -> Result<(), ToolError> {
        // Unknown args, sorted (BTreeMap over the arg keys gives sorted order).
        let known: BTreeMap<&str, ()> = self.params.iter().map(|p| (p.name, ())).collect();
        let mut unknown: Vec<&str> = args
            .keys()
            .filter(|k| !known.contains_key(k.as_str()))
            .map(|k| k.as_str())
            .collect();
        unknown.sort_unstable();
        if !unknown.is_empty() {
            let valid = if self.params.is_empty() {
                "(this tool takes no arguments)".to_string()
            } else {
                self.params
                    .iter()
                    .map(|p| p.name)
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            return Err(ToolError::Argument(format!(
                "{}: unknown argument(s): {}. Valid: {}",
                self.name,
                unknown.join(", "),
                valid
            )));
        }
        // Missing required, in schema order.
        let missing: Vec<&str> = self
            .params
            .iter()
            .filter(|p| p.required && !args.contains_key(p.name))
            .map(|p| p.name)
            .collect();
        if !missing.is_empty() {
            return Err(ToolError::Argument(format!(
                "{}: missing required argument(s): {}",
                self.name,
                missing.join(", ")
            )));
        }
        Ok(())
    }
}

/// What a tool produces on success.
#[derive(Debug, Clone)]
pub enum ToolOutput {
    Text(String),
    Image { data: String, mime: String },
}

impl ToolOutput {
    pub fn text(s: impl Into<String>) -> Self {
        ToolOutput::Text(s.into())
    }
}

/// Tool failure. `Argument` is a caller error (bad params); `Failed` is a runtime
/// fault. Both surface to the model as MCP `isError` text, never a crash.
#[derive(Debug, Error)]
pub enum ToolError {
    #[error("{0}")]
    Argument(String),
    #[error("{0}")]
    Failed(String),
}

impl From<crate::cdp::CdpError> for ToolError {
    fn from(e: crate::cdp::CdpError) -> Self {
        ToolError::Failed(e.to_string())
    }
}

impl From<crate::chrome::ChromeError> for ToolError {
    fn from(e: crate::chrome::ChromeError) -> Self {
        ToolError::Failed(e.to_string())
    }
}

/// Shared context handed to every tool call.
#[derive(Clone)]
pub struct ToolCtx {
    pub browser: Arc<Browser>,
    /// The tool registry, so meta-tools (replay) can re-invoke other tools.
    pub registry: Arc<Registry>,
}

/// A callable tool.
#[async_trait]
pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;
    async fn call(&self, ctx: &ToolCtx, args: &Map<String, Value>)
        -> Result<ToolOutput, ToolError>;
}

/// The set of tools this server exposes. Grows per phase; `tools/list` only ever
/// advertises registered (i.e. genuinely working) tools.
#[derive(Default)]
pub struct Registry {
    tools: Vec<Arc<dyn Tool>>,
}

impl Registry {
    pub fn new() -> Self {
        Self { tools: Vec::new() }
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.push(tool);
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.iter().find(|t| t.spec().name == name)
    }

    pub fn descriptors(&self) -> Vec<Value> {
        self.tools.iter().map(|t| t.spec().descriptor()).collect()
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn spec() -> ToolSpec {
        ToolSpec {
            name: "wait",
            description: "wait a bit",
            params: vec![
                ParamSpec::new("ms", ParamType::Integer, "milliseconds"),
                ParamSpec::new("intent", ParamType::String, "what").required(),
            ],
        }
    }

    fn args(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn unknown_arg_message_matches_python() {
        let err = spec()
            .validate_args(&args(json!({ "seconds": 5, "intent": "x" })))
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "wait: unknown argument(s): seconds. Valid: ms, intent"
        );
    }

    #[test]
    fn unknown_args_are_sorted() {
        let err = spec()
            .validate_args(&args(json!({ "zebra": 1, "apple": 2, "intent": "x" })))
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "wait: unknown argument(s): apple, zebra. Valid: ms, intent"
        );
    }

    #[test]
    fn missing_required_message_matches_python() {
        let err = spec().validate_args(&args(json!({ "ms": 5 }))).unwrap_err();
        assert_eq!(
            err.to_string(),
            "wait: missing required argument(s): intent"
        );
    }

    #[test]
    fn valid_args_pass() {
        assert!(spec()
            .validate_args(&args(json!({ "ms": 5, "intent": "x" })))
            .is_ok());
    }

    #[test]
    fn no_arg_tool_reports_takes_none() {
        let s = ToolSpec {
            name: "status",
            description: "status",
            params: vec![],
        };
        let err = s.validate_args(&args(json!({ "x": 1 }))).unwrap_err();
        assert_eq!(
            err.to_string(),
            "status: unknown argument(s): x. Valid: (this tool takes no arguments)"
        );
    }

    #[test]
    fn input_schema_shape() {
        let schema = spec().input_schema();
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["properties"]["ms"]["type"], "integer");
        assert_eq!(schema["required"], json!(["intent"]));
    }
}
