//! The table of every setting, and the JSON Schema generated from it.
//!
//! `KEYS` is the single source of truth: the parser validates against it, the schema is
//! generated from it, and the documentation is generated from it. Adding a setting in one
//! place and forgetting the others is the usual way configuration drifts from its docs, and
//! this makes that impossible rather than merely discouraged.

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

use super::CURRENT_VERSION;
use serde_json::{json, Value};

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
