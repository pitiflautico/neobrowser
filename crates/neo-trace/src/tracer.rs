//! TraceStore — thread-safe storage for trace entries.

use neo_types::{PageState, TraceEntry};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Thread-safe store for trace entries with relative timestamps.
#[derive(Debug, Clone)]
pub struct TraceStore {
    entries: Arc<Mutex<Vec<TraceEntry>>>,
    start: Instant,
}

impl TraceStore {
    /// Create a new empty trace store.
    pub fn new() -> Self {
        Self {
            entries: Arc::new(Mutex::new(Vec::new())),
            start: Instant::now(),
        }
    }

    /// Elapsed milliseconds since session start.
    pub fn elapsed_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    /// Push a new trace entry.
    pub fn push(&self, entry: TraceEntry) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.push(entry);
        }
    }

    /// Return a snapshot of all entries.
    pub fn snapshot(&self) -> Vec<TraceEntry> {
        self.entries.lock().map(|e| e.clone()).unwrap_or_default()
    }

    /// Number of entries stored.
    pub fn len(&self) -> usize {
        self.entries.lock().map(|e| e.len()).unwrap_or(0)
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for TraceStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a [`TraceEntry`] for an intent declaration.
pub fn intent_entry(
    ts: u64,
    action_id: &str,
    intent: &str,
    target: &str,
    confidence: f32,
) -> TraceEntry {
    TraceEntry {
        timestamp_ms: ts,
        action: format!("intent:{action_id}"),
        target: Some(target.to_string()),
        state_before: None,
        state_after: None,
        duration_ms: 0,
        network_requests: 0,
        dom_mutations: 0,
        error: None,
        metadata: serde_json::json!({
            "intent": intent,
            "confidence": confidence,
        }),
    }
}

/// Build a [`TraceEntry`] for an action result.
pub fn action_entry(
    ts: u64,
    action_id: &str,
    success: bool,
    effect: &str,
    error: Option<&str>,
) -> TraceEntry {
    TraceEntry {
        timestamp_ms: ts,
        action: format!("action:{action_id}"),
        target: None,
        state_before: None,
        state_after: None,
        duration_ms: 0,
        network_requests: 0,
        dom_mutations: 0,
        error: error.map(String::from),
        metadata: serde_json::json!({
            "success": success,
            "effect": effect,
        }),
    }
}

/// Build a [`TraceEntry`] for a network event.
pub fn network_entry(ts: u64, event: &crate::NetworkEvent<'_>) -> TraceEntry {
    TraceEntry {
        timestamp_ms: ts,
        action: format!("network:{}", event.request_id),
        target: Some(event.url.to_string()),
        state_before: None,
        state_after: None,
        duration_ms: event.duration_ms,
        network_requests: 1,
        dom_mutations: 0,
        error: if event.status >= 400 {
            Some(format!("HTTP {}", event.status))
        } else {
            None
        },
        metadata: serde_json::json!({
            "method": event.method,
            "status": event.status,
            "action_id": event.action_id,
            "frame_id": event.frame_id,
            "kind": event.kind,
        }),
    }
}

/// Build a [`TraceEntry`] for a navigation event.
pub fn navigation_entry(
    ts: u64,
    event: &str,
    url: &str,
    nav_id: &str,
    status: Option<u16>,
) -> TraceEntry {
    TraceEntry {
        timestamp_ms: ts,
        action: format!("nav:{event}:{nav_id}"),
        target: Some(url.to_string()),
        state_before: None,
        state_after: None,
        duration_ms: 0,
        network_requests: 0,
        dom_mutations: 0,
        error: None,
        metadata: serde_json::json!({ "status": status }),
    }
}

/// Build a [`TraceEntry`] for a state change.
pub fn state_change_entry(ts: u64, from: PageState, to: PageState, reason: &str) -> TraceEntry {
    TraceEntry {
        timestamp_ms: ts,
        action: "state_change".to_string(),
        target: None,
        state_before: Some(from),
        state_after: Some(to),
        duration_ms: 0,
        network_requests: 0,
        dom_mutations: 0,
        error: None,
        metadata: serde_json::json!({ "reason": reason }),
    }
}

/// Build a [`TraceEntry`] for a DOM diff.
pub fn dom_diff_entry(
    ts: u64,
    added: usize,
    removed: usize,
    changed: usize,
    summary: &str,
) -> TraceEntry {
    TraceEntry {
        timestamp_ms: ts,
        action: "dom_diff".to_string(),
        target: None,
        state_before: None,
        state_after: None,
        duration_ms: 0,
        network_requests: 0,
        dom_mutations: added + removed + changed,
        error: None,
        metadata: serde_json::json!({
            "added": added,
            "removed": removed,
            "changed": changed,
            "summary": summary,
        }),
    }
}

/// Build a [`TraceEntry`] for a console message.
pub fn console_entry(ts: u64, level: &str, message: &str) -> TraceEntry {
    TraceEntry {
        timestamp_ms: ts,
        action: format!("console:{level}"),
        target: None,
        state_before: None,
        state_after: None,
        duration_ms: 0,
        network_requests: 0,
        dom_mutations: 0,
        error: if level == "error" {
            Some(message.to_string())
        } else {
            None
        },
        metadata: serde_json::json!({ "message": message }),
    }
}

/// Build a [`TraceEntry`] for a JS exception.
pub fn js_exception_entry(ts: u64, error: &str, stack: Option<&str>) -> TraceEntry {
    TraceEntry {
        timestamp_ms: ts,
        action: "js_exception".to_string(),
        target: None,
        state_before: None,
        state_after: None,
        duration_ms: 0,
        network_requests: 0,
        dom_mutations: 0,
        error: Some(error.to_string()),
        metadata: serde_json::json!({ "stack": stack }),
    }
}

/// Build a [`TraceEntry`] for a blocked resource.
pub fn resource_blocked_entry(ts: u64, url: &str, reason: &str) -> TraceEntry {
    TraceEntry {
        timestamp_ms: ts,
        action: "resource_blocked".to_string(),
        target: Some(url.to_string()),
        state_before: None,
        state_after: None,
        duration_ms: 0,
        network_requests: 0,
        dom_mutations: 0,
        error: None,
        metadata: serde_json::json!({ "reason": reason }),
    }
}
