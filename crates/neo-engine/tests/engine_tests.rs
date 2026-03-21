//! Integration tests for neo-engine using mock subsystems.

use std::collections::HashMap;

use neo_dom::MockDomEngine;
use neo_extract::MockExtractor;
use neo_http::mock::MockHttpClient;
use neo_http::HttpClient;
use neo_interact::MockInteractor;
use neo_runtime::mock::MockRuntime;
use neo_trace::mock::MockTracer;
use neo_trace::Tracer;
use neo_types::{HttpResponse, PageState};

use neo_engine::config::EngineConfig;
use neo_engine::{BrowserEngine, NeoSession};

/// Build a mock HTTP client that responds to example.com and subpages.
///
/// Rules are ordered most-specific first because MockHttpClient uses
/// substring `contains` matching and returns the first match.
fn mock_http() -> MockHttpClient {
    let http = MockHttpClient::new();
    http.when_url("example.com/page3").returns(HttpResponse {
        status: 200,
        headers: HashMap::new(),
        body: "<html><head><title>Page 3</title></head><body>Page 3</body></html>".to_string(),
        url: "https://example.com/page3".to_string(),
        duration_ms: 10,
    });
    http.when_url("example.com/page2").returns(HttpResponse {
        status: 200,
        headers: HashMap::new(),
        body: "<html><head><title>Page 2</title></head><body>Page 2</body></html>".to_string(),
        url: "https://example.com/page2".to_string(),
        duration_ms: 10,
    });
    http.when_url("example.com").returns(HttpResponse {
        status: 200,
        headers: HashMap::new(),
        body: "<html><head><title>Test</title></head><body>Hello</body></html>".to_string(),
        url: "https://example.com".to_string(),
        duration_ms: 42,
    });
    http
}

/// Build a session with all mocks.
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
    assert_eq!(summary.state, PageState::Idle);
    assert_eq!(summary.failed, 0);
}

// --- Tier 1.4: Navigation State Machine tests ---

#[test]
fn test_back_navigates_to_previous() {
    let mut session = build_session();
    session.navigate("https://example.com").unwrap();
    session.navigate("https://example.com/page2").unwrap();
    let result = session.back().unwrap();
    assert_eq!(result.url, "https://example.com");
}

#[test]
fn test_forward_after_back() {
    let mut session = build_session();
    session.navigate("https://example.com").unwrap();
    session.navigate("https://example.com/page2").unwrap();
    session.back().unwrap();
    let result = session.forward().unwrap();
    assert_eq!(result.url, "https://example.com/page2");
}

#[test]
fn test_history_has_three_urls() {
    let mut session = build_session();
    session.navigate("https://example.com").unwrap();
    session.navigate("https://example.com/page2").unwrap();
    session.navigate("https://example.com/page3").unwrap();
    let history = session.history();
    assert_eq!(history.len(), 3);
    assert_eq!(history[0], "https://example.com");
    assert_eq!(history[1], "https://example.com/page2");
    assert_eq!(history[2], "https://example.com/page3");
}

#[test]
fn test_back_at_start_fails() {
    let mut session = build_session();
    session.navigate("https://example.com").unwrap();
    let result = session.back();
    assert!(result.is_err());
}

#[test]
fn test_forward_at_end_fails() {
    let mut session = build_session();
    session.navigate("https://example.com").unwrap();
    let result = session.forward();
    assert!(result.is_err());
}

#[test]
fn test_redirect_chain() {
    let http = MockHttpClient::new();
    http.when_url("original.com").returns(HttpResponse {
        status: 200,
        headers: HashMap::new(),
        body: "<html><head><title>Redirected</title></head><body></body></html>".to_string(),
        url: "https://final.com/landing".to_string(),
        duration_ms: 10,
    });
    let mut session = NeoSession::new(
        Box::new(http),
        Box::new(neo_dom::MockDomEngine::new()),
        Some(Box::new(MockRuntime::new())),
        Box::new(MockInteractor::new()),
        Box::new(MockExtractor::new()),
        Box::new(MockTracer::new()),
        Box::new(MockTracer::new()),
        EngineConfig::default(),
    );
    let result = session.navigate("https://original.com").unwrap();
    assert!(!result.redirect_chain.is_empty());
    assert_eq!(result.redirect_chain[0], "https://original.com");
}

// --- Tier 1.5: Network Layer tests ---

#[test]
fn test_block_pattern() {
    let http = MockHttpClient::new();
    http.when_url("example.com").returns(HttpResponse {
        status: 200,
        headers: HashMap::new(),
        body: "<html><body>Normal</body></html>".to_string(),
        url: "https://example.com".to_string(),
        duration_ms: 42,
    });
    http.block_pattern("analytics.example.com");
    assert!(!http.is_blocked("https://example.com"));
    assert!(http.is_blocked("https://analytics.example.com/track"));
    let req = neo_http::HttpRequest {
        method: "GET".to_string(),
        url: "https://analytics.example.com/track".to_string(),
        headers: HashMap::new(),
        body: None,
        context: neo_http::RequestContext {
            kind: neo_http::RequestKind::Subresource,
            initiator: "test".to_string(),
            referrer: None,
            frame_id: None,
            top_level_url: None,
        },
        timeout_ms: 5000,
    };
    let resp = http.request(&req).unwrap();
    assert_eq!(resp.status, 200);
    assert!(resp.body.is_empty());
}

#[test]
fn test_network_log() {
    let mut session = build_session();
    session.navigate("https://example.com").unwrap();
    let log = session.network_log();
    assert!(!log.is_empty(), "network log should have entries");
    assert_eq!(log[0].url, "https://example.com");
    assert_eq!(log[0].method, "GET");
    assert_eq!(log[0].status, 200);
}
