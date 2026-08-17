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
    /// Resolved once at startup: the policy evaluated before every dispatch. Held
    /// here rather than read from the environment per call so a session cannot have
    /// its rules changed underneath it mid-run.
    pub policy: Arc<crate::policy::Policy>,
    /// This session's trace. Shared so tools can add evidence to the same timeline
    /// the dispatch layer is already writing to.
    pub trace: Arc<crate::trace::Trace>,
    /// The Chrome bridge, when enabled. `None` is the ordinary case, so the bridge
    /// tools can report "not enabled" with instructions rather than failing opaquely.
    pub bridge: Option<Arc<crate::bridge::Bridge>>,
}

/// A callable tool.
#[async_trait]
pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;
    async fn call(&self, ctx: &ToolCtx, args: &Map<String, Value>)
        -> Result<ToolOutput, ToolError>;
}

/// Which tools an MCP client sees.
///
/// The full set is 55 tools, and every one of them costs schema in the model's
/// context on every single request — before it has done anything. Most sessions use
/// eight or nine. `Core` advertises the ones that cover ordinary work; `Full` keeps
/// everything for scripted callers and expert use.
///
/// Deliberately a *filter over one registry*, not a second façade layer with its own
/// names. A parallel `browser_*` API mapping onto these tools would mean two
/// surfaces to keep in step and two places for behaviour to drift — and the names
/// would collide with Playwright MCP's for no benefit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toolset {
    /// The default: enough to navigate, observe, act, extract and debug.
    Core,
    /// Everything registered.
    Full,
}

impl Toolset {
    pub fn from_env() -> Self {
        match std::env::var("NEOBROWSER_TOOLSET")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "full" | "all" => Toolset::Full,
            // Anything unrecognised falls back to the default rather than erroring:
            // a typo should not leave a client with no tools at all.
            _ => Toolset::Core,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Toolset::Core => "core",
            Toolset::Full => "full",
        }
    }
}

/// The core set. Chosen by what a session actually needs end to end, not by
/// category: observe/act/verify, plus the escape hatches (`js`) and the debugging
/// entry points a developer reaches for first.
pub const CORE_TOOLS: &[&str] = &[
    "status",
    "navigate",
    "observe",
    "read",
    "find",
    "click",
    "type",
    "fill",
    "form_fill",
    "submit",
    "press",
    "screenshot",
    "extract",
    "search",
    "upload",
    "download",
    "js",
    "wait",
    "new_tab",
    "list_tabs",
    "switch_tab",
    "close_tab",
    "console_logs",
    "network_log",
    "perf_trace",
    "session_info",
];

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

    /// Descriptors for `tools/list`, filtered by the active toolset.
    ///
    /// Filtering the *advertisement* only: a tool outside the core set is still
    /// callable if a client knows its name. Hiding a tool from the catalogue reduces
    /// context; refusing to run it would break scripted callers for no security gain,
    /// since the policy engine is what decides what is allowed.
    pub fn descriptors_for(&self, set: Toolset) -> Vec<Value> {
        self.tools
            .iter()
            .filter(|t| match set {
                Toolset::Full => true,
                Toolset::Core => CORE_TOOLS.contains(&t.spec().name),
            })
            .map(|t| t.spec().descriptor())
            .collect()
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

#[cfg(test)]
mod toolset_tests {
    use super::*;

    /// Every core name must actually exist, or `tools/list` would silently advertise
    /// fewer tools than intended and nobody would notice.
    #[test]
    fn every_core_tool_name_is_registered() {
        let reg = crate::tool_impls::build_registry();
        let registered: std::collections::HashSet<&str> = reg
            .descriptors()
            .iter()
            .filter_map(|d| d.get("name").and_then(Value::as_str))
            .map(|s| Box::leak(s.to_string().into_boxed_str()) as &str)
            .collect();
        let missing: Vec<&&str> = CORE_TOOLS
            .iter()
            .filter(|n| !registered.contains(**n))
            .collect();
        assert!(
            missing.is_empty(),
            "CORE_TOOLS names a tool that does not exist: {missing:?}"
        );
    }

    #[test]
    fn the_core_set_is_materially_smaller_than_full() {
        let reg = crate::tool_impls::build_registry();
        let core = reg.descriptors_for(Toolset::Core).len();
        let full = reg.descriptors_for(Toolset::Full).len();
        assert_eq!(core, CORE_TOOLS.len());
        assert!(
            core < full,
            "core ({core}) should be smaller than full ({full}) or the filter is pointless"
        );
    }

    /// The core set has to be able to complete a task on its own: observe, act,
    /// verify, extract. A "slim" set missing one of those just forces `full`.
    #[test]
    fn the_core_set_covers_a_whole_workflow() {
        for essential in [
            "navigate",
            "observe",
            "click",
            "type",
            "read",
            "extract",
            "screenshot",
        ] {
            assert!(
                CORE_TOOLS.contains(&essential),
                "{essential} is required for the core set to be usable alone"
            );
        }
    }

    #[test]
    fn toolset_parsing_defaults_to_core_on_anything_unrecognised() {
        let _g = crate::env_test_guard();
        let prev = std::env::var("NEOBROWSER_TOOLSET").ok();
        for (value, expected) in [
            ("full", Toolset::Full),
            ("ALL", Toolset::Full),
            ("core", Toolset::Core),
            ("", Toolset::Core),
            ("banana", Toolset::Core),
        ] {
            std::env::set_var("NEOBROWSER_TOOLSET", value);
            assert_eq!(Toolset::from_env(), expected, "value {value:?}");
        }
        match prev {
            Some(v) => std::env::set_var("NEOBROWSER_TOOLSET", v),
            None => std::env::remove_var("NEOBROWSER_TOOLSET"),
        }
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
