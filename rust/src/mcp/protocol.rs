//! Protocol-version negotiation.
//!
//! Clients ask for a version; this answers with one both sides support, preferring the
//! newest. Getting this wrong is not a graceful degradation — a client told a version it does
//! not implement will misparse every response after `initialize`, so an unrecognised request
//! falls back to the oldest supported version rather than echoing whatever was asked for.

//! MCP protocol (JSON-RPC 2.0 over stdin/stdout).
//!
//! Port of the protocol half of the Python `server.py`: `initialize`, `tools/list`,
//! `tools/call`, and `notifications/initialized`, with the same argument-validation
//! contract and the same 500k-char text cap. Screenshots return native MCP image
//! content instead of the Python string-JSON round-trip.

use serde_json::Value;

use super::PROTOCOL_VERSION;

/// MCP protocol versions this server can speak, newest first.
pub(super) const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// Pick the protocol version to answer with, and record any declared roots.
///
/// The rule from the MCP spec: echo the client's version when we support it, otherwise
/// answer with our preferred one and let the client decide whether it can proceed.
pub(super) fn negotiate_protocol_version(params: &Value) -> String {
    // Roots arrive in the same handshake, so this is the one place they can be captured
    // before any tool runs.
    if let Some(roots) = params
        .get("capabilities")
        .and_then(|c| c.get("roots"))
        .and_then(|r| r.get("roots"))
        .and_then(Value::as_array)
    {
        let paths: Vec<std::path::PathBuf> = roots
            .iter()
            .filter_map(|r| r.get("uri").and_then(Value::as_str))
            // Only file:// roots mean anything for filesystem access; an http root is
            // not a directory and must not be treated as one.
            .filter_map(|uri| uri.strip_prefix("file://"))
            .map(std::path::PathBuf::from)
            .collect();
        if !paths.is_empty() {
            tracing::info!(roots = ?paths, "client declared MCP roots; upload is scoped to them");
            crate::reach::set_mcp_roots(paths);
        }
    }

    let requested = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or("");
    if SUPPORTED_PROTOCOL_VERSIONS.contains(&requested) {
        return requested.to_string();
    }
    if !requested.is_empty() {
        tracing::info!(
            requested,
            offering = PROTOCOL_VERSION,
            "client asked for an unsupported MCP protocol version; offering ours"
        );
    }
    PROTOCOL_VERSION.to_string()
}
