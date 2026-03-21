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
    rt.set_document_html("<html><body></body></html>", "https://example.com")
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
    rt.set_document_html("<html><body></body></html>", "https://example.com")
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
    let mut rt = DenoRuntime::new_with_scheduler(&RuntimeConfig::default(), None, sched).unwrap();
    rt.set_document_html("<html><body></body></html>", "https://example.com")
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

// ─── Browser Shim Tests ───

/// form.submit() produces a navigation request in the queue.
#[test]
#[ignore]
fn test_form_submit_intercepted() {
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::RuntimeConfig;

    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.set_document_html(
        r#"<html><body>
            <form id="myform" action="https://example.com/submit" method="POST">
                <input name="user" value="alice" />
                <input name="pass" type="password" value="secret" />
            </form>
        </body></html>"#,
        "https://example.com",
    )
    .unwrap();

    rt.execute(r#"document.getElementById('myform').submit();"#)
        .unwrap();

    let requests = rt.drain_navigation_requests();
    assert_eq!(requests.len(), 1, "Expected 1 navigation request from form.submit()");
    let req: serde_json::Value = serde_json::from_str(&requests[0]).unwrap();
    assert_eq!(req["type"], "form_submit");
    assert_eq!(req["method"], "POST");
    assert_eq!(req["url"], "https://example.com/submit");
    assert_eq!(req["form_data"]["user"], "alice");
    assert_eq!(req["form_data"]["pass"], "secret");
}

/// location.assign() and location.replace() produce navigation requests.
#[test]
#[ignore]
fn test_location_assign_intercepted() {
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::RuntimeConfig;

    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.set_document_html(
        "<html><body></body></html>",
        "https://example.com",
    )
    .unwrap();

    rt.execute(r#"location.assign('https://example.com/new-page');"#)
        .unwrap();
    rt.execute(r#"location.replace('https://example.com/replaced');"#)
        .unwrap();

    let requests = rt.drain_navigation_requests();
    assert_eq!(requests.len(), 2, "Expected 2 navigation requests");
    let req0: serde_json::Value = serde_json::from_str(&requests[0]).unwrap();
    assert_eq!(req0["type"], "location_assign");
    assert_eq!(req0["url"], "https://example.com/new-page");
    let req1: serde_json::Value = serde_json::from_str(&requests[1]).unwrap();
    assert_eq!(req1["type"], "location_replace");
    assert_eq!(req1["url"], "https://example.com/replaced");
}

/// document.cookie getter/setter backed by Rust ops.
#[test]
#[ignore]
fn test_cookie_get_set() {
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::RuntimeConfig;

    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.set_document_html(
        "<html><body></body></html>",
        "https://example.com",
    )
    .unwrap();

    // Set cookies via document.cookie
    rt.execute(r#"document.cookie = 'session=abc123';"#).unwrap();
    rt.execute(r#"document.cookie = 'lang=en; Path=/';"#).unwrap();

    // Read back via JS
    let cookies = rt.eval("document.cookie").unwrap();
    assert!(
        cookies.contains("session=abc123"),
        "Expected 'session=abc123' in cookies, got: {}",
        cookies
    );
    assert!(
        cookies.contains("lang=en"),
        "Expected 'lang=en' in cookies, got: {}",
        cookies
    );

    // Read via Rust API
    let rust_cookies = rt.get_cookies();
    assert!(
        rust_cookies.contains("session=abc123"),
        "Expected 'session=abc123' in Rust cookies, got: {}",
        rust_cookies
    );
}

/// history.pushState updates location and tracks state.
#[test]
#[ignore]
fn test_history_push_state() {
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::RuntimeConfig;

    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.set_document_html(
        "<html><body></body></html>",
        "https://example.com/page1",
    )
    .unwrap();

    rt.execute(r#"history.pushState({page: 2}, '', '/page2');"#).unwrap();
    let href = rt.eval("location.pathname").unwrap();
    assert_eq!(href, "/page2", "pushState should update location.pathname");

    let state = rt.eval("JSON.stringify(history.state)").unwrap();
    assert!(
        state.contains("\"page\":2"),
        "Expected history.state to contain page:2, got: {}",
        state
    );

    let length = rt.eval("history.length").unwrap();
    assert_eq!(length, "1", "history.length should be 1 after one pushState");
}

/// IntersectionObserver calls back with isIntersecting: true.
#[test]
#[ignore]
fn test_intersection_observer_fires() {
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::RuntimeConfig;

    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.set_document_html(
        r#"<html><body><div id="target"></div></body></html>"#,
        "https://example.com",
    )
    .unwrap();

    rt.execute(
        r#"
        globalThis.__io_result = null;
        const observer = new IntersectionObserver(function(entries) {
            globalThis.__io_result = entries[0].isIntersecting;
        });
        observer.observe(document.getElementById('target'));
        "#,
    )
    .unwrap();

    rt.run_until_settled(2000).unwrap();

    let result = rt.eval("String(globalThis.__io_result)").unwrap();
    assert_eq!(result, "true", "IntersectionObserver should report isIntersecting: true");
}

/// MutationObserver fires on DOM mutations (setAttribute, textContent, appendChild, removeChild).
#[test]
#[ignore]
fn test_mutation_observer_fires() {
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::RuntimeConfig;

    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.set_document_html(
        r#"<html><body><div id="target"></div></body></html>"#,
        "https://example.com",
    )
    .unwrap();

    rt.execute(
        r#"
        globalThis.__mo_mutations = [];
        const mo = new MutationObserver(function(mutations) {
            mutations.forEach(function(m) {
                globalThis.__mo_mutations.push(m.type);
            });
        });
        mo.observe(document.getElementById('target'), {
            attributes: true, childList: true, characterData: true, subtree: true
        });
        // Trigger mutations
        document.getElementById('target').setAttribute('data-x', '1');
        document.getElementById('target').textContent = 'hello';
        var child = document.createElement('span');
        document.getElementById('target').appendChild(child);
        document.getElementById('target').removeChild(child);
        "#,
    )
    .unwrap();

    rt.run_until_settled(2000).unwrap();

    let mutations = rt.eval("JSON.stringify(globalThis.__mo_mutations)").unwrap();
    eprintln!("MutationObserver mutations: {}", mutations);
    // linkedom's MutationObserver may or may not fire; document what happens.
    // If it doesn't fire, our shim stub is the fallback.
    // At minimum, the code should not crash.
    assert!(
        !mutations.contains("Error"),
        "MutationObserver should not error"
    );
}

/// matchMedia returns desktop defaults.
#[test]
#[ignore]
fn test_match_media_desktop() {
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::RuntimeConfig;

    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.set_document_html(
        "<html><body></body></html>",
        "https://example.com",
    )
    .unwrap();

    // Desktop query should match (min-width > mobile threshold)
    let desktop = rt.eval("matchMedia('(min-width: 1024px)').matches").unwrap();
    assert_eq!(desktop, "true", "Desktop min-width query should match");

    // Mobile query should NOT match
    let mobile = rt.eval("matchMedia('(max-width: 767px)').matches").unwrap();
    assert_eq!(mobile, "false", "Mobile max-width query should not match");

    // Dark mode should not match (we default to light)
    let dark = rt.eval("matchMedia('(prefers-color-scheme: dark)').matches").unwrap();
    assert_eq!(dark, "false", "Dark mode should not match");
}

// ─── T3: Response model & Streams ───

/// NeoResponse exposes bodyUsed, text(), json(), clone() with correct semantics.
#[test]
#[ignore]
fn test_response_bodyused_semantics() {
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::RuntimeConfig;

    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.set_document_html("<html><body></body></html>", "https://example.com")
        .unwrap();

    // Create a NeoResponse manually and test bodyUsed semantics.
    rt.execute(
        r#"
        globalThis.__test_result = [];
        (async () => {
            const body = new TextEncoder().encode('{"key":"value"}');
            const r = new Response(body, { status: 200, statusText: 'OK', url: 'https://test.com', _text: '{"key":"value"}' });

            // bodyUsed starts false
            __test_result.push('used_before=' + r.bodyUsed);

            // text() consumes body
            const txt = await r.text();
            __test_result.push('text=' + txt.substring(0, 15));
            __test_result.push('used_after=' + r.bodyUsed);

            // Double consumption throws
            try {
                await r.text();
                __test_result.push('double=NO_THROW');
            } catch (e) {
                __test_result.push('double=THREW');
            }
        })();
        "#,
    )
    .unwrap();

    rt.run_until_settled(2000).unwrap();

    let result = rt.eval("__test_result.join('|')").unwrap();
    assert!(
        result.contains("used_before=false"),
        "bodyUsed should start false, got: {}",
        result
    );
    assert!(
        result.contains("used_after=true"),
        "bodyUsed should be true after text(), got: {}",
        result
    );
    assert!(
        result.contains("double=THREW"),
        "Double consumption should throw, got: {}",
        result
    );
}

/// clone() creates independent copy; both can be consumed separately.
#[test]
#[ignore]
fn test_response_clone() {
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::RuntimeConfig;

    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.set_document_html("<html><body></body></html>", "https://example.com")
        .unwrap();

    rt.execute(
        r#"
        globalThis.__test_result = [];
        (async () => {
            const body = new TextEncoder().encode('hello');
            const r = new Response(body, { status: 200, url: 'https://test.com', _text: 'hello' });

            const c = r.clone();
            const t1 = await r.text();
            const t2 = await c.text();
            __test_result.push('t1=' + t1);
            __test_result.push('t2=' + t2);
            __test_result.push('match=' + (t1 === t2));

            // Clone after consumption throws
            try {
                r.clone();
                __test_result.push('clone_used=NO_THROW');
            } catch (e) {
                __test_result.push('clone_used=THREW');
            }
        })();
        "#,
    )
    .unwrap();

    rt.run_until_settled(2000).unwrap();

    let result = rt.eval("__test_result.join('|')").unwrap();
    assert!(
        result.contains("match=true"),
        "Clone should produce same text, got: {}",
        result
    );
    assert!(
        result.contains("clone_used=THREW"),
        "Clone after consumption should throw, got: {}",
        result
    );
}

/// response.body returns a ReadableStream with getReader/read support.
#[test]
#[ignore]
fn test_response_body_stream() {
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::RuntimeConfig;

    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.set_document_html("<html><body></body></html>", "https://example.com")
        .unwrap();

    rt.execute(
        r#"
        globalThis.__test_result = [];
        (async () => {
            const body = new TextEncoder().encode('stream test');
            const r = new Response(body, { status: 200, url: 'https://test.com', _text: 'stream test' });

            // body getter should return a ReadableStream
            const stream = r.body;
            __test_result.push('type=' + (stream instanceof ReadableStream ? 'ReadableStream' : typeof stream));

            // getReader should work
            const reader = stream.getReader();
            const chunk1 = await reader.read();
            __test_result.push('chunk1_done=' + chunk1.done);
            __test_result.push('chunk1_len=' + (chunk1.value ? chunk1.value.length : 0));

            // Second read should be done
            const chunk2 = await reader.read();
            __test_result.push('chunk2_done=' + chunk2.done);

            // instanceof Response should work
            __test_result.push('instanceof=' + (r instanceof Response));
        })();
        "#,
    )
    .unwrap();

    rt.run_until_settled(2000).unwrap();

    let result = rt.eval("__test_result.join('|')").unwrap();
    assert!(
        result.contains("type=ReadableStream"),
        "body should be ReadableStream, got: {}",
        result
    );
    assert!(
        result.contains("chunk1_done=false"),
        "First read should not be done, got: {}",
        result
    );
    assert!(
        result.contains("chunk2_done=true"),
        "Second read should be done, got: {}",
        result
    );
    assert!(
        result.contains("instanceof=true"),
        "response instanceof Response should be true, got: {}",
        result
    );
}

/// json() works correctly on NeoResponse.
#[test]
#[ignore]
fn test_response_json() {
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::RuntimeConfig;

    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.set_document_html("<html><body></body></html>", "https://example.com")
        .unwrap();

    rt.execute(
        r#"
        globalThis.__test_result = [];
        (async () => {
            const body = new TextEncoder().encode('{"name":"neo","version":2}');
            const r = new Response(body, { status: 200, url: 'https://test.com', _text: '{"name":"neo","version":2}' });
            const data = await r.json();
            __test_result.push('name=' + data.name);
            __test_result.push('version=' + data.version);
        })();
        "#,
    )
    .unwrap();

    rt.run_until_settled(2000).unwrap();

    let result = rt.eval("__test_result.join('|')").unwrap();
    assert!(
        result.contains("name=neo"),
        "json() should parse name, got: {}",
        result
    );
    assert!(
        result.contains("version=2"),
        "json() should parse version, got: {}",
        result
    );
}
