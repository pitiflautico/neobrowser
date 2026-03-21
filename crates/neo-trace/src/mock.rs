//! MockTracer — records all calls for assertion in tests.

use crate::summary::build_summary;
use crate::{ExecutionSummary, NavEvent, NetworkEvent, Tracer};
use neo_types::{PageState, TraceEntry};
use std::sync::Mutex;

/// A recorded intent call.
#[derive(Debug, Clone)]
pub struct Intent {
    pub action_id: String,
    pub intent: String,
    pub target: String,
    pub confidence: f32,
}

/// A recorded action result call.
#[derive(Debug, Clone)]
pub struct ActionResult {
    pub action_id: String,
    pub success: bool,
    pub effect: String,
    pub error: Option<String>,
}

/// A recorded network call.
#[derive(Debug, Clone)]
pub struct NetworkCall {
    pub request_id: String,
    pub url: String,
    pub method: String,
    pub status: u16,
    pub duration_ms: u64,
    pub action_id: Option<String>,
    pub kind: String,
}

/// Mock tracer that records all calls for test assertions.
#[derive(Debug, Default)]
pub struct MockTracer {
    intents: Mutex<Vec<Intent>>,
    actions: Mutex<Vec<ActionResult>>,
    networks: Mutex<Vec<NetworkCall>>,
    entries: Mutex<Vec<TraceEntry>>,
}

impl MockTracer {
    /// Create a new mock tracer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get all recorded intents.
    pub fn intents(&self) -> Vec<Intent> {
        self.intents.lock().map(|i| i.clone()).unwrap_or_default()
    }

    /// Get all recorded action results.
    pub fn actions(&self) -> Vec<ActionResult> {
        self.actions.lock().map(|a| a.clone()).unwrap_or_default()
    }

    /// Get all recorded network calls.
    pub fn networks(&self) -> Vec<NetworkCall> {
        self.networks.lock().map(|n| n.clone()).unwrap_or_default()
    }
}

impl Tracer for MockTracer {
    fn intent(&self, action_id: &str, intent: &str, target: &str, confidence: f32) {
        if let Ok(mut v) = self.intents.lock() {
            v.push(Intent {
                action_id: action_id.to_string(),
                intent: intent.to_string(),
                target: target.to_string(),
                confidence,
            });
        }
        if let Ok(mut v) = self.entries.lock() {
            v.push(crate::tracer::intent_entry(
                0, action_id, intent, target, confidence,
            ));
        }
    }

    fn action_result(&self, action_id: &str, success: bool, effect: &str, error: Option<&str>) {
        if let Ok(mut v) = self.actions.lock() {
            v.push(ActionResult {
                action_id: action_id.to_string(),
                success,
                effect: effect.to_string(),
                error: error.map(String::from),
            });
        }
        if let Ok(mut v) = self.entries.lock() {
            v.push(crate::tracer::action_entry(
                0, action_id, success, effect, error,
            ));
        }
    }

    fn network(&self, event: &NetworkEvent<'_>) {
        if let Ok(mut v) = self.networks.lock() {
            v.push(NetworkCall {
                request_id: event.request_id.to_string(),
                url: event.url.to_string(),
                method: event.method.to_string(),
                status: event.status,
                duration_ms: event.duration_ms,
                action_id: event.action_id.map(String::from),
                kind: event.kind.to_string(),
            });
        }
        if let Ok(mut v) = self.entries.lock() {
            v.push(crate::tracer::network_entry(0, event));
        }
    }

    fn navigation(&self, _event: NavEvent, _url: &str, _nav_id: &str, _status: Option<u16>) {}

    fn state_change(&self, from: PageState, to: PageState, reason: &str) {
        if let Ok(mut v) = self.entries.lock() {
            v.push(crate::tracer::state_change_entry(0, from, to, reason));
        }
    }

    fn dom_diff(&self, added: usize, removed: usize, changed: usize, summary: &str) {
        if let Ok(mut v) = self.entries.lock() {
            v.push(crate::tracer::dom_diff_entry(
                0, added, removed, changed, summary,
            ));
        }
    }

    fn console(&self, level: &str, message: &str) {
        if let Ok(mut v) = self.entries.lock() {
            v.push(crate::tracer::console_entry(0, level, message));
        }
    }

    fn js_exception(&self, error: &str, stack: Option<&str>) {
        if let Ok(mut v) = self.entries.lock() {
            v.push(crate::tracer::js_exception_entry(0, error, stack));
        }
    }

    fn resource_blocked(&self, url: &str, reason: &str) {
        if let Ok(mut v) = self.entries.lock() {
            v.push(crate::tracer::resource_blocked_entry(0, url, reason));
        }
    }

    fn export(&self) -> Vec<TraceEntry> {
        self.entries.lock().map(|e| e.clone()).unwrap_or_default()
    }

    fn summary(&self) -> ExecutionSummary {
        let entries = self.export();
        build_summary(&entries)
    }
}
