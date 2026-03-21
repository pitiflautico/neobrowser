//! FileTracer — writes trace entries to an in-memory store and exports to JSON file.

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
}

impl FileTracer {
    /// Create a new file tracer with an optional output path.
    pub fn new(path: Option<PathBuf>) -> Self {
        Self {
            store: TraceStore::new(),
            path,
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
        let entries = self.store.snapshot();
        let json = serde_json::to_string_pretty(&entries)?;
        std::fs::write(path, json)?;
        Ok(())
    }
}

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
        self.store.snapshot()
    }

    fn summary(&self) -> ExecutionSummary {
        let entries = self.store.snapshot();
        build_summary(&entries)
    }
}
