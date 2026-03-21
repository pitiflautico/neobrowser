use neo_trace::file_tracer::FileTracer;
use neo_trace::mock::MockTracer;
use neo_trace::{NetworkEvent, Tracer};
use neo_types::PageState;

#[test]
fn test_intent_recorded() {
    let tracer = FileTracer::new(None);
    tracer.intent("a1", "click login button", "#login-btn", 0.95);

    let entries = tracer.export();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].action.contains("a1"));
    assert_eq!(entries[0].target.as_deref(), Some("#login-btn"));
}

#[test]
fn test_action_result_links_to_intent() {
    let tracer = FileTracer::new(None);
    tracer.intent("a2", "fill email field", "#email", 0.9);
    tracer.action_result("a2", true, "field filled with test@example.com", None);

    let entries = tracer.export();
    assert_eq!(entries.len(), 2);
    // Both entries share the same action_id "a2"
    assert!(entries[0].action.contains("a2"));
    assert!(entries[1].action.contains("a2"));
}

#[test]
fn test_network_correlates_to_action() {
    let tracer = MockTracer::new();
    tracer.intent("a3", "submit form", "#form", 0.85);
    tracer.network(&NetworkEvent {
        request_id: "r1",
        url: "https://api.example.com/login",
        method: "POST",
        status: 200,
        duration_ms: 150,
        action_id: Some("a3"),
        frame_id: None,
        kind: "fetch",
    });

    let networks = tracer.networks();
    assert_eq!(networks.len(), 1);
    assert_eq!(networks[0].action_id.as_deref(), Some("a3"));
    assert_eq!(networks[0].url, "https://api.example.com/login");
}

#[test]
fn test_state_changes_ordered() {
    let tracer = FileTracer::new(None);
    tracer.state_change(PageState::Idle, PageState::Navigating, "goto requested");
    tracer.state_change(PageState::Navigating, PageState::Loading, "committed");
    tracer.state_change(
        PageState::Loading,
        PageState::Interactive,
        "DOMContentLoaded",
    );
    tracer.state_change(PageState::Interactive, PageState::Complete, "load event");

    let entries = tracer.export();
    assert_eq!(entries.len(), 4);
    assert_eq!(entries[0].state_before, Some(PageState::Idle));
    assert_eq!(entries[0].state_after, Some(PageState::Navigating));
    assert_eq!(entries[3].state_before, Some(PageState::Interactive));
    assert_eq!(entries[3].state_after, Some(PageState::Complete));
    // Timestamps should be non-decreasing
    for i in 1..entries.len() {
        assert!(entries[i].timestamp_ms >= entries[i - 1].timestamp_ms);
    }
}

#[test]
fn test_noop_tracer_returns_empty() {
    let tracer = neo_trace::noop::NoopTracer::new();
    tracer.intent("a1", "click", "#btn", 0.9);
    tracer.action_result("a1", true, "clicked", None);

    assert!(tracer.export().is_empty());
    assert_eq!(tracer.summary().total_actions, 0);
}

#[test]
fn test_mock_records_intents_and_actions() {
    let tracer = MockTracer::new();
    tracer.intent("a1", "click", "#btn", 0.9);
    tracer.intent("a2", "type", "#input", 0.85);
    tracer.action_result("a1", true, "clicked", None);
    tracer.action_result("a2", false, "", Some("element not found"));

    let intents = tracer.intents();
    assert_eq!(intents.len(), 2);
    assert_eq!(intents[0].action_id, "a1");
    assert_eq!(intents[1].confidence, 0.85);

    let actions = tracer.actions();
    assert_eq!(actions.len(), 2);
    assert!(actions[0].success);
    assert!(!actions[1].success);
    assert_eq!(actions[1].error.as_deref(), Some("element not found"));
}
