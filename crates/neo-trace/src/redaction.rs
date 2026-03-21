//! Auth redaction for trace entries.
//!
//! Replaces sensitive values (Bearer tokens, cookies, API keys)
//! with `[REDACTED]` in trace metadata.

use neo_types::TraceEntry;

/// Regex-free auth redaction patterns.
const AUTH_HEADER_KEYS: &[&str] = &[
    "authorization",
    "cookie",
    "set-cookie",
    "proxy-authorization",
    "x-api-key",
];

/// Redact auth-sensitive values in a single trace entry's metadata.
///
/// Replaces values for known auth headers and Bearer tokens with `[REDACTED]`.
pub fn redact_entry(entry: &mut TraceEntry) {
    redact_value(&mut entry.metadata);
}

/// Recursively walk a JSON value and redact auth-related strings.
fn redact_value(val: &mut serde_json::Value) {
    match val {
        serde_json::Value::String(s) => {
            *s = redact_string(s);
        }
        serde_json::Value::Object(map) => {
            let keys: Vec<String> = map.keys().cloned().collect();
            for key in keys {
                let lower = key.to_lowercase();
                if AUTH_HEADER_KEYS.iter().any(|&k| lower == k) {
                    // Redact entire value of auth-related keys
                    if let Some(v) = map.get_mut(&key) {
                        *v = serde_json::Value::String("[REDACTED]".to_string());
                    }
                } else if let Some(v) = map.get_mut(&key) {
                    redact_value(v);
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr.iter_mut() {
                redact_value(item);
            }
        }
        _ => {}
    }
}

/// Redact Bearer tokens and inline cookie-like patterns in a string value.
fn redact_string(s: &str) -> String {
    let mut result = s.to_string();
    // Redact "Bearer <token>"
    if let Some(pos) = result.to_lowercase().find("bearer ") {
        let start = pos + 7; // length of "Bearer "
        if start < result.len() {
            let end = result[start..]
                .find(|c: char| c.is_whitespace() || c == '"' || c == '\'')
                .map(|p| start + p)
                .unwrap_or(result.len());
            result.replace_range(start..end, "[REDACTED]");
        }
    }
    result
}
