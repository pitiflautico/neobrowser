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

/// Domain allowlist for `navigate`, from `NEOBROWSER_DOMAIN_ALLOWLIST`
/// (comma-separated; exact host or `*.suffix` for any subdomain, e.g.
/// `github.com,*.docs.rs`). Unset or empty means everything is allowed.
/// Returns Err with an actionable message when the URL's host is not listed.
pub fn check_domain_allowlist(url: &str) -> Result<(), String> {
    let raw = match std::env::var("NEOBROWSER_DOMAIN_ALLOWLIST") {
        Ok(v) if !v.trim().is_empty() => v,
        _ => return Ok(()),
    };
    let host = reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
        .ok_or_else(|| format!("navigate: cannot parse host from '{url}'"))?;
    let entries: Vec<String> = raw
        .split(',')
        .map(|e| e.trim().to_ascii_lowercase())
        .filter(|e| !e.is_empty())
        .collect();
    let allowed = entries.iter().any(|e| {
        if let Some(suffix) = e.strip_prefix("*.") {
            host == suffix || host.ends_with(&format!(".{suffix}"))
        } else {
            &host == e
        }
    });
    if allowed {
        Ok(())
    } else {
        Err(format!(
            "navigate: '{host}' is not in NEOBROWSER_DOMAIN_ALLOWLIST (allowed: {})",
            entries.join(", ")
        ))
    }
}

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
    fn unknown_arg_gets_near_miss_hint() {
        let err = spec()
            .validate_args(&args(json!({ "inten": "x", "ms": 1 })))
            .unwrap_err();
        assert!(err.to_string().contains("Did you mean: inten → intent?"));
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
    fn domain_allowlist_blocks_unlisted_hosts() {
        // Single test fn so env manipulation can't race across test threads.
        const VAR: &str = "NEOBROWSER_DOMAIN_ALLOWLIST";
        std::env::remove_var(VAR);
        // Unset: everything allowed.
        assert!(crate::tools::check_domain_allowlist("https://anything.example/x").is_ok());
        // Exact hosts.
        std::env::set_var(VAR, "github.com,docs.rs");
        assert!(crate::tools::check_domain_allowlist("https://github.com/a").is_ok());
        assert!(crate::tools::check_domain_allowlist("https://docs.rs/").is_ok());
        let err = crate::tools::check_domain_allowlist("https://evil.com/").unwrap_err();
        assert!(err.contains("not in NEOBROWSER_DOMAIN_ALLOWLIST"));
        // Exact entries must not leak to subdomains.
        assert!(crate::tools::check_domain_allowlist("https://sub.github.com/").is_err());
        // Wildcard entries cover the apex and subdomains.
        std::env::set_var(VAR, "*.example.com");
        assert!(crate::tools::check_domain_allowlist("https://example.com/").is_ok());
        assert!(crate::tools::check_domain_allowlist("https://a.b.example.com/").is_ok());
        assert!(crate::tools::check_domain_allowlist("https://notexample.com/").is_err());
        // Case-insensitive, spaces tolerated.
        std::env::set_var(VAR, " GitHub.COM ");
        assert!(crate::tools::check_domain_allowlist("https://github.com/").is_ok());
        std::env::remove_var(VAR);
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
