//! FileTracer — writes trace entries to an in-memory store and exports to JSON file.

use crate::redaction::redact_entry;
use crate::summary::build_summary;
use crate::tracer::{self, TraceStore};
use crate::{ExecutionSummary, NavEvent, NetworkEvent, Tracer};
use neo_types::{PageState, TraceEntry};
use std::path::PathBuf;

/// A tracer that stores events in memory and can export to a JSON file.
#[derive(Debug)]
pub struct FileTracer {
    store: TraceStore,
    path: Option<PathBuf>,
    /// When true, auth tokens/cookies are replaced with [REDACTED] on export.
    redact_auth: bool,
}

impl FileTracer {
    /// Create a new file tracer with an optional output path.
    pub fn new(path: Option<PathBuf>) -> Self {
        Self {
            store: TraceStore::new(),
            path,
            redact_auth: false,
        }
    }

    /// Create a file tracer with auth redaction enabled.
    pub fn with_redaction(path: Option<PathBuf>, redact: bool) -> Self {
        Self {
            store: TraceStore::new(),
            path,
            redact_auth: redact,
        }
    }

    /// Write the current trace to the configured file path.
    ///
    /// Returns an error if no path is set or serialization/IO fails.
    pub fn flush(&self) -> Result<(), crate::TraceError> {
        let path = self.path.as_ref().ok_or_else(|| {
            crate::TraceError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no trace path configured",
            ))
        })?;
        let entries = self.export();
        let json = serde_json::to_string_pretty(&entries)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Apply redaction to a list of entries if configured.
    fn maybe_redact(&self, mut entries: Vec<TraceEntry>) -> Vec<TraceEntry> {
        if self.redact_auth {
            for entry in entries.iter_mut() {
                redact_entry(entry);
            }
        }
        entries
    }
}

/// Convert a [`NavEvent`] to its string representation.
fn nav_event_str(event: NavEvent) -> &'static str {
    match event {
        NavEvent::Started => "started",
        NavEvent::Committed => "committed",
        NavEvent::Finished => "finished",
        NavEvent::Failed => "failed",
    }
}

impl Tracer for FileTracer {
    fn intent(&self, action_id: &str, intent: &str, target: &str, confidence: f32) {
        let ts = self.store.elapsed_ms();
        self.store.push(tracer::intent_entry(
            ts, action_id, intent, target, confidence,
        ));
    }

    fn action_result(&self, action_id: &str, success: bool, effect: &str, error: Option<&str>) {
        let ts = self.store.elapsed_ms();
        self.store
            .push(tracer::action_entry(ts, action_id, success, effect, error));
    }

    fn network(&self, event: &NetworkEvent<'_>) {
        let ts = self.store.elapsed_ms();
        self.store.push(tracer::network_entry(ts, event));
    }

    fn navigation(&self, event: NavEvent, url: &str, nav_id: &str, status: Option<u16>) {
        let ts = self.store.elapsed_ms();
        self.store.push(tracer::navigation_entry(
            ts,
            nav_event_str(event),
            url,
            nav_id,
            status,
        ));
    }

    fn state_change(&self, from: PageState, to: PageState, reason: &str) {
        let ts = self.store.elapsed_ms();
        self.store
            .push(tracer::state_change_entry(ts, from, to, reason));
    }

    fn dom_diff(&self, added: usize, removed: usize, changed: usize, summary: &str) {
        let ts = self.store.elapsed_ms();
        self.store
            .push(tracer::dom_diff_entry(ts, added, removed, changed, summary));
    }

    fn console(&self, level: &str, message: &str) {
        let ts = self.store.elapsed_ms();
        self.store.push(tracer::console_entry(ts, level, message));
    }

    fn js_exception(&self, error: &str, stack: Option<&str>) {
        let ts = self.store.elapsed_ms();
        self.store
            .push(tracer::js_exception_entry(ts, error, stack));
    }

    fn resource_blocked(&self, url: &str, reason: &str) {
        let ts = self.store.elapsed_ms();
        self.store
            .push(tracer::resource_blocked_entry(ts, url, reason));
    }

    fn export(&self) -> Vec<TraceEntry> {
        let entries = self.store.snapshot();
        self.maybe_redact(entries)
    }

    fn summary(&self) -> ExecutionSummary {
        let entries = self.store.snapshot();
        build_summary(&entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redact_bearer_token() {
        let tracer = FileTracer::with_redaction(None, true);
        {
            let entries = vec![TraceEntry {
                timestamp_ms: 0,
                action: "network:r1".to_string(),
                target: Some("https://api.example.com/data".to_string()),
                state_before: None,
                state_after: None,
                duration_ms: 50,
                network_requests: 1,
                dom_mutations: 0,
                error: None,
                metadata: serde_json::json!({
                    "method": "GET",
                    "status": 200,
                    "authorization": "Bearer eyJhbGciOiJIUzI1NiJ9.secret",
                    "cookie": "session=abc123; token=xyz",
                }),
            }];
            let redacted = tracer.maybe_redact(entries);
            assert_eq!(
                redacted[0].metadata["authorization"],
                serde_json::Value::String("[REDACTED]".to_string()),
            );
            assert_eq!(
                redacted[0].metadata["cookie"],
                serde_json::Value::String("[REDACTED]".to_string()),
            );
            assert_eq!(redacted[0].metadata["method"], "GET");
            assert_eq!(redacted[0].metadata["status"], 200);
        }
    }

    #[test]
    fn test_redact_bearer_in_string_value() {
        let _tracer = FileTracer::with_redaction(None, true);
        let mut entry = TraceEntry {
            timestamp_ms: 0,
            action: "network:r1".to_string(),
            target: None,
            state_before: None,
            state_after: None,
            duration_ms: 0,
            network_requests: 1,
            dom_mutations: 0,
            error: None,
            metadata: serde_json::json!({
                "headers": {
                    "Authorization": "Bearer secret-token-123",
                    "Content-Type": "application/json",
                }
            }),
        };
        redact_entry(&mut entry);
        let auth = entry.metadata["headers"]["Authorization"].as_str().unwrap();
        assert!(
            auth.contains("[REDACTED]"),
            "Bearer token not redacted: {auth}"
        );
        assert!(!auth.contains("secret-token-123"));
        assert_eq!(
            entry.metadata["headers"]["Content-Type"],
            "application/json"
        );
    }

    #[test]
    fn test_no_redaction_when_disabled() {
        let tracer = FileTracer::new(None);
        let entries = vec![TraceEntry {
            timestamp_ms: 0,
            action: "network:r1".to_string(),
            target: None,
            state_before: None,
            state_after: None,
            duration_ms: 0,
            network_requests: 1,
            dom_mutations: 0,
            error: None,
            metadata: serde_json::json!({
                "authorization": "Bearer keep-this-token",
            }),
        }];
        let result = tracer.maybe_redact(entries);
        assert_eq!(
            result[0].metadata["authorization"],
            "Bearer keep-this-token",
        );
    }
}
