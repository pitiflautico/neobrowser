//! Matching a host against a configured domain set.
//!
//! Label-aware, and that is the whole point: `host.ends_with("google.com")` also matches
//! `notgoogle.com`, a domain anyone can register. So a host matches only if it equals the
//! configured domain or ends with a dot followed by it. A matcher that fails open here
//! silently grants access to whatever an attacker chose to name their domain.

//! Central policy engine: every tool call is classified and evaluated before it runs.
//!
//! Before this existed, security lived in scattered point-checks — an SSRF guard in
//! `reach`, an https requirement in `login`, an upload-root check in `reach`. Each is
//! correct, but there was no single place that could answer "is this call allowed?",
//! so every new tool had to remember to re-implement the relevant guard.
//!
//! The engine sits in the MCP dispatch path (see `mcp::handle_tools_call`) between
//! argument validation and execution. It does not replace the point-checks — defence
//! in depth — it decides whether the call happens at all.
//!
//! Design constraints worth stating, because they shaped the defaults:
//!
//! - **A denial must be legible.** A model that gets an opaque failure retries; one
//!   that is told which rule fired and what would satisfy it can adapt or ask.
//! - **`ask` is only useful if someone can answer.** Most MCP clients do not
//!   implement elicitation, so a profile that asks for confirmation on ordinary
//!   actions would simply be a profile that fails. Confirmation is therefore
//!   reserved for the `Safe` profile's elevated classes, and returns a structured
//!   `requires_confirmation` the caller can act on rather than a dead end.
//! - **The default profile does not break existing setups.** `Developer` enforces
//!   the domain lists and logs elevated actions but does not gate them. Users who
//!   want gating opt into `Safe`; unattended agents should use `Autonomous`, which
//!   demands an explicit allowlist.

use std::collections::BTreeSet;

use serde_json::{Map, Value};

/// Extract the destination host from a tool's arguments, if they name one.
///
/// Only a real URL counts. A bare string is not treated as a host: guessing would
/// let a selector like `div.example.com` be read as a destination and either grant
/// or deny access on nonsense.
pub fn target_host(args: &Map<String, Value>) -> Option<String> {
    let raw = args.get("url").and_then(Value::as_str)?;
    reqwest::Url::parse(raw)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
}

/// Comma-separated env var to a set of lowercase host suffixes.
pub(super) fn domain_set(var: &str) -> BTreeSet<String> {
    std::env::var(var)
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Does `host` equal or sit under any suffix in `set`?
///
/// Matching is label-aware: `evil-example.com` must not match `example.com`, which
/// a plain `ends_with` would wrongly accept.
pub(super) fn matches_suffix(set: &BTreeSet<String>, host: &str) -> bool {
    set.iter()
        .any(|s| host == s.as_str() || host.ends_with(&format!(".{s}")))
}
