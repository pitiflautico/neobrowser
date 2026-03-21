use neo_trace::mock::MockTracer;
use neo_trace::{NetworkEvent, Tracer};

#[test]
fn test_summary_counts() {
    let tracer = MockTracer::new();
    tracer.action_result("a1", true, "clicked button", None);
    tracer.action_result("a2", true, "filled field", None);
    tracer.action_result("a3", false, "", Some("timeout"));

    let summary = tracer.summary();
    assert_eq!(summary.total_actions, 3);
    assert_eq!(summary.succeeded, 2);
    assert_eq!(summary.failed, 1);
}

#[test]
fn test_summary_warnings() {
    let tracer = MockTracer::new();
    tracer.resource_blocked("https://tracker.example.com/pixel", "telemetry");
    tracer.resource_blocked("https://ads.example.com/banner", "ads");
    tracer.js_exception("TypeError: null is not an object", None);
    tracer.action_result("a1", false, "", Some("click failed"));

    let summary = tracer.summary();
    assert_eq!(summary.blocked_requests, 2);
    assert_eq!(summary.js_errors, 1);
    assert_eq!(summary.failed, 1);

    // Warnings should mention blocked requests, JS errors, and failed actions
    assert!(summary.warnings.iter().any(|w| w.contains("blocked")));
    assert!(summary.warnings.iter().any(|w| w.contains("JS errors")));
    assert!(summary.warnings.iter().any(|w| w.contains("failed")));
}

#[test]
fn test_summary_dom_changes() {
    let tracer = MockTracer::new();
    tracer.dom_diff(10, 2, 5, "modal appeared with form");
    tracer.dom_diff(0, 8, 0, "old content removed");

    let summary = tracer.summary();
    assert_eq!(summary.dom_changes, 25); // 10+2+5 + 0+8+0
}

#[test]
fn test_summary_network_requests() {
    let tracer = MockTracer::new();
    tracer.network(&NetworkEvent {
        request_id: "r1",
        url: "https://api.example.com/data",
        method: "GET",
        status: 200,
        duration_ms: 50,
        action_id: None,
        frame_id: None,
        kind: "fetch",
    });
    tracer.network(&NetworkEvent {
        request_id: "r2",
        url: "https://api.example.com/auth",
        method: "POST",
        status: 401,
        duration_ms: 30,
        action_id: None,
        frame_id: None,
        kind: "xhr",
    });

    let summary = tracer.summary();
    assert_eq!(summary.total_requests, 2);
}
