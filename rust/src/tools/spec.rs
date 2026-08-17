//! Describing a tool: its parameters, their types, and the JSON Schema derived from them.
//!
//! One declaration produces the schema a client validates against, the Markdown in the docs,
//! and the runtime coercion. Declaring those separately is how a tool's documented signature
//! drifts from the one it actually accepts.

//! Tool registry, schemas, argument validation, and the `Tool` trait.
//!
//! Mirrors the Python `TOOLS` dict + `_validate_args` + `dispatch_tool`, but with
//! typed specs and a trait-object registry so each tool is a self-contained unit
//! that the MCP layer (see `mcp.rs`) can list and call generically.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use super::result::ToolError;

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
