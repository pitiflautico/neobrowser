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
//!
//! Split into [`redact`] (removing secrets before anything is written), [`record`] (bounded
//! recording) and [`bundle`] (reading a trace back, refusing traversal in its id).

pub mod bundle;
pub mod record;
pub mod redact;

pub use bundle::{list_bundles, read_bundle};
pub use record::{Event, Trace};
pub use redact::{redact, redact_value};

/// How many events one trace keeps.
///
/// Bounded because a long-running agent would otherwise grow this without limit; the
/// oldest events are dropped and the drop is *reported*, so a truncated trace never
/// looks complete.
const MAX_EVENTS: usize = 2000;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
