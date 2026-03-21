//! Entry builders for pipeline phase observability (R2.6).

use neo_types::TraceEntry;

/// Build a [`TraceEntry`] for a pipeline phase start.
pub fn phase_start_entry(ts: u64, phase: &str, trace_id: &str) -> TraceEntry {
    TraceEntry {
        timestamp_ms: ts,
        action: format!("phase_start:{phase}"),
        target: None,
        state_before: None,
        state_after: None,
        duration_ms: 0,
        network_requests: 0,
        dom_mutations: 0,
        error: None,
        metadata: serde_json::json!({ "trace_id": trace_id }),
    }
}

/// Build a [`TraceEntry`] for a pipeline phase end.
pub fn phase_end_entry(
    ts: u64,
    phase: &str,
    trace_id: &str,
    duration_ms: u64,
    decisions: &[String],
    severity: &str,
) -> TraceEntry {
    TraceEntry {
        timestamp_ms: ts,
        action: format!("phase_end:{phase}"),
        target: None,
        state_before: None,
        state_after: None,
        duration_ms,
        network_requests: 0,
        dom_mutations: 0,
        error: None,
        metadata: serde_json::json!({
            "trace_id": trace_id,
            "decisions": decisions,
            "severity": severity,
        }),
    }
}

/// Build a [`TraceEntry`] for a module-level event.
pub fn module_event_entry(ts: u64, module_url: &str, event: &str, trace_id: &str) -> TraceEntry {
    TraceEntry {
        timestamp_ms: ts,
        action: format!("module:{event}"),
        target: Some(module_url.to_string()),
        state_before: None,
        state_after: None,
        duration_ms: 0,
        network_requests: 0,
        dom_mutations: 0,
        error: None,
        metadata: serde_json::json!({ "trace_id": trace_id }),
    }
}

/// Build a [`TraceEntry`] for a failure snapshot.
pub fn failure_snapshot_entry(
    ts: u64,
    phase: &str,
    trace_id: &str,
    partial_state: &str,
) -> TraceEntry {
    TraceEntry {
        timestamp_ms: ts,
        action: format!("failure_snapshot:{phase}"),
        target: None,
        state_before: None,
        state_after: None,
        duration_ms: 0,
        network_requests: 0,
        dom_mutations: 0,
        error: Some(format!("snapshot during {phase}")),
        metadata: serde_json::json!({
            "trace_id": trace_id,
            "partial_state": partial_state,
        }),
    }
}
