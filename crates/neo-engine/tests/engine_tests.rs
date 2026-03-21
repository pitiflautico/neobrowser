//! Integration tests for neo-engine using mock subsystems.

use std::collections::HashMap;

use neo_dom::MockDomEngine;
use neo_extract::MockExtractor;
use neo_http::mock::MockHttpClient;
use neo_interact::MockInteractor;
use neo_runtime::mock::MockRuntime;
use neo_trace::mock::MockTracer;
use neo_trace::Tracer;
use neo_types::{HttpResponse, PageState};

use neo_engine::config::EngineConfig;
use neo_engine::{BrowserEngine, NeoSession};

/// Build a mock HTTP client that responds to example.com.
fn mock_http() -> MockHttpClient {
    let http = MockHttpClient::new();
    http.when_url("example.com").returns(HttpResponse {
        status: 200,
        headers: HashMap::new(),
        body: "<html><head><title>Test</title></head><body>Hello</body></html>".to_string(),
        url: "https://example.com".to_string(),
        duration_ms: 42,
    });
    http
}

/// Build a session with all mocks. Returns the session only.
///
/// The tracer is inside the session and accessible via `trace()`/`summary()`.
fn build_session() -> NeoSession {
    NeoSession::new(
        Box::new(mock_http()),
        Box::new(MockDomEngine::new()),
        Some(Box::new(MockRuntime::new())),
        Box::new(MockInteractor::new()),
        Box::new(MockExtractor::new()),
        Box::new(MockTracer::new()),
        Box::new(MockTracer::new()),
        EngineConfig::default(),
    )
}

#[test]
fn test_navigate_lifecycle() {
    let mut session = build_session();

    assert_eq!(session.page_state(), PageState::Idle);

    let result = session.navigate("https://example.com").unwrap();
    assert_eq!(result.state, PageState::Complete);
    assert_eq!(session.page_state(), PageState::Complete);
}

#[test]
fn test_navigate_traces() {
    let mut session = build_session();

    session.navigate("https://example.com").unwrap();

    let entries = session.trace();
    assert!(!entries.is_empty(), "should have trace entries");

    // Should contain at least an intent and action result entry.
    let has_intent = entries.iter().any(|e| e.action.starts_with("intent:"));
    let has_result = entries.iter().any(|e| e.action.starts_with("action:"));
    assert!(has_intent, "should trace navigate intent");
    assert!(has_result, "should trace navigate result");
}

#[test]
fn test_click_delegates() {
    let mut session = build_session();

    session.navigate("https://example.com").unwrap();

    let result = session.click("#submit-btn").unwrap();
    assert_eq!(result, neo_interact::ClickResult::NoEffect);

    // Click should appear in trace.
    let entries = session.trace();
    let click_entries: Vec<_> = entries
        .iter()
        .filter(|e| e.action.starts_with("intent:") && e.target.as_deref() == Some("#submit-btn"))
        .collect();
    assert!(!click_entries.is_empty(), "click should be traced");
}

#[test]
fn test_extract_returns_wom() {
    let mut session = build_session();

    session.navigate("https://example.com").unwrap();

    let wom = session.extract().unwrap();
    // MockExtractor returns empty WOM with page_type "unknown".
    assert_eq!(wom.page_type, "unknown");
    assert!(wom.nodes.is_empty());
}

#[test]
fn test_config_defaults() {
    let config = EngineConfig::default();

    assert_eq!(config.navigation_timeout_ms, 10_000);
    assert_eq!(config.script_timeout_ms, 5_000);
    assert_eq!(config.stability_timeout_ms, 3_000);
    assert_eq!(config.max_redirects, 10);
    assert!(config.execute_js);
    assert!(config.cache_modules);
    assert!(config.stub_heavy_modules);
    assert_eq!(config.stub_threshold_bytes, 1_000_000);
}

#[test]
fn test_history_tracked() {
    let mut session = build_session();

    session.navigate("https://example.com").unwrap();
    session.navigate("https://example.com/page2").unwrap();

    let history = session.history();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0], "https://example.com");
    assert_eq!(history[1], "https://example.com/page2");
}

#[test]
fn test_invalid_url_returns_error() {
    let mut session = build_session();

    let result = session.navigate("not a url");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("invalid url"));
}

#[test]
fn test_mock_browser_engine() {
    use neo_engine::MockBrowserEngine;

    let mut mock = MockBrowserEngine::new();
    assert_eq!(mock.page_state(), PageState::Idle);

    let result = mock.navigate("https://test.com").unwrap();
    assert_eq!(result.state, PageState::Complete);
    assert_eq!(mock.page_state(), PageState::Complete);
    assert_eq!(mock.actions.len(), 1);

    mock.click("button").unwrap();
    mock.type_text("input", "hello").unwrap();
    assert_eq!(mock.actions.len(), 3);
}

#[test]
fn test_summary_after_navigate() {
    let mut session = build_session();
    session.navigate("https://example.com").unwrap();

    let summary = session.summary();
    assert_eq!(summary.state, PageState::Idle); // MockTracer builds from entries
    assert_eq!(summary.failed, 0);
}
