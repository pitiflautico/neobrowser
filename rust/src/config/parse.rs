//! Reading a config file, and refusing one it does not understand.
//!
//! An unknown key is an error, not a warning. Silently ignoring it means a user who typos a
//! security setting gets the default and a success message — the exact case where failing
//! loudly is worth the inconvenience.

use std::collections::BTreeMap;

use super::keys::KEYS;
use super::{Config, ConfigError, CURRENT_VERSION};

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
pub(super) fn validate(values: &BTreeMap<String, String>) -> Result<(), ConfigError> {
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
