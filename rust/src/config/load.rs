//! Finding the config file, and the templates `config init` writes.

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

use std::path::{Path, PathBuf};

use super::parse::parse;
use super::{Config, ConfigError, CURRENT_VERSION};

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
