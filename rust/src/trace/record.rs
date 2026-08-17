//! Recording what happened, bounded.
//!
//! Capped at a fixed number of events on purpose: an unbounded trace on a long-running agent
//! session is a memory leak that only shows up in the runs that matter most.

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

use super::redact::redact_value;
use super::MAX_EVENTS;
use serde_json::{json, Value};

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
