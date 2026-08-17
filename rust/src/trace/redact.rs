//! Removing secrets before anything is written down.
//!
//! A trace exists to be shared — attached to a bug report, pasted into an issue — which makes
//! it the most likely place for a credential to escape. Redaction walks byte offsets rather
//! than reconstructing the string, because an earlier version indexed the input with the
//! output's length and corrupted the separators it was supposed to preserve, turning `&`
//! into `T` in the middle of a query string.

//! Execution traces: correlated events, secret redaction, and shareable evidence
//! bundles.
//!
//! When an agent run goes wrong, the question is always "what actually happened, in
//! order?" — and the answer was previously spread across the model's transcript, the
//! server's log lines, and nothing else. This module records the timeline itself:
//! every action with its ids, the policy decisions, the walls hit, the URLs and
//! redirects.
//!
//! Two properties do the work.
//!
//! **Correlation.** Every event carries `trace_id`, and where applicable `action_id`
//! and `tab_id`. Without those, interleaved events from two tabs are unreadable.
//!
//! **Redaction by default.** A trace of a browser session naturally contains cookies,
//! `Authorization` headers, tokens in query strings and form values. A bundle exists
//! to be shared — attached to a bug report, pasted into an issue — so redaction is
//! not an option a user has to remember to switch on. [`redact`] runs on every value
//! entering the trace, and the tests are the specification.

use serde_json::Value;

/// Query/form parameter names whose values are secrets.
const SENSITIVE_PARAMS: &[&str] = &[
    "token",
    "access_token",
    "refresh_token",
    "id_token",
    "code",
    "api_key",
    "apikey",
    "key",
    "secret",
    "client_secret",
    "password",
    "passwd",
    "pwd",
    "session",
    "sessionid",
    "sid",
    "auth",
    "authorization",
    "signature",
    "sig",
];

/// Header names never recorded in the clear.
const SENSITIVE_HEADERS: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
    "x-api-key",
    "x-auth-token",
    "x-csrf-token",
];

const REDACTED: &str = "[redacted]";

/// Redact secrets from a string that is about to enter a trace.
///
/// Handles the three shapes that actually occur: `key=value` pairs in URLs and form
/// bodies, `Header: value` lines, and bare high-entropy bearer tokens. It is
/// deliberately aggressive — over-redacting a trace costs a little debuggability,
/// while under-redacting one publishes a live session.
pub fn redact(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;

    // `Bearer <token>` regardless of surrounding syntax.
    while let Some(pos) = rest.to_ascii_lowercase().find("bearer ") {
        let (head, tail) = rest.split_at(pos + "bearer ".len());
        out.push_str(head);
        let end = tail
            .find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ',')
            .unwrap_or(tail.len());
        out.push_str(REDACTED);
        rest = &tail[end..];
    }
    out.push_str(rest);

    // key=value in query strings and form bodies.
    //
    // Walked by byte offset rather than `split`: an earlier version reconstructed the
    // separators by indexing the *input* with the length of the *output*, which stops
    // lining up the moment a redaction changes a segment's length — and quietly
    // replaced `&` with whatever character happened to sit at that offset.
    fn push_segment(result: &mut String, segment: &str) {
        match segment.split_once('=') {
            Some((k, _)) if SENSITIVE_PARAMS.contains(&k.trim().to_ascii_lowercase().as_str()) => {
                result.push_str(k);
                result.push('=');
                result.push_str(REDACTED);
            }
            _ => result.push_str(segment),
        }
    }
    let mut result = String::with_capacity(out.len());
    let mut start = 0;
    for (idx, ch) in out.char_indices() {
        if matches!(ch, '&' | '?' | ';') {
            push_segment(&mut result, &out[start..idx]);
            result.push(ch);
            start = idx + ch.len_utf8();
        }
    }
    push_segment(&mut result, &out[start..]);

    // `Header: value` lines.
    result
        .lines()
        .map(|line| match line.split_once(':') {
            Some((name, _))
                if SENSITIVE_HEADERS.contains(&name.trim().to_ascii_lowercase().as_str()) =>
            {
                format!("{name}: {REDACTED}")
            }
            _ => line.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Recursively redact every string in a JSON value, and drop values under keys whose
/// *name* marks them sensitive.
///
/// Key-based removal matters as much as value scrubbing: `{"cookie": "abc"}` has no
/// `key=value` shape for the string pass to catch.
pub fn redact_value(v: &Value) -> Value {
    match v {
        Value::String(s) => Value::String(redact(s)),
        Value::Array(a) => Value::Array(a.iter().map(redact_value).collect()),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, val)| {
                    let lower = k.to_ascii_lowercase();
                    let sensitive = SENSITIVE_HEADERS.contains(&lower.as_str())
                        || SENSITIVE_PARAMS.contains(&lower.as_str());
                    if sensitive {
                        (k.clone(), Value::String(REDACTED.into()))
                    } else {
                        (k.clone(), redact_value(val))
                    }
                })
                .collect(),
        ),
        other => other.clone(),
    }
}
