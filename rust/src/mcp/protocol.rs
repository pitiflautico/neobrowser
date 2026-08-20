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
            .filter_map(file_uri_to_path)
            // A root that did not resolve to an absolute path is not a root. Keeping a
            // relative one would scope uploads against the process's working directory,
            // which is not what the client declared.
            .filter(|p| p.is_absolute())
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

/// Turn a `file://` URI into a filesystem path, or None if it is not one.
///
/// `strip_prefix("file://")` alone is wrong on Windows: `file:///C:/Users/x` becomes
/// `/C:/Users/x`, which `is_absolute()` rejects, so a client's declared roots were
/// silently dropped there while working fine on Unix. On Unix that leading slash IS
/// the root and must be kept.
///
/// Percent-escapes are decoded because a directory with a space arrives as `%20`, and a
/// root that does not match the real directory name scopes uploads to nothing.
fn file_uri_to_path(uri: &str) -> Option<std::path::PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // file://localhost/x is the same local path as file:///x.
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    let decoded = percent_decode(rest);
    #[cfg(windows)]
    let decoded = {
        let b = decoded.as_bytes();
        // "/C:/x" -> "C:/x", and only when a drive letter actually follows.
        if b.len() >= 3 && b[0] == b'/' && b[2] == b':' {
            decoded[1..].to_string()
        } else {
            decoded
        }
    };
    Some(std::path::PathBuf::from(decoded))
}

/// Decode `%XX` escapes; anything malformed is left as written rather than dropped.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

#[cfg(test)]
mod uri_tests {
    use super::*;

    /// A `file://` root has to survive the trip on every platform. The naive
    /// `strip_prefix("file://")` turns `file:///C:/x` into `/C:/x`, which Windows does
    /// not consider absolute — so the client's declared roots were dropped there and
    /// upload silently fell back to the default directory set.
    #[test]
    fn file_uris_become_absolute_paths_on_this_platform() {
        let uri = if cfg!(windows) {
            "file:///C:/Users/dani"
        } else {
            "file:///tmp/work"
        };
        let p = file_uri_to_path(uri).expect("a file:// URI must parse");
        assert!(p.is_absolute(), "{uri} produced a relative path: {p:?}");
    }

    #[test]
    fn non_file_uris_are_not_paths() {
        assert!(file_uri_to_path("https://example.com/").is_none());
        assert!(file_uri_to_path("ftp://host/x").is_none());
    }

    /// A directory with a space arrives percent-encoded; leaving it encoded produces a
    /// root that matches no real directory, which scopes uploads to nothing.
    #[test]
    fn percent_escapes_are_decoded() {
        assert_eq!(percent_decode("/a%20b/c"), "/a b/c");
        // Malformed escapes are left alone rather than swallowed.
        assert_eq!(percent_decode("/a%zz/b"), "/a%zz/b");
        assert_eq!(percent_decode("/plain/path"), "/plain/path");
    }
}
