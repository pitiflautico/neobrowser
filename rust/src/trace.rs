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

use std::collections::VecDeque;
use std::sync::Mutex;

use serde_json::{json, Value};

/// How many events one trace keeps.
///
/// Bounded because a long-running agent would otherwise grow this without limit; the
/// oldest events are dropped and the drop is *reported*, so a truncated trace never
/// looks complete.
const MAX_EVENTS: usize = 2000;

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

/// One recorded event.
#[derive(Debug, Clone)]
pub struct Event {
    pub seq: u64,
    pub kind: String,
    pub action_id: Option<String>,
    pub tab_id: Option<String>,
    pub data: Value,
}

impl Event {
    fn to_json(&self, trace_id: &str) -> Value {
        let mut v = json!({
            "seq": self.seq,
            "trace_id": trace_id,
            "kind": self.kind,
            "data": self.data,
        });
        if let Some(a) = &self.action_id {
            v["action_id"] = json!(a);
        }
        if let Some(t) = &self.tab_id {
            v["tab_id"] = json!(t);
        }
        v
    }
}

/// A single session's trace.
pub struct Trace {
    trace_id: String,
    inner: Mutex<Inner>,
}

struct Inner {
    events: VecDeque<Event>,
    next_seq: u64,
    dropped: u64,
}

impl Trace {
    pub fn new(trace_id: impl Into<String>) -> Self {
        Self {
            trace_id: trace_id.into(),
            inner: Mutex::new(Inner {
                events: VecDeque::new(),
                next_seq: 1,
                dropped: 0,
            }),
        }
    }

    pub fn trace_id(&self) -> &str {
        &self.trace_id
    }

    /// Record an event. `data` is redacted here, at the boundary, so no caller can
    /// forget to do it.
    pub fn record(&self, kind: &str, action_id: Option<&str>, tab_id: Option<&str>, data: Value) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let seq = inner.next_seq;
        inner.next_seq += 1;
        inner.events.push_back(Event {
            seq,
            kind: kind.to_string(),
            action_id: action_id.map(str::to_string),
            tab_id: tab_id.map(str::to_string),
            data: redact_value(&data),
        });
        while inner.events.len() > MAX_EVENTS {
            inner.events.pop_front();
            inner.dropped += 1;
        }
    }

    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .events
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Build the shareable evidence bundle.
    ///
    /// `truncated` is stated explicitly rather than left implicit in the sequence
    /// numbers: a bundle that quietly lost its first thousand events would be read as
    /// the whole story.
    pub fn bundle(&self) -> Value {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let events: Vec<Value> = inner
            .events
            .iter()
            .map(|e| e.to_json(&self.trace_id))
            .collect();
        let kinds = {
            let mut counts: std::collections::BTreeMap<&str, usize> = Default::default();
            for e in &inner.events {
                *counts.entry(e.kind.as_str()).or_insert(0) += 1;
            }
            counts
        };
        json!({
            "trace_id": self.trace_id,
            "events": events,
            "event_count": inner.events.len(),
            "dropped_events": inner.dropped,
            "truncated": inner.dropped > 0,
            "summary": kinds.iter().map(|(k, v)| json!({ "kind": k, "count": v })).collect::<Vec<_>>(),
            "redaction": "applied: cookies, auth headers, bearer tokens and sensitive query/form parameters are removed",
        })
    }

    /// Write the bundle under `~/.neobrowser/traces/<trace_id>.json`.
    pub fn write_bundle(&self) -> std::io::Result<std::path::PathBuf> {
        let dir = crate::paths::home().join("traces");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}.json", self.trace_id));
        let body = serde_json::to_string_pretty(&self.bundle()).unwrap_or_else(|_| "{}".into());
        // 0600 even though it is redacted: redaction is a best effort over unbounded
        // page content, so the file should not be world-readable on top of that.
        crate::sessions::write_private(&path, &body)?;
        Ok(path)
    }
}

/// Read a previously written bundle, for `neobrowser trace open <id>`.
pub fn read_bundle(trace_id: &str) -> std::io::Result<Value> {
    // The id becomes a filename, so it is validated rather than trusted: an id of
    // `../../.ssh/id_rsa` must not turn a debug command into a file read.
    if !trace_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "trace id must be alphanumeric with _ or -",
        ));
    }
    let path = crate::paths::home()
        .join("traces")
        .join(format!("{trace_id}.json"));
    let text = std::fs::read_to_string(path)?;
    serde_json::from_str(&text)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
}

/// List available traces, newest first.
pub fn list_bundles() -> Vec<String> {
    let dir = crate::paths::home().join("traces");
    let mut out: Vec<(std::time::SystemTime, String)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let id = name.strip_suffix(".json")?.to_string();
            let modified = e.metadata().ok()?.modified().ok()?;
            Some((modified, id))
        })
        .collect();
    // Newest first, so `trace list` shows the run you just finished at the top.
    out.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    out.into_iter().map(|(_, id)| id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- redaction: the tests ARE the specification ---------------------------

    #[test]
    fn bearer_tokens_are_redacted() {
        assert_eq!(
            redact("Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.abc.def"),
            "Authorization: [redacted]"
        );
        // Inside JSON-ish text too.
        assert!(!redact(r#"{"h":"Bearer sk-live-1234567890"}"#).contains("sk-live"));
    }

    #[test]
    fn sensitive_query_parameters_are_redacted() {
        let out = redact("https://api.test/cb?code=abc123&state=xyz&access_token=secret");
        assert!(out.contains("code=[redacted]"), "{out}");
        assert!(out.contains("access_token=[redacted]"), "{out}");
        // Non-sensitive parameters survive, or the trace stops being useful.
        assert!(out.contains("state=xyz"), "{out}");
    }

    /// Regression: the separators must survive verbatim. An earlier version rebuilt
    /// them by indexing the input with the output's length, so `&` came back as
    /// whatever byte sat at that offset and every traced URL was corrupted.
    #[test]
    fn redaction_preserves_url_structure() {
        let out = redact("https://example.com/?access_token=SECRET&code=AUTH&state=ok");
        assert_eq!(
            out,
            "https://example.com/?access_token=[redacted]&code=[redacted]&state=ok"
        );
        // Semicolon-separated form bodies too.
        assert_eq!(redact("a=1;password=x;b=2"), "a=1;password=[redacted];b=2");
    }

    #[test]
    fn sensitive_headers_are_redacted_by_name() {
        for header in ["Cookie", "cookie", "Set-Cookie", "X-Api-Key"] {
            let out = redact(&format!("{header}: super-secret-value"));
            assert!(
                !out.contains("super-secret-value"),
                "{header} leaked: {out}"
            );
        }
        // An ordinary header is left alone.
        assert_eq!(redact("Content-Type: text/html"), "Content-Type: text/html");
    }

    /// A key called `cookie` has no `key=value` shape for the string pass to catch,
    /// so removal has to be driven by the key name as well.
    #[test]
    fn json_keys_that_name_a_secret_are_removed() {
        let v = json!({
            "cookie": "SID=abc",
            "nested": { "password": "hunter2", "safe": "keep me" },
            "list": [{ "api_key": "k-123" }],
        });
        let r = redact_value(&v);
        assert_eq!(r["cookie"], "[redacted]");
        assert_eq!(r["nested"]["password"], "[redacted]");
        assert_eq!(r["nested"]["safe"], "keep me");
        assert_eq!(r["list"][0]["api_key"], "[redacted]");
        // And nothing of the secret survives anywhere in the serialised form.
        let text = r.to_string();
        for secret in ["SID=abc", "hunter2", "k-123"] {
            assert!(!text.contains(secret), "{secret} survived: {text}");
        }
    }

    #[test]
    fn redaction_leaves_ordinary_text_intact() {
        let plain = "navigated to https://example.com/pricing";
        assert_eq!(redact(plain), plain);
    }

    // --- trace mechanics -------------------------------------------------------

    #[test]
    fn events_are_sequenced_and_correlated() {
        let t = Trace::new("trace_test");
        t.record("action", Some("act_1"), Some("tab_0"), json!({ "x": 1 }));
        t.record("policy", None, None, json!({ "decision": "allow" }));
        let b = t.bundle();
        assert_eq!(b["event_count"], 2);
        assert_eq!(b["events"][0]["seq"], 1);
        assert_eq!(b["events"][0]["trace_id"], "trace_test");
        assert_eq!(b["events"][0]["action_id"], "act_1");
        assert_eq!(b["events"][0]["tab_id"], "tab_0");
        // An event with no action/tab must not invent empty ids.
        assert!(b["events"][1].get("action_id").is_none());
        assert_eq!(b["events"][1]["seq"], 2);
    }

    /// Recording goes through redaction at the boundary, so a caller cannot forget.
    #[test]
    fn recorded_data_is_redacted_on_the_way_in() {
        let t = Trace::new("trace_redact");
        t.record("request", None, None, json!({ "cookie": "SID=leak" }));
        let text = t.bundle().to_string();
        assert!(!text.contains("SID=leak"), "{text}");
    }

    #[test]
    fn the_event_buffer_is_bounded_and_reports_truncation() {
        let t = Trace::new("trace_bound");
        for i in 0..(MAX_EVENTS + 50) {
            t.record("tick", None, None, json!({ "i": i }));
        }
        let b = t.bundle();
        assert_eq!(b["event_count"], MAX_EVENTS);
        assert_eq!(b["dropped_events"], 50);
        assert_eq!(b["truncated"], true, "a truncated bundle must say so");
        // The oldest were dropped, so the first surviving seq is not 1.
        assert_eq!(b["events"][0]["seq"], 51);
    }

    #[test]
    fn an_untruncated_bundle_does_not_claim_truncation() {
        let t = Trace::new("trace_small");
        t.record("tick", None, None, json!({}));
        let b = t.bundle();
        assert_eq!(b["truncated"], false);
        assert_eq!(b["dropped_events"], 0);
    }

    #[test]
    fn the_summary_counts_events_by_kind() {
        let t = Trace::new("trace_sum");
        t.record("action", None, None, json!({}));
        t.record("action", None, None, json!({}));
        t.record("wall", None, None, json!({}));
        let b = t.bundle();
        let summary = b["summary"].as_array().unwrap();
        let action = summary.iter().find(|s| s["kind"] == "action").unwrap();
        assert_eq!(action["count"], 2);
    }

    /// A trace id becomes a path component, so traversal must be refused rather than
    /// turning a debug command into an arbitrary file read.
    #[test]
    fn reading_a_bundle_rejects_path_traversal() {
        for bad in ["../../etc/passwd", "..", "a/b", "with space", "x/../y"] {
            let err = read_bundle(bad).unwrap_err();
            assert_eq!(
                err.kind(),
                std::io::ErrorKind::InvalidInput,
                "{bad} should be rejected as an id"
            );
        }
    }

    #[test]
    fn bundles_round_trip_through_disk() {
        let _g = crate::env_test_guard();
        std::env::set_var("NEOBROWSER_HOME", "/tmp/nb-trace-test");
        let t = Trace::new("trace_roundtrip");
        t.record(
            "action",
            Some("act_1"),
            None,
            json!({ "url": "https://example.com/" }),
        );
        let path = t.write_bundle().unwrap();
        assert!(path.exists());

        let read = read_bundle("trace_roundtrip").unwrap();
        assert_eq!(read["trace_id"], "trace_roundtrip");
        assert_eq!(read["event_count"], 1);
        assert!(list_bundles().contains(&"trace_roundtrip".to_string()));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "a bundle must not be world-readable");
        }
        let _ = std::fs::remove_dir_all("/tmp/nb-trace-test");
    }
}
