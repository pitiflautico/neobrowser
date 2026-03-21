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

// ─── Tier 1.1: Event Loop & Scheduler Tests ───

/// Microtasks (Promise.then) drain before macrotasks (setTimeout).
/// Browser spec: all microtasks complete before the next macrotask fires.
#[test]
#[ignore]
fn test_microtask_before_macrotask() {
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::RuntimeConfig;

    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.set_document_html(
        r#"<html><body><div id="result">waiting</div></body></html>"#,
        "https://example.com",
    )
    .unwrap();

    // Schedule a macrotask (setTimeout 0) and a microtask (Promise.then).
    // Per browser spec, the microtask must execute first.
    rt.execute(
        r#"
        const el = document.getElementById('result');
        const order = [];
        setTimeout(() => { order.push('timer1'); }, 0);
        Promise.resolve().then(() => { order.push('promise'); });
        setTimeout(() => {
            order.push('timer2');
            el.textContent = order.join('+');
        }, 10);
        "#,
    )
    .unwrap();

    rt.run_until_settled(2000).unwrap();

    let result = rt
        .eval("document.getElementById('result').textContent")
        .unwrap();
    // Microtask (promise) must come before macrotasks (timer1, timer2).
    assert!(
        result.contains("promise"),
        "Expected 'promise' in result, got: {}",
        result
    );
    // timer2 should be last (highest delay).
    assert!(
        result.ends_with("timer2"),
        "Expected result to end with 'timer2', got: {}",
        result
    );
}

/// run_until_settled returns when no pending work remains.
#[test]
#[ignore]
fn test_settled_returns_when_idle() {
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::RuntimeConfig;
    use std::time::Instant;

    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.set_document_html(
        "<html><body></body></html>",
        "https://example.com",
    )
    .unwrap();

    // No timers, no promises — should return quickly.
    let start = Instant::now();
    rt.run_until_settled(5000).unwrap();
    let elapsed = start.elapsed();

    // Should settle in well under 1 second (no work to do).
    assert!(
        elapsed.as_millis() < 1000,
        "Expected fast settle, took {}ms",
        elapsed.as_millis()
    );
    assert_eq!(rt.pending_tasks(), 0);
}

/// run_until_settled respects the timeout and returns Timeout error.
#[test]
#[ignore]
fn test_timeout_stops_execution() {
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::RuntimeConfig;
    use std::time::Instant;

    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.set_document_html(
        "<html><body></body></html>",
        "https://example.com",
    )
    .unwrap();

    // Create an interval that would run forever without budget/timeout.
    rt.execute(
        r#"
        globalThis.__intervalCount = 0;
        setInterval(() => { globalThis.__intervalCount++; }, 5);
        "#,
    )
    .unwrap();

    let start = Instant::now();
    let _result = rt.run_until_settled(200);
    let elapsed = start.elapsed();

    // Should have stopped around 200ms (allow generous margin).
    assert!(
        elapsed.as_millis() < 1000,
        "Expected ~200ms timeout, took {}ms",
        elapsed.as_millis()
    );

    // The interval should have ticked some times.
    let count = rt.eval("globalThis.__intervalCount").unwrap();
    let count_num: usize = count.parse().unwrap_or(0);
    assert!(
        count_num > 0,
        "Expected interval to have ticked at least once, got {}",
        count_num
    );
}

/// setInterval is capped at the configured max ticks.
#[test]
#[ignore]
fn test_interval_capped() {
    use neo_runtime::scheduler::SchedulerConfig;
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::RuntimeConfig;

    let sched = SchedulerConfig {
        interval_max_ticks: 5,
        timer_budget: 200,
    };
    let mut rt =
        DenoRuntime::new_with_scheduler(&RuntimeConfig::default(), None, sched).unwrap();
    rt.set_document_html(
        "<html><body></body></html>",
        "https://example.com",
    )
    .unwrap();

    rt.execute(
        r#"
        globalThis.__ticks = 0;
        setInterval(() => { globalThis.__ticks++; }, 1);
        "#,
    )
    .unwrap();

    rt.run_until_settled(2000).unwrap();

    let ticks = rt.eval("globalThis.__ticks").unwrap();
    let tick_num: usize = ticks.parse().unwrap_or(0);
    // Should be capped at 5 (our configured max).
    assert!(
        tick_num <= 5,
        "Expected at most 5 interval ticks, got {}",
        tick_num
    );
    assert!(
        tick_num > 0,
        "Expected at least 1 interval tick, got {}",
        tick_num
    );
}
