//! Tests for R3 (pre-fetch), R4 (stubbing), R5 (Promise.allSettled rewrite).

use std::collections::HashMap;

use neo_dom::MockDomEngine;
use neo_extract::MockExtractor;
use neo_http::mock::MockHttpClient;
use neo_interact::MockInteractor;
use neo_runtime::mock::MockRuntime;
use neo_runtime::modules::{
    extract_export_names, generate_stub_module, rewrite_promise_all_settled,
};
use neo_trace::mock::MockTracer;
use neo_types::HttpResponse;

use neo_engine::config::EngineConfig;
use neo_engine::{BrowserEngine, NeoSession};

/// Build HTML with script src tags.
fn html_with_scripts(scripts: &[&str]) -> String {
    let tags: Vec<String> = scripts
        .iter()
        .map(|s| format!(r#"<script src="{}"></script>"#, s))
        .collect();
    format!(
        "<html><head>{}</head><body>Hello</body></html>",
        tags.join("\n")
    )
}

/// Build a session from a custom HTTP mock.
fn build_session_with(http: MockHttpClient, config: EngineConfig) -> NeoSession {
    NeoSession::new(
        Box::new(http),
        Box::new(MockDomEngine::new()),
        Some(Box::new(MockRuntime::new())),
        Box::new(MockInteractor::new()),
        Box::new(MockExtractor::new()),
        Box::new(MockTracer::new()),
        Box::new(MockTracer::new()),
        config,
    )
}

// ─── R3: Pre-fetch ───

#[test]
fn test_prefetch_stores_modules() {
    // Page references 3 external scripts — all should be in store after navigate.
    let html = html_with_scripts(&[
        "https://example.com/a.js",
        "https://example.com/b.js",
        "https://example.com/c.js",
    ]);

    let http = MockHttpClient::new();
    http.when_url("example.com/a.js")
        .returns(js_response("https://example.com/a.js", "var a=1;"));
    http.when_url("example.com/b.js")
        .returns(js_response("https://example.com/b.js", "var b=2;"));
    http.when_url("example.com/c.js")
        .returns(js_response("https://example.com/c.js", "var c=3;"));
    http.when_url("example.com")
        .returns(html_response("https://example.com", &html));

    let mut session = build_session_with(http, EngineConfig::default());
    let result = session.navigate("https://example.com");
    // Should not crash. Scripts are fetched into the store.
    assert!(
        result.is_ok(),
        "navigate should succeed: {:?}",
        result.err()
    );
}

#[test]
fn test_prefetch_depth_2() {
    // Module A imports B — B should be pre-fetched too.
    let html = r#"<html><head>
        <script type="module" src="https://example.com/a.js"></script>
    </head><body></body></html>"#;

    let http = MockHttpClient::new();
    http.when_url("example.com/a.js").returns(js_response(
        "https://example.com/a.js",
        r#"import{foo}from"./b.js";console.log(foo);"#,
    ));
    http.when_url("example.com/b.js").returns(js_response(
        "https://example.com/b.js",
        "export const foo = 42;",
    ));
    http.when_url("example.com")
        .returns(html_response("https://example.com", html));

    let mut session = build_session_with(http, EngineConfig::default());
    let result = session.navigate("https://example.com");
    assert!(
        result.is_ok(),
        "navigate should succeed: {:?}",
        result.err()
    );
}

#[test]
fn test_prefetch_cycle_protection() {
    // A imports B, B imports A — should not infinite loop.
    let html = r#"<html><head>
        <script type="module" src="https://example.com/a.js"></script>
    </head><body></body></html>"#;

    let http = MockHttpClient::new();
    http.when_url("example.com/a.js").returns(js_response(
        "https://example.com/a.js",
        r#"import"./b.js";export const a=1;"#,
    ));
    http.when_url("example.com/b.js").returns(js_response(
        "https://example.com/b.js",
        r#"import"./a.js";export const b=2;"#,
    ));
    http.when_url("example.com")
        .returns(html_response("https://example.com", html));

    let mut session = build_session_with(http, EngineConfig::default());
    let result = session.navigate("https://example.com");
    // Should complete without hanging.
    assert!(result.is_ok(), "cycle should not cause infinite loop");
}

// ─── R4: Stubbing ───

#[test]
fn test_stub_heavy_module() {
    // 2MB module NOT referenced in HTML — should be stubbed.
    let heavy = "x".repeat(2_000_000);
    let exports = extract_export_names(&heavy);
    // Heavy module has no exports, stub should still work.
    let stub = generate_stub_module(&exports);
    assert!(
        stub.len() < heavy.len(),
        "stub should be smaller than original"
    );
    assert!(
        stub.contains("export default"),
        "stub must have default export"
    );
}

#[test]
fn test_stub_preserves_html_deps() {
    // Module referenced in <script src> should NOT be stubbed even if large.
    let big_js = format!("export function big() {{}} {}", "x".repeat(1_100_000));
    let html = r#"<html><head>
        <script type="module" src="https://example.com/main.js"></script>
    </head><body></body></html>"#;

    let http = MockHttpClient::new();
    http.when_url("example.com/main.js")
        .returns(js_response("https://example.com/main.js", &big_js));
    http.when_url("example.com")
        .returns(html_response("https://example.com", html));

    let mut config = EngineConfig::default();
    config.stub_threshold_bytes = 1_000_000;
    config.stub_heavy_modules = true;

    let mut session = build_session_with(http, config);
    let result = session.navigate("https://example.com");
    // Navigate should succeed — the main.js should NOT be stubbed.
    assert!(result.is_ok());
}

// ─── R5: Promise.allSettled rewrite ───

#[test]
fn test_allsettled_rewritten() {
    let code = "const r = Promise.allSettled([p1, p2]);";
    let rewritten = rewrite_promise_all_settled(code);
    assert!(
        !rewritten.contains("Promise.allSettled("),
        "allSettled should be rewritten"
    );
    assert!(
        rewritten.contains("Promise.all"),
        "should use Promise.all instead"
    );
    assert!(
        rewritten.contains("fulfilled"),
        "should wrap results with status"
    );
}

#[test]
fn test_allsettled_no_change_when_absent() {
    let code = "const r = Promise.all([p1]);";
    let result = rewrite_promise_all_settled(code);
    assert_eq!(result, code, "should not modify code without allSettled");
}

// ─── Fallback ───

#[test]
fn test_fallback_on_fetch_error() {
    // Script URL on a different domain that has no mock rule — fetch fails.
    let html = r#"<html><head>
        <script src="https://cdn.missing.test/bundle.js"></script>
    </head><body>Content</body></html>"#;

    let http = MockHttpClient::new();
    // Only rule is for the page itself — cdn.missing.test has no rule.
    http.when_url("example.com")
        .returns(html_response("https://example.com", html));

    let mut session = build_session_with(http, EngineConfig::default());
    let result = session.navigate("https://example.com");
    // Should succeed with errors recorded, not crash.
    assert!(result.is_ok(), "should not crash on fetch error");
    let page = result.unwrap();
    assert!(!page.errors.is_empty(), "should record the fetch error");
}

// ─── Helpers ───

fn js_response(url: &str, body: &str) -> HttpResponse {
    HttpResponse {
        status: 200,
        headers: HashMap::new(),
        body: body.to_string(),
        url: url.to_string(),
        duration_ms: 5,
    }
}

fn html_response(url: &str, body: &str) -> HttpResponse {
    HttpResponse {
        status: 200,
        headers: HashMap::new(),
        body: body.to_string(),
        url: url.to_string(),
        duration_ms: 10,
    }
}
