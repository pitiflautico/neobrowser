//! Tool registry, schemas, argument validation, and the `Tool` trait.
//!
//! Mirrors the Python `TOOLS` dict + `_validate_args` + `dispatch_tool`, but with
//! typed specs and a trait-object registry so each tool is a self-contained unit
//! that the MCP layer (see `mcp.rs`) can list and call generically.
//!
//! Split into [`spec`] (describing a tool and deriving its schema), [`result`] (what a tool
//! returns and how it fails), [`ctx`] (the context tools receive and the trait they
//! implement) and [`catalogue`] (which tools exist and which are advertised).

pub mod catalogue;
pub mod ctx;
pub mod result;
pub mod spec;

pub use catalogue::{Registry, Toolset, CORE_TOOLS};
pub use ctx::{Tool, ToolCtx};
pub use result::{ToolError, ToolOutput};
pub use spec::{ParamSpec, ParamType, ToolSpec};

#[cfg(test)]
mod toolset_tests {
    use serde_json::Value;

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
    use serde_json::{json, Map, Value};

    use super::*;

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
