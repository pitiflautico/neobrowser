//! NoopTracer — zero-cost tracer that discards all events.
//!
//! Used when tracing is disabled or in tests that don't need trace data.

use crate::{ExecutionSummary, NavEvent, NetworkEvent, Severity, Tracer};
use neo_types::{PageState, TraceEntry};

/// A tracer that does nothing. Zero allocations, zero cost.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopTracer;

impl NoopTracer {
    /// Create a new no-op tracer.
    pub fn new() -> Self {
        Self
    }
}

impl Tracer for NoopTracer {
    fn intent(&self, _action_id: &str, _intent: &str, _target: &str, _confidence: f32) {}

    fn action_result(&self, _action_id: &str, _success: bool, _effect: &str, _error: Option<&str>) {
    }

    fn network(&self, _event: &NetworkEvent<'_>) {}

    fn navigation(&self, _event: NavEvent, _url: &str, _nav_id: &str, _status: Option<u16>) {}

    fn state_change(&self, _from: PageState, _to: PageState, _reason: &str) {}

    fn dom_diff(&self, _added: usize, _removed: usize, _changed: usize, _summary: &str) {}

    fn console(&self, _level: &str, _message: &str) {}

    fn js_exception(&self, _error: &str, _stack: Option<&str>) {}

    fn resource_blocked(&self, _url: &str, _reason: &str) {}

    fn phase_start(&self, _phase: &str, _trace_id: &str) {}

    fn phase_end(
        &self,
        _phase: &str,
        _trace_id: &str,
        _duration_ms: u64,
        _decisions: &[String],
        _severity: Severity,
    ) {
    }

    fn module_event(&self, _module_url: &str, _event: &str, _trace_id: &str) {}

    fn failure_snapshot(&self, _phase: &str, _trace_id: &str, _partial_state: &str) {}

    fn export(&self) -> Vec<TraceEntry> {
        Vec::new()
    }

    fn summary(&self) -> ExecutionSummary {
        ExecutionSummary {
            total_actions: 0,
            succeeded: 0,
            failed: 0,
            total_requests: 0,
            blocked_requests: 0,
            dom_changes: 0,
            js_errors: 0,
            duration_ms: 0,
            warnings: Vec::new(),
            state: PageState::Idle,
        }
    }
}
