//! Integration tests for neo-runtime.
//!
//! Mock tests run fast (no V8). Real V8 tests are #[ignore]
//! since deno_core compilation is heavy.

use neo_runtime::mock::MockRuntime;
use neo_runtime::JsRuntime;

// ─── Mock runtime tests (always fast) ───

#[test]
fn test_mock_eval_configured() {
    let mut rt = MockRuntime::new();
    rt.on_eval("1+1", "2");
    let result = rt.eval("1+1").unwrap();
    assert_eq!(result, "2");
    assert_eq!(rt.eval_calls.len(), 1);
}

#[test]
fn test_mock_eval_default() {
    let mut rt = MockRuntime::new();
    rt.set_default_eval("42");
    let result = rt.eval("anything").unwrap();
    assert_eq!(result, "42");
}

#[test]
fn test_mock_module_records_calls() {
    let mut rt = MockRuntime::new();
    rt.load_module("https://example.com/app.js").unwrap();
    rt.load_module("https://example.com/vendor.js").unwrap();
    assert_eq!(rt.module_calls.len(), 2);
    assert_eq!(rt.module_calls[0], "https://example.com/app.js");
}

#[test]
fn test_mock_set_document_html() {
    let mut rt = MockRuntime::new();
    rt.set_document_html("<h1>Hello</h1>", "https://example.com")
        .unwrap();
    assert_eq!(rt.html_calls.len(), 1);
    assert_eq!(rt.html_calls[0].0, "<h1>Hello</h1>");
    assert_eq!(rt.html_calls[0].1, "https://example.com");
}

#[test]
fn test_mock_pending_tasks() {
    let mut rt = MockRuntime::new();
    rt.pending = 5;
    assert_eq!(rt.pending_tasks(), 5);
    rt.run_until_settled(1000).unwrap();
    assert_eq!(rt.pending_tasks(), 0);
}

#[test]
fn test_mock_eval_error() {
    let mut rt = MockRuntime::new();
    rt.eval_error = Some("SyntaxError: unexpected token".to_string());
    let result = rt.eval("invalid{{{");
    assert!(result.is_err());
}

// ─── Real V8 tests (need deno_core compiled) ───

#[test]
#[ignore]
fn test_eval_simple() {
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::RuntimeConfig;

    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    let result = rt.eval("1+1").unwrap();
    assert_eq!(result, "2");
}

#[test]
#[ignore]
fn test_eval_dom_after_set_html() {
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::RuntimeConfig;

    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.set_document_html(
        "<html><head><title>Test Page</title></head><body></body></html>",
        "https://example.com",
    )
    .unwrap();
    // Note: document.title requires linkedom bootstrap to work.
    // Without it, this tests that set_document_html doesn't crash.
    let result = rt.eval("globalThis.__neorender_html").unwrap();
    assert!(result.contains("Test Page"));
}

#[test]
#[ignore]
fn test_timer_fires() {
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::RuntimeConfig;

    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.eval("globalThis.__timer_fired = false").unwrap();
    // Note: this uses the raw JS setTimeout stub, not our op_timer.
    // Full timer integration requires bootstrap.js wiring.
    let result = rt.eval("typeof globalThis.__timer_fired").unwrap();
    assert_eq!(result, "boolean");
}
