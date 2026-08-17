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

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use thiserror::Error;

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

/// Every recognised key, its env-var equivalent, and one line of help.
///
/// Single source of truth: the parser, the JSON Schema, and the docs are all derived
/// from this table, so they cannot drift apart.
pub const KEYS: &[(&str, &str, &str)] = &[
    (
        "profile",
        "NEOBROWSER_PROFILE",
        "Ghost profile name for this session's browser data",
    ),
    (
        "real_profile",
        "NEOBROWSER_REAL_PROFILE",
        "Real Chrome profile folder to import cookies from (advanced)",
    ),
    (
        "attach_port",
        "NEOBROWSER_ATTACH_PORT",
        "Attach to an already-running Chrome on this debug port",
    ),
    (
        "chrome_bin",
        "NEOBROWSER_CHROME_BIN",
        "Path to the Chrome/Chromium binary",
    ),
    ("home", "NEOBROWSER_HOME", "Base directory for all state"),
    ("proxy", "NEOBROWSER_PROXY", "Upstream proxy URL"),
    (
        "policy",
        "NEOBROWSER_POLICY",
        "Policy profile: developer | safe | autonomous",
    ),
    (
        "allow_domains",
        "NEOBROWSER_ALLOW_DOMAINS",
        "Comma-separated host suffixes the agent may reach (exclusive once set)",
    ),
    (
        "deny_domains",
        "NEOBROWSER_DENY_DOMAINS",
        "Comma-separated host suffixes always refused",
    ),
    (
        "upload_dir",
        "NEOBROWSER_UPLOAD_DIR",
        "The only directory `upload` may read from",
    ),
    (
        "session_ttl_days",
        "NEOBROWSER_SESSION_TTL_DAYS",
        "Lifetime of stored session material; 0 disables expiry",
    ),
    (
        "max_download_mb",
        "NEOBROWSER_MAX_DOWNLOAD_MB",
        "Maximum download size in MiB",
    ),
    (
        "allow_no_sandbox",
        "NEOBROWSER_ALLOW_NO_SANDBOX",
        "Last resort: run Chrome without its sandbox (1 | with-real-profile)",
    ),
    (
        "disable_gpu",
        "NEOBROWSER_DISABLE_GPU",
        "Force software rendering (GPU-less hosts)",
    ),
    (
        "log_format",
        "NEOBROWSER_LOG_FORMAT",
        "text | json — JSON logs carry task/action/trace ids",
    ),
];

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

/// Parse config text: `key = value` lines, `#` comments, optional quotes.
///
/// A deliberately tiny subset of TOML rather than a dependency. The config is flat
/// key/value by design — nesting would buy nothing and cost a parser.
pub fn parse(text: &str) -> Result<Config, ConfigError> {
    let mut values = BTreeMap::new();
    let mut version: Option<u32> = None;
    let known: BTreeMap<&str, ()> = KEYS.iter().map(|(k, _, _)| (*k, ())).collect();
    let mut unknown = Vec::new();

    for (lineno, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        // A TOML table header would mean the file is richer than this parser handles;
        // say so instead of silently ignoring everything under it.
        if line.starts_with('[') {
            return Err(ConfigError::Parse(format!(
                "line {}: table headers are not supported; the config is flat key = value",
                lineno + 1
            )));
        }
        let Some((k, v)) = line.split_once('=') else {
            return Err(ConfigError::Parse(format!(
                "line {}: expected `key = value`, got {line:?}",
                lineno + 1
            )));
        };
        let key = k.trim().to_ascii_lowercase();
        let value = v.trim().trim_matches('"').trim_matches('\'').to_string();

        if key == "version" {
            version = Some(value.parse().map_err(|_| ConfigError::BadValue {
                key: "version".into(),
                value: value.clone(),
                reason: "must be a positive integer".into(),
            })?);
            continue;
        }
        if !known.contains_key(key.as_str()) {
            unknown.push(key);
            continue;
        }
        values.insert(key, value);
    }

    if !unknown.is_empty() {
        return Err(ConfigError::UnknownKeys {
            keys: unknown.join(", "),
            known: KEYS
                .iter()
                .map(|(k, _, _)| *k)
                .collect::<Vec<_>>()
                .join(", "),
        });
    }
    let version = version.ok_or(ConfigError::MissingVersion)?;
    if version > CURRENT_VERSION {
        return Err(ConfigError::TooNew { found: version });
    }
    validate(&values)?;
    Ok(Config { version, values })
}

/// Reject values that would otherwise fail much later, at the point of use.
fn validate(values: &BTreeMap<String, String>) -> Result<(), ConfigError> {
    if let Some(p) = values.get("policy") {
        if !["developer", "dev", "safe", "autonomous", "auto"].contains(&p.as_str()) {
            return Err(ConfigError::BadValue {
                key: "policy".into(),
                value: p.clone(),
                reason: "must be developer, safe, or autonomous".into(),
            });
        }
    }
    for numeric in ["attach_port", "session_ttl_days", "max_download_mb"] {
        if let Some(v) = values.get(numeric) {
            if v.parse::<u64>().is_err() {
                return Err(ConfigError::BadValue {
                    key: numeric.into(),
                    value: v.clone(),
                    reason: "must be a non-negative integer".into(),
                });
            }
        }
    }
    if let Some(f) = values.get("log_format") {
        if !["text", "json"].contains(&f.as_str()) {
            return Err(ConfigError::BadValue {
                key: "log_format".into(),
                value: f.clone(),
                reason: "must be text or json".into(),
            });
        }
    }
    Ok(())
}

/// Where the config lives, in precedence order.
///
/// A project-local file first, so a repository can carry its own settings, then the
/// user-level one.
pub fn candidate_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(explicit) = std::env::var("NEOBROWSER_CONFIG") {
        if !explicit.trim().is_empty() {
            out.push(PathBuf::from(explicit));
            // An explicitly-named file is the only one considered: falling back after
            // a typo'd path would silently use different settings than asked for.
            return out;
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        out.push(cwd.join("neobrowser.toml"));
        out.push(cwd.join(".neobrowser.toml"));
    }
    out.push(crate::paths::home().join("neobrowser.toml"));
    out
}

/// Load the first config file that exists. `Ok(None)` when there is none, which is a
/// perfectly normal state — the file is optional.
pub fn load() -> Result<Option<(PathBuf, Config)>, ConfigError> {
    for path in candidate_paths() {
        if !path.is_file() {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|e| ConfigError::NotFound(format!("{}: {e}", path.display())))?;
        return Ok(Some((path.clone(), parse(&text)?)));
    }
    Ok(None)
}

/// Built-in starting points, matching the PRD's named profiles.
pub fn template(name: &str) -> Option<String> {
    let body = match name {
        "safe" => {
            "# Interactive use with a human present to approve elevated actions.\n\
             policy = \"safe\"\n\
             session_ttl_days = 7\n"
        }
        "developer" => {
            "# Day-to-day development: permissive, but elevated actions are logged.\n\
             policy = \"developer\"\n\
             log_format = \"text\"\n"
        }
        "autonomous" => {
            "# Unattended agent. The allowlist is REQUIRED: with it empty, every call is\n\
             # refused, because an agent with no boundary has no boundary.\n\
             policy = \"autonomous\"\n\
             allow_domains = \"example.com\"\n\
             upload_dir = \"/var/lib/neobrowser/uploads\"\n\
             session_ttl_days = 1\n\
             log_format = \"json\"\n"
        }
        "ci" => {
            "# Hermetic CI: no real sessions, structured logs, tight limits.\n\
             policy = \"autonomous\"\n\
             allow_domains = \"localhost.test\"\n\
             session_ttl_days = 0\n\
             max_download_mb = 16\n\
             log_format = \"json\"\n"
        }
        _ => return None,
    };
    Some(format!("version = {CURRENT_VERSION}\n\n{body}"))
}

/// The public JSON Schema for the config file.
///
/// Generated from [`KEYS`] rather than hand-written, so it cannot describe a key the
/// parser does not accept or miss one it does.
pub fn json_schema() -> Value {
    let mut properties = serde_json::Map::new();
    properties.insert(
        "version".into(),
        json!({
            "type": "integer",
            "minimum": 1,
            "maximum": CURRENT_VERSION,
            "description": "Config schema version. Required.",
        }),
    );
    for (key, env_var, help) in KEYS {
        properties.insert(
            (*key).into(),
            json!({
                "type": "string",
                "description": format!("{help} (env override: {env_var})"),
            }),
        );
    }
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "NeoBrowser configuration",
        "type": "object",
        "required": ["version"],
        "additionalProperties": false,
        "properties": Value::Object(properties),
    })
}

/// Write `path` only if it does not exist, so `init` cannot destroy a real config.
pub fn write_template(path: &Path, name: &str) -> Result<(), ConfigError> {
    let body = template(name).ok_or_else(|| ConfigError::BadValue {
        key: "template".into(),
        value: name.into(),
        reason: "must be safe, developer, autonomous, or ci".into(),
    })?;
    if path.exists() {
        return Err(ConfigError::BadValue {
            key: "path".into(),
            value: path.display().to_string(),
            reason: "already exists; refusing to overwrite an existing config".into(),
        });
    }
    std::fs::write(path, body).map_err(|e| ConfigError::NotFound(e.to_string()))
}

#[cfg(test)]
mod tests {
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
