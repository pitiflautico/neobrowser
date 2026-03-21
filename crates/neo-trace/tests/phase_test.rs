use neo_trace::mock::MockTracer;
use neo_trace::{Severity, Tracer};

#[test]
fn test_phase_start_end_recorded() {
    let tracer = MockTracer::new();
    tracer.phase_start("fetch", "t1");
    tracer.phase_end(
        "fetch",
        "t1",
        42,
        &["prefetched 3 modules".to_string()],
        Severity::Info,
    );

    let phases = tracer.phases();
    assert_eq!(phases.len(), 2);

    // Start record
    assert_eq!(phases[0].phase, "fetch");
    assert_eq!(phases[0].trace_id, "t1");
    assert!(!phases[0].is_end);
    assert!(phases[0].duration_ms.is_none());

    // End record
    assert_eq!(phases[1].phase, "fetch");
    assert_eq!(phases[1].trace_id, "t1");
    assert!(phases[1].is_end);
    assert_eq!(phases[1].duration_ms, Some(42));
    assert_eq!(phases[1].decisions, vec!["prefetched 3 modules"]);
    assert_eq!(phases[1].severity, Some(Severity::Info));

    // Entries should also be recorded
    let entries = tracer.export();
    assert_eq!(entries.len(), 2);
    assert!(entries[0].action.contains("phase_start:fetch"));
    assert!(entries[1].action.contains("phase_end:fetch"));
    assert_eq!(entries[1].duration_ms, 42);
}

#[test]
fn test_module_event_correlation() {
    let tracer = MockTracer::new();
    let url = "https://cdn.example.com/app.mjs";
    let tid = "t2";

    tracer.module_event(url, "fetch", tid);
    tracer.module_event(url, "stub", tid);
    tracer.module_event(url, "rewrite", tid);
    tracer.module_event(url, "eval", tid);

    let modules = tracer.modules();
    assert_eq!(modules.len(), 4);

    // All events share the same module URL and trace ID
    for m in &modules {
        assert_eq!(m.module_url, url);
        assert_eq!(m.trace_id, tid);
    }

    // Events are in order
    assert_eq!(modules[0].event, "fetch");
    assert_eq!(modules[1].event, "stub");
    assert_eq!(modules[2].event, "rewrite");
    assert_eq!(modules[3].event, "eval");

    // Entries correlate via target (module URL)
    let entries = tracer.export();
    assert_eq!(entries.len(), 4);
    for entry in &entries {
        assert_eq!(entry.target.as_deref(), Some(url));
        assert_eq!(entry.metadata["trace_id"], tid);
    }
}

#[test]
fn test_failure_snapshot_captured() {
    let tracer = MockTracer::new();

    // Simulate a phase that fails and captures a snapshot
    tracer.phase_start("eval", "t3");
    tracer.failure_snapshot(
        "eval",
        "t3",
        "<div id=\"root\"><p>partial render...</p></div>",
    );
    tracer.phase_end(
        "eval",
        "t3",
        100,
        &["eval failed".to_string()],
        Severity::Error,
    );

    let snapshots = tracer.snapshots();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].phase, "eval");
    assert_eq!(snapshots[0].trace_id, "t3");
    assert!(snapshots[0].partial_state.contains("partial render"));

    // Snapshot entry has error field set
    let entries = tracer.export();
    let snap_entry = entries
        .iter()
        .find(|e| e.action.starts_with("failure_snapshot:"))
        .expect("snapshot entry should exist");
    assert!(snap_entry.error.is_some());
    assert!(snap_entry.metadata["partial_state"]
        .as_str()
        .expect("partial_state should be string")
        .contains("partial render"));
}

#[test]
fn test_severity_levels() {
    let tracer = MockTracer::new();

    tracer.phase_start("classify", "t4");
    tracer.phase_end("classify", "t4", 10, &[], Severity::Info);

    tracer.phase_start("stub", "t4");
    tracer.phase_end(
        "stub",
        "t4",
        20,
        &["3 modules stubbed".to_string()],
        Severity::Warn,
    );

    tracer.phase_start("eval", "t4");
    tracer.phase_end(
        "eval",
        "t4",
        50,
        &["ReferenceError".to_string()],
        Severity::Error,
    );

    let phases = tracer.phases();
    let ends: Vec<_> = phases.iter().filter(|p| p.is_end).collect();
    assert_eq!(ends.len(), 3);
    assert_eq!(ends[0].severity, Some(Severity::Info));
    assert_eq!(ends[1].severity, Some(Severity::Warn));
    assert_eq!(ends[2].severity, Some(Severity::Error));

    // Verify severity is stored in exported entries too
    let entries = tracer.export();
    let phase_ends: Vec<_> = entries
        .iter()
        .filter(|e| e.action.starts_with("phase_end:"))
        .collect();
    assert_eq!(phase_ends.len(), 3);
    assert_eq!(phase_ends[0].metadata["severity"], "info");
    assert_eq!(phase_ends[1].metadata["severity"], "warn");
    assert_eq!(phase_ends[2].metadata["severity"], "error");
}
