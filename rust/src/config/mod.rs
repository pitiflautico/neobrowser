//! Versioned configuration file, with environment variables as overrides.
//!
//! Everything was env-only, which is fine for one setting and unmanageable for
//! fifteen: an MCP client config ends up with a wall of `NEOBROWSER_*` strings that
//! cannot be commented, diffed usefully, or shared between the `safe` setup and the
//! `ci` one. So there is a file, and the env still wins over it.
//!
//! Three deliberate choices:
//!
//! - **`version` is required.** A config format without a version marker cannot be
//!   migrated later without guessing, so the field is mandatory from the first
//!   release rather than added once it is already too late.
//! - **Env overrides file, never the reverse.** A file is a project's default; an env
//!   var is what an operator sets for one run. Inverting that would make a checked-in
//!   file able to silently override what someone typed on the command line.
//! - **Unknown keys are an error, not ignored.** A typo in `polcy` that silently
//!   leaves the policy at its default is the worst possible failure for a security
//!   setting: it looks configured and is not.
//!
//! Split into [`keys`] (the settings table and the schema generated from it), [`parse`]
//! (reading and validating a file) and [`load`] (finding one, and the templates). The
//! `Config` type and the error live here.

use std::collections::BTreeMap;

use thiserror::Error;

pub mod keys;
pub mod load;
pub mod parse;

pub use keys::{json_schema, KEYS};
pub use load::{candidate_paths, load, template, write_template};
pub use parse::parse;

/// The config schema version this build understands.
pub const CURRENT_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("config file not found: {0}")]
    NotFound(String),
    #[error("config is not valid TOML-ish key/value text: {0}")]
    Parse(String),
    #[error(
        "config `version` is missing. Add `version = {CURRENT_VERSION}` so this file can \
         be migrated by future releases"
    )]
    MissingVersion,
    #[error(
        "config version {found} is newer than this build understands ({CURRENT_VERSION}). \
         Upgrade NeoBrowser, or pin the config to a version it supports"
    )]
    TooNew { found: u32 },
    #[error("unknown config key(s): {keys}. Known keys: {known}")]
    UnknownKeys { keys: String, known: String },
    #[error("config key `{key}` has an invalid value {value:?}: {reason}")]
    BadValue {
        key: String,
        value: String,
        reason: String,
    },
}

/// A parsed config file.
#[derive(Debug, Clone, Default)]
pub struct Config {
    pub version: u32,
    values: BTreeMap<String, String>,
}

impl Config {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    pub fn keys(&self) -> Vec<&str> {
        self.values.keys().map(String::as_str).collect()
    }

    /// Apply this config to the process environment, WITHOUT overwriting anything
    /// already set.
    ///
    /// Applied as env rather than threaded through every call site: the rest of the
    /// codebase already reads env, and duplicating a second lookup path in each
    /// module would be a second place for the precedence rule to go wrong. Returns
    /// the keys it actually set.
    pub fn apply_to_env(&self) -> Vec<String> {
        let mut applied = Vec::new();
        for (key, env_var, _) in KEYS {
            let Some(value) = self.get(key) else { continue };
            // An operator's env var beats the file, always.
            if std::env::var_os(env_var).is_some() {
                continue;
            }
            std::env::set_var(env_var, value);
            applied.push((*key).to_string());
        }
        applied
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::load::write_template;
    use super::*;

    #[test]
    fn parses_keys_comments_and_quotes() {
        let c = parse(
            "version = 1\n\
             # a comment\n\
             policy = \"safe\"\n\
             allow_domains = 'example.com, api.example.com'   # trailing comment\n\
             \n\
             session_ttl_days = 7\n",
        )
        .unwrap();
        assert_eq!(c.version, 1);
        assert_eq!(c.get("policy"), Some("safe"));
        assert_eq!(c.get("allow_domains"), Some("example.com, api.example.com"));
        assert_eq!(c.get("session_ttl_days"), Some("7"));
    }

    /// A version marker from day one is what makes a later migration possible.
    #[test]
    fn version_is_required() {
        assert!(matches!(
            parse("policy = \"safe\"\n"),
            Err(ConfigError::MissingVersion)
        ));
    }

    #[test]
    fn a_newer_version_is_refused_with_advice() {
        match parse("version = 99\n") {
            Err(ConfigError::TooNew { found }) => assert_eq!(found, 99),
            other => panic!("expected TooNew, got {other:?}"),
        }
    }

    /// The important one: a typo in a security key must be loud. Silently ignoring
    /// `polcy = "safe"` leaves the policy permissive while looking configured.
    #[test]
    fn unknown_keys_are_an_error_not_ignored() {
        match parse("version = 1\npolcy = \"safe\"\n") {
            Err(ConfigError::UnknownKeys { keys, known }) => {
                assert_eq!(keys, "polcy");
                assert!(known.contains("policy"));
            }
            other => panic!("expected UnknownKeys, got {other:?}"),
        }
    }

    #[test]
    fn invalid_values_are_rejected_at_parse_time() {
        assert!(matches!(
            parse("version = 1\npolicy = \"banana\"\n"),
            Err(ConfigError::BadValue { .. })
        ));
        assert!(matches!(
            parse("version = 1\nmax_download_mb = \"lots\"\n"),
            Err(ConfigError::BadValue { .. })
        ));
        assert!(matches!(
            parse("version = 1\nlog_format = \"xml\"\n"),
            Err(ConfigError::BadValue { .. })
        ));
    }

    #[test]
    fn malformed_lines_and_tables_are_reported_with_a_line_number() {
        match parse("version = 1\nthis is not a pair\n") {
            Err(ConfigError::Parse(m)) => assert!(m.contains("line 2"), "{m}"),
            other => panic!("expected Parse, got {other:?}"),
        }
        match parse("version = 1\n[section]\nkey = 1\n") {
            Err(ConfigError::Parse(m)) => assert!(m.contains("table headers"), "{m}"),
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    /// Precedence: what an operator sets for one run must beat a checked-in file.
    #[test]
    fn env_wins_over_the_config_file() {
        let _g = crate::env_test_guard();
        let prev = std::env::var("NEOBROWSER_POLICY").ok();
        std::env::set_var("NEOBROWSER_POLICY", "developer");

        let c = parse("version = 1\npolicy = \"autonomous\"\n").unwrap();
        let applied = c.apply_to_env();

        assert!(
            !applied.contains(&"policy".to_string()),
            "the file must not overwrite an env var the operator set"
        );
        assert_eq!(std::env::var("NEOBROWSER_POLICY").unwrap(), "developer");

        match prev {
            Some(v) => std::env::set_var("NEOBROWSER_POLICY", v),
            None => std::env::remove_var("NEOBROWSER_POLICY"),
        }
    }

    #[test]
    fn the_file_fills_in_what_env_does_not_set() {
        let _g = crate::env_test_guard();
        std::env::remove_var("NEOBROWSER_MAX_DOWNLOAD_MB");
        let c = parse("version = 1\nmax_download_mb = 42\n").unwrap();
        let applied = c.apply_to_env();
        assert!(applied.contains(&"max_download_mb".to_string()));
        assert_eq!(std::env::var("NEOBROWSER_MAX_DOWNLOAD_MB").unwrap(), "42");
        std::env::remove_var("NEOBROWSER_MAX_DOWNLOAD_MB");
    }

    #[test]
    fn every_template_is_valid_and_parseable() {
        for name in ["safe", "developer", "autonomous", "ci"] {
            let text = template(name).unwrap_or_else(|| panic!("no template for {name}"));
            let c = parse(&text).unwrap_or_else(|e| panic!("{name} template invalid: {e}"));
            assert_eq!(c.version, CURRENT_VERSION);
        }
        assert!(template("nope").is_none());
    }

    /// The `autonomous` template must not ship an empty allowlist, or following the
    /// documented starting point would produce a session that refuses everything.
    #[test]
    fn the_autonomous_template_ships_an_allowlist() {
        let c = parse(&template("autonomous").unwrap()).unwrap();
        assert!(
            c.get("allow_domains").is_some_and(|d| !d.trim().is_empty()),
            "the autonomous template needs a non-empty allow_domains"
        );
    }

    /// The schema is generated from KEYS, so it can never drift from the parser.
    #[test]
    fn the_schema_covers_exactly_the_known_keys() {
        let schema = json_schema();
        let props = schema["properties"].as_object().unwrap();
        assert!(props.contains_key("version"));
        for (key, _, _) in KEYS {
            assert!(props.contains_key(*key), "schema missing {key}");
        }
        // version + every key, and nothing else.
        assert_eq!(props.len(), KEYS.len() + 1);
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["required"][0], "version");
    }

    #[test]
    fn write_template_refuses_to_clobber() {
        let dir = std::env::temp_dir().join(format!("nb-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("neobrowser.toml");
        write_template(&p, "safe").unwrap();
        assert!(p.exists());
        // Second write must refuse rather than replace a real config.
        assert!(matches!(
            write_template(&p, "safe"),
            Err(ConfigError::BadValue { .. })
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_explicit_config_path_disables_the_fallback_search() {
        let _g = crate::env_test_guard();
        std::env::set_var("NEOBROWSER_CONFIG", "/tmp/explicit-neobrowser.toml");
        let paths = candidate_paths();
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0], PathBuf::from("/tmp/explicit-neobrowser.toml"));
        std::env::remove_var("NEOBROWSER_CONFIG");
    }
}
