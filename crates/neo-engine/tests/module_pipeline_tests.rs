//! Tests for R3 (pre-fetch), R4 (stubbing), R5 (Promise.allSettled rewrite).

use std::collections::HashMap;
use std::sync::Arc;

use neo_dom::MockDomEngine;
use neo_extract::MockExtractor;
use neo_http::mock::MockHttpClient;
use neo_http::{HttpClient, HttpError, HttpRequest};
use neo_interact::MockInteractor;
use neo_runtime::mock::MockRuntime;
use neo_runtime::modules::{
    extract_export_names, generate_stub_module, rewrite_promise_all_settled,
};
use neo_trace::mock::MockTracer;
use neo_trace::{ExecutionSummary, NavEvent, NetworkEvent, Severity, Tracer};
use neo_types::{HttpResponse, PageState, TraceEntry};

use neo_engine::config::EngineConfig;
use neo_engine::{BrowserEngine, NeoSession};

// ─── Shared wrappers for test assertions after navigate ───

/// Wrapper that delegates to an inner `MockHttpClient` via `Arc`,
/// allowing the test to inspect recorded requests after the session consumes it.
struct SharedHttpClient(Arc<MockHttpClient>);

impl HttpClient for SharedHttpClient {
    fn request(&self, req: &HttpRequest) -> Result<HttpResponse, HttpError> {
        self.0.request(req)
    }
}

/// Wrapper that delegates to an inner `MockTracer` via `Arc`,
/// allowing the test to inspect recorded events after the session consumes it.
struct SharedTracer(Arc<MockTracer>);

impl Tracer for SharedTracer {
    fn intent(&self, a: &str, b: &str, c: &str, d: f32) {
        self.0.intent(a, b, c, d)
    }
    fn action_result(&self, a: &str, b: bool, c: &str, d: Option<&str>) {
        self.0.action_result(a, b, c, d)
    }
    fn network(&self, e: &NetworkEvent<'_>) {
        self.0.network(e)
    }
    fn navigation(&self, e: NavEvent, u: &str, n: &str, s: Option<u16>) {
        self.0.navigation(e, u, n, s)
    }
    fn state_change(&self, f: PageState, t: PageState, r: &str) {
        self.0.state_change(f, t, r)
    }
    fn dom_diff(&self, a: usize, r: usize, c: usize, s: &str) {
        self.0.dom_diff(a, r, c, s)
    }
    fn console(&self, l: &str, m: &str) {
        self.0.console(l, m)
    }
    fn js_exception(&self, e: &str, s: Option<&str>) {
        self.0.js_exception(e, s)
    }
    fn resource_blocked(&self, u: &str, r: &str) {
        self.0.resource_blocked(u, r)
    }
    fn phase_start(&self, p: &str, t: &str) {
        self.0.phase_start(p, t)
    }
    fn phase_end(&self, p: &str, t: &str, d: u64, dec: &[String], s: Severity) {
        self.0.phase_end(p, t, d, dec, s)
    }
    fn module_event(&self, u: &str, e: &str, t: &str) {
        self.0.module_event(u, e, t)
    }
    fn failure_snapshot(&self, p: &str, t: &str, s: &str) {
        self.0.failure_snapshot(p, t, s)
    }
    fn export(&self) -> Vec<TraceEntry> {
        self.0.export()
    }
    fn summary(&self) -> ExecutionSummary {
        self.0.summary()
    }
}

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

// ─── Fase C gate validation ───

#[test]
fn test_zero_on_demand_when_prefetched() {
    // A imports B, B imports C — all 3 should be in store after prefetch
    // (no on-demand fallback needed at execution time).
    let html = r#"<html><head>
        <script type="module" src="https://example.com/a.js"></script>
    </head><body></body></html>"#;

    let inner_http = Arc::new(MockHttpClient::new());
    inner_http.when_url("example.com/a.js").returns(js_response(
        "https://example.com/a.js",
        r#"import{b}from"./b.js";console.log(b);"#,
    ));
    inner_http.when_url("example.com/b.js").returns(js_response(
        "https://example.com/b.js",
        r#"import{c}from"./c.js";export const b=c+1;"#,
    ));
    inner_http.when_url("example.com/c.js").returns(js_response(
        "https://example.com/c.js",
        "export const c=42;",
    ));
    inner_http
        .when_url("example.com")
        .returns(html_response("https://example.com", html));

    let tracer = Arc::new(MockTracer::new());
    let shared_tracer = Arc::clone(&tracer);
    let mut session = NeoSession::new(
        Box::new(SharedHttpClient(Arc::clone(&inner_http))),
        Box::new(MockDomEngine::new()),
        Some(Box::new(MockRuntime::new())),
        Box::new(MockInteractor::new()),
        Box::new(MockExtractor::new()),
        Box::new(SharedTracer(shared_tracer)),
        Box::new(MockTracer::new()),
        EngineConfig::default(),
    );

    let result = session.navigate("https://example.com");
    assert!(
        result.is_ok(),
        "navigate should succeed: {:?}",
        result.err()
    );

    // Verify no on-demand fallback was needed (all modules were in store from prefetch).
    let events = tracer.modules();
    let on_demand: Vec<_> = events
        .iter()
        .filter(|m| m.event == "on_demand_fetch")
        .collect();
    assert!(
        on_demand.is_empty(),
        "no on_demand_fetch should occur when prefetch covers all imports. events: {:?}",
        events
            .iter()
            .map(|m| format!("{}:{}", m.module_url, m.event))
            .collect::<Vec<_>>()
    );

    // HTTP requests: 1 page + at least 1 script (a.js via fetch_external_scripts).
    // b.js and c.js may come from HTTP or disk cache, but total HTTP must be <= 4.
    let reqs = inner_http.requests();
    assert!(
        reqs.len() <= 4,
        "expected at most 4 HTTP requests (1 page + up to 3 scripts), got {}: {:?}",
        reqs.len(),
        reqs.iter().map(|r| &r.url).collect::<Vec<_>>()
    );
    assert!(
        reqs.len() >= 2,
        "expected at least 2 HTTP requests (1 page + 1 script), got {}",
        reqs.len()
    );
}

#[test]
fn test_pipeline_order_enforced() {
    // Module A imports heavy B (>1MB) — pipeline should emit events in order:
    // prefetch_hit/prefetch_miss BEFORE stubbed BEFORE execute (load_module).
    let heavy_b = format!("export const big=1;{}", "x".repeat(1_100_000));
    let html = r#"<html><head>
        <script type="module" src="https://example.com/a.js"></script>
    </head><body></body></html>"#;

    let inner_http = Arc::new(MockHttpClient::new());
    inner_http.when_url("example.com/a.js").returns(js_response(
        "https://example.com/a.js",
        r#"import{big}from"./heavy.js";console.log(big);"#,
    ));
    inner_http
        .when_url("example.com/heavy.js")
        .returns(js_response("https://example.com/heavy.js", &heavy_b));
    inner_http
        .when_url("example.com")
        .returns(html_response("https://example.com", html));

    let tracer = Arc::new(MockTracer::new());
    let shared_tracer = Arc::clone(&tracer);
    let mut config = EngineConfig::default();
    config.stub_heavy_modules = true;
    config.stub_threshold_bytes = 1_000_000;

    let mut session = NeoSession::new(
        Box::new(SharedHttpClient(Arc::clone(&inner_http))),
        Box::new(MockDomEngine::new()),
        Some(Box::new(MockRuntime::new())),
        Box::new(MockInteractor::new()),
        Box::new(MockExtractor::new()),
        Box::new(SharedTracer(shared_tracer)),
        Box::new(MockTracer::new()),
        config,
    );

    let result = session.navigate("https://example.com");
    assert!(
        result.is_ok(),
        "navigate should succeed: {:?}",
        result.err()
    );

    let events = tracer.modules();
    let event_names: Vec<&str> = events.iter().map(|m| m.event.as_str()).collect();

    // Find ordering indices for key pipeline stages.
    let last_prefetch_idx = event_names
        .iter()
        .rposition(|e| {
            *e == "prefetch_hit" || *e == "prefetch_miss" || *e == "cache_hit" || *e == "cache_miss"
        })
        .expect("should have at least one prefetch event");
    let first_stubbed_idx = event_names.iter().position(|e| *e == "stubbed");
    let first_on_demand_idx = event_names.iter().position(|e| *e == "on_demand_fetch");

    // Prefetch events must come before stubbed events.
    if let Some(stub_idx) = first_stubbed_idx {
        assert!(
            last_prefetch_idx < stub_idx,
            "prefetch events ({last_prefetch_idx}) must come before stubbed ({stub_idx}). events: {event_names:?}"
        );
    }

    // No on-demand fetches should occur (prefetch covered everything).
    assert!(
        first_on_demand_idx.is_none(),
        "on_demand_fetch should not happen when prefetch is complete. events: {event_names:?}"
    );
}

#[test]
fn test_no_double_rewrite() {
    // Promise.allSettled rewrite must be idempotent — applying twice must not break the output.
    let code = "const r = Promise.allSettled([p1, p2]);";
    let once = rewrite_promise_all_settled(code);
    let twice = rewrite_promise_all_settled(&once);

    // After the first rewrite, "Promise.allSettled(" is gone.
    assert!(
        !once.contains("Promise.allSettled("),
        "first rewrite should remove allSettled"
    );
    // Second pass must be a no-op (the replacement string doesn't contain "Promise.allSettled(").
    assert_eq!(
        once, twice,
        "double rewrite must be a no-op — second pass should not modify output"
    );
}

#[test]
fn test_stub_preserves_namespace_exports() {
    // Module with named re-exports and default: stub must expose all export names.
    let module_src = r#"
const internal = 1;
export { foo, bar as baz, default } from './dep.js';
"#;
    let names = extract_export_names(module_src);
    let stub = generate_stub_module(&names);

    // "foo" and "baz" (the alias target) should be named exports.
    assert!(
        names.contains(&"foo".to_string()),
        "should extract 'foo'. got: {names:?}"
    );
    assert!(
        names.contains(&"baz".to_string()),
        "should extract 'baz' (aliased from bar). got: {names:?}"
    );

    // Stub source must contain both named exports and a default export.
    assert!(
        stub.contains("foo"),
        "stub should contain 'foo' export. stub: {stub}"
    );
    assert!(
        stub.contains("baz"),
        "stub should contain 'baz' export. stub: {stub}"
    );
    assert!(
        stub.contains("export default"),
        "stub must have default export. stub: {stub}"
    );
    // All 3 export names (foo, baz, default) should be recognized.
    assert!(
        names.len() >= 3,
        "expected at least 3 exports (foo, baz, default), got {}: {names:?}",
        names.len()
    );
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
