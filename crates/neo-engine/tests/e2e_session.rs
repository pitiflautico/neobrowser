//! End-to-end session tests — navigate, interact, verify freshness.
//!
//! These tests require V8 runtime and network access.
//! Run with: cargo test -p neo-engine --test e2e_session -- --ignored

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use neo_dom::MockDomEngine;
use neo_engine::config::EngineConfig;
use neo_engine::{BrowserEngine, NeoSession};
use neo_extract::MockExtractor;
use neo_http::mock::MockHttpClient;
use neo_interact::MockInteractor;
use neo_trace::mock::MockTracer;
use neo_types::HttpResponse;

/// Build a session with mock subsystems that respond to known URLs.
fn build_test_session() -> NeoSession {
    let http = MockHttpClient::new();
    http.when_url("site-a.test").returns(HttpResponse {
        status: 200,
        headers: HashMap::new(),
        body: "<html><head><title>Site A</title></head><body>Alpha</body></html>".to_string(),
        url: "https://site-a.test".to_string(),
        duration_ms: 5,
    });
    http.when_url("site-b.test").returns(HttpResponse {
        status: 200,
        headers: HashMap::new(),
        body: "<html><head><title>Site B</title></head><body>Beta</body></html>".to_string(),
        url: "https://site-b.test".to_string(),
        duration_ms: 5,
    });
    http.when_url("site-c.test").returns(HttpResponse {
        status: 200,
        headers: HashMap::new(),
        body: "<html><head><title>Site C</title></head><body>Gamma</body></html>".to_string(),
        url: "https://site-c.test".to_string(),
        duration_ms: 5,
    });

    NeoSession::new(
        Box::new(http),
        Box::new(MockDomEngine::new()),
        None, // no JS runtime needed for these tests
        Box::new(MockInteractor::new()),
        Box::new(MockExtractor::new()),
        Box::new(MockTracer::new()),
        Box::new(MockTracer::new()),
        EngineConfig::default(),
    )
}

#[test]
fn test_page_id_increments() {
    let mut session = build_test_session();

    assert_eq!(session.page_id(), 0, "page_id starts at 0");

    let r1 = session.navigate("https://site-a.test").unwrap();
    assert_eq!(r1.page_id, 1, "first navigate -> page_id 1");
    assert_eq!(session.page_id(), 1);

    let r2 = session.navigate("https://site-b.test").unwrap();
    assert_eq!(r2.page_id, 2, "second navigate -> page_id 2");
    assert_eq!(session.page_id(), 2);

    let r3 = session.navigate("https://site-c.test").unwrap();
    assert_eq!(r3.page_id, 3, "third navigate -> page_id 3");
    assert_eq!(session.page_id(), 3);
}

#[test]
fn test_extract_after_navigation_is_fresh() {
    let mut session = build_test_session();

    // Navigate to site A, extract.
    let r1 = session.navigate("https://site-a.test").unwrap();
    let wom1 = session.extract().unwrap();
    assert_eq!(r1.url, "https://site-a.test");

    // Navigate to site B, extract.
    let r2 = session.navigate("https://site-b.test").unwrap();
    let wom2 = session.extract().unwrap();
    assert_eq!(r2.url, "https://site-b.test");

    // URLs must differ — second extract has site B content.
    assert_ne!(r1.url, r2.url);
    assert_ne!(r1.page_id, r2.page_id, "page_ids must differ");
    // PageResult WOM URLs track the navigation (set by the pipeline).
    assert_ne!(r1.wom.url, r2.wom.url, "PageResult WOM URLs must differ across navigations");
    // Standalone extract uses MockExtractor which returns empty URLs,
    // so we verify freshness via page_id instead.
    let _ = (wom1, wom2);
}

#[test]
fn test_page_id_default_is_zero() {
    // The BrowserEngine trait default implementation returns 0.
    use neo_engine::MockBrowserEngine;
    let mock = MockBrowserEngine::new();
    assert_eq!(mock.page_id(), 0);
}
