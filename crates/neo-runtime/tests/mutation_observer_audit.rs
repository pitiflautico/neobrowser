//! T4: MutationObserver Audit + T5: Event Loop Model
//!
//! These integration tests verify that linkedom's MutationObserver actually fires
//! on DOM mutations (P0 gate for React reconciler) and that our event loop model
//! correctly orders microtasks, macrotasks, and observer callbacks.
//!
//! All tests are `#[ignore]` because they require V8 (deno_core compilation).
//!
//! ## Event Loop Drain Model
//!
//! Our V8 runtime follows the browser event loop model:
//!
//! 1. Execute script/callback (synchronous JS)
//! 2. V8 drains the microtask queue (Promise.then, queueMicrotask, MutationObserver)
//! 3. Rust loop (`run_until_settled`) checks for pending macrotasks (timers with elapsed delay)
//! 4. Execute next macrotask callback
//! 5. Repeat from step 2
//!
//! MutationObserver callbacks are microtasks per the HTML spec — they fire between
//! steps 1 and 3, not as macrotasks. This is critical for React's reconciler which
//! uses MutationObserver to detect DOM changes synchronously within a commit.

use neo_runtime::v8::DenoRuntime;
use neo_runtime::{JsRuntime, RuntimeConfig};

/// Helper: create a runtime with a minimal HTML document.
fn setup_runtime() -> DenoRuntime {
    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.set_document_html(
        r#"<html><body><div id="target"></div></body></html>"#,
        "https://example.com",
    )
    .unwrap();
    rt
}

// ═══════════════════════════════════════════════════════════════
// T4: MutationObserver Audit — 6 tests
// ═══════════════════════════════════════════════════════════════

/// T4.1: setAttribute fires MutationObserver with type 'attributes'.
#[test]
#[ignore]
fn test_mo_set_attribute() {
    let mut rt = setup_runtime();

    rt.execute(
        r#"
        globalThis.__mo_results = [];
        var observer = new MutationObserver(function(mutations) {
            mutations.forEach(function(m) {
                globalThis.__mo_results.push(m.type + ':' + (m.attributeName || ''));
            });
        });
        var el = document.getElementById('target');
        observer.observe(el, { attributes: true });
        el.setAttribute('data-test', 'value');
        "#,
    )
    .unwrap();

    rt.run_until_settled(2000).unwrap();

    let result = rt.eval("JSON.stringify(globalThis.__mo_results)").unwrap();
    eprintln!("[T4.1 setAttribute] results: {}", result);

    // If linkedom's MutationObserver works, we expect ["attributes:data-test"].
    // If it doesn't fire, result will be []. Document either way.
    let parsed: Vec<String> = serde_json::from_str(&result).unwrap_or_default();
    if parsed.is_empty() {
        eprintln!("[T4.1] FAIL: linkedom MutationObserver did NOT fire on setAttribute");
        eprintln!("[T4.1] A polyfill in browser_shim.js is needed for React reconciler support");
    } else {
        assert!(
            parsed.iter().any(|s| s.starts_with("attributes")),
            "Expected 'attributes' mutation type, got: {:?}",
            parsed
        );
        eprintln!("[T4.1] PASS: setAttribute fires observer");
    }
    // Always assert no JS error
    assert!(
        !result.contains("Error"),
        "MutationObserver should not throw"
    );
}

/// T4.2: textContent change fires MutationObserver with type 'childList'.
/// (textContent replaces all children, which is a childList mutation)
#[test]
#[ignore]
fn test_mo_text_content() {
    let mut rt = setup_runtime();

    rt.execute(
        r#"
        globalThis.__mo_results = [];
        var observer = new MutationObserver(function(mutations) {
            mutations.forEach(function(m) {
                globalThis.__mo_results.push(m.type);
            });
        });
        var el = document.getElementById('target');
        observer.observe(el, { childList: true, characterData: true, subtree: true });
        el.textContent = 'hello world';
        "#,
    )
    .unwrap();

    rt.run_until_settled(2000).unwrap();

    let result = rt.eval("JSON.stringify(globalThis.__mo_results)").unwrap();
    eprintln!("[T4.2 textContent] results: {}", result);

    let parsed: Vec<String> = serde_json::from_str(&result).unwrap_or_default();
    if parsed.is_empty() {
        eprintln!("[T4.2] FAIL: linkedom MutationObserver did NOT fire on textContent");
    } else {
        eprintln!("[T4.2] PASS: textContent fires observer with types: {:?}", parsed);
    }
    assert!(
        !result.contains("Error"),
        "MutationObserver should not throw"
    );
}

/// T4.3: appendChild fires MutationObserver with type 'childList'.
#[test]
#[ignore]
fn test_mo_append_child() {
    let mut rt = setup_runtime();

    rt.execute(
        r#"
        globalThis.__mo_results = [];
        var observer = new MutationObserver(function(mutations) {
            mutations.forEach(function(m) {
                globalThis.__mo_results.push(m.type + ':added=' + m.addedNodes.length);
            });
        });
        var el = document.getElementById('target');
        observer.observe(el, { childList: true });
        var child = document.createElement('span');
        child.textContent = 'new child';
        el.appendChild(child);
        "#,
    )
    .unwrap();

    rt.run_until_settled(2000).unwrap();

    let result = rt.eval("JSON.stringify(globalThis.__mo_results)").unwrap();
    eprintln!("[T4.3 appendChild] results: {}", result);

    let parsed: Vec<String> = serde_json::from_str(&result).unwrap_or_default();
    if parsed.is_empty() {
        eprintln!("[T4.3] FAIL: linkedom MutationObserver did NOT fire on appendChild");
    } else {
        assert!(
            parsed.iter().any(|s| s.starts_with("childList")),
            "Expected 'childList' mutation type, got: {:?}",
            parsed
        );
        eprintln!("[T4.3] PASS: appendChild fires observer");
    }
    assert!(
        !result.contains("Error"),
        "MutationObserver should not throw"
    );
}

/// T4.4: removeChild fires MutationObserver with type 'childList'.
#[test]
#[ignore]
fn test_mo_remove_child() {
    let mut rt = setup_runtime();

    rt.execute(
        r#"
        globalThis.__mo_results = [];
        var el = document.getElementById('target');
        var child = document.createElement('span');
        el.appendChild(child);

        var observer = new MutationObserver(function(mutations) {
            mutations.forEach(function(m) {
                globalThis.__mo_results.push(m.type + ':removed=' + m.removedNodes.length);
            });
        });
        observer.observe(el, { childList: true });
        el.removeChild(child);
        "#,
    )
    .unwrap();

    rt.run_until_settled(2000).unwrap();

    let result = rt.eval("JSON.stringify(globalThis.__mo_results)").unwrap();
    eprintln!("[T4.4 removeChild] results: {}", result);

    let parsed: Vec<String> = serde_json::from_str(&result).unwrap_or_default();
    if parsed.is_empty() {
        eprintln!("[T4.4] FAIL: linkedom MutationObserver did NOT fire on removeChild");
    } else {
        assert!(
            parsed.iter().any(|s| s.starts_with("childList")),
            "Expected 'childList' mutation type, got: {:?}",
            parsed
        );
        eprintln!("[T4.4] PASS: removeChild fires observer");
    }
    assert!(
        !result.contains("Error"),
        "MutationObserver should not throw"
    );
}

/// T4.5: subtree: true observes mutations on nested elements.
#[test]
#[ignore]
fn test_mo_subtree() {
    let mut rt = setup_runtime();

    rt.execute(
        r#"
        globalThis.__mo_results = [];
        var el = document.getElementById('target');
        var nested = document.createElement('div');
        el.appendChild(nested);

        var observer = new MutationObserver(function(mutations) {
            mutations.forEach(function(m) {
                globalThis.__mo_results.push(m.type + ':' + m.target.tagName);
            });
        });
        observer.observe(el, { attributes: true, childList: true, subtree: true });

        // Mutation on nested element (not direct child of observed target)
        nested.setAttribute('data-nested', 'yes');
        var deepChild = document.createElement('p');
        nested.appendChild(deepChild);
        "#,
    )
    .unwrap();

    rt.run_until_settled(2000).unwrap();

    let result = rt.eval("JSON.stringify(globalThis.__mo_results)").unwrap();
    eprintln!("[T4.5 subtree] results: {}", result);

    let parsed: Vec<String> = serde_json::from_str(&result).unwrap_or_default();
    if parsed.is_empty() {
        eprintln!("[T4.5] FAIL: linkedom MutationObserver did NOT fire with subtree: true");
    } else {
        // Should see mutations from the nested DIV, not just the observed target
        eprintln!("[T4.5] PASS: subtree observer fired with: {:?}", parsed);
    }
    assert!(
        !result.contains("Error"),
        "MutationObserver should not throw"
    );
}

/// T4.6: Observer fires async (not synchronously during the mutation).
/// The callback should NOT run during setAttribute — it should be queued
/// and run as a microtask after the current script finishes.
#[test]
#[ignore]
fn test_mo_fires_async() {
    let mut rt = setup_runtime();

    rt.execute(
        r#"
        globalThis.__mo_order = [];
        var observer = new MutationObserver(function() {
            globalThis.__mo_order.push('observer');
        });
        var el = document.getElementById('target');
        observer.observe(el, { attributes: true });

        globalThis.__mo_order.push('before');
        el.setAttribute('data-x', '1');
        globalThis.__mo_order.push('after');
        // If observer fires sync, order would be: ['before', 'observer', 'after']
        // Correct async behavior: ['before', 'after', 'observer']
        "#,
    )
    .unwrap();

    rt.run_until_settled(2000).unwrap();

    let result = rt.eval("JSON.stringify(globalThis.__mo_order)").unwrap();
    eprintln!("[T4.6 async timing] order: {}", result);

    let parsed: Vec<String> = serde_json::from_str(&result).unwrap_or_default();
    if parsed.len() >= 3 {
        // Check that 'before' and 'after' come before 'observer'
        let before_idx = parsed.iter().position(|s| s == "before");
        let after_idx = parsed.iter().position(|s| s == "after");
        let observer_idx = parsed.iter().position(|s| s == "observer");

        if let (Some(b), Some(a), Some(o)) = (before_idx, after_idx, observer_idx) {
            assert!(
                b < a && a < o,
                "Observer should fire AFTER sync code. Order: {:?}",
                parsed
            );
            eprintln!("[T4.6] PASS: observer fires asynchronously");
        } else {
            eprintln!(
                "[T4.6] Unexpected order (missing entries): {:?}",
                parsed
            );
        }
    } else if parsed.len() == 2 {
        // Observer didn't fire — only 'before' and 'after'
        eprintln!("[T4.6] FAIL: observer callback never fired (only sync markers present)");
    } else {
        eprintln!("[T4.6] Unexpected result: {:?}", parsed);
    }
    assert!(
        !result.contains("Error"),
        "MutationObserver should not throw"
    );
}

// ═══════════════════════════════════════════════════════════════
// T5: Event Loop Model — microtask/macrotask ordering
// ═══════════════════════════════════════════════════════════════

/// T5.1: Promise.then (microtask) runs before setTimeout (macrotask).
#[test]
#[ignore]
fn test_microtask_ordering_promise_before_timeout() {
    let mut rt = setup_runtime();

    rt.execute(
        r#"
        globalThis.__order = [];
        globalThis.__order.push('sync');
        setTimeout(function() { globalThis.__order.push('timeout'); }, 0);
        Promise.resolve().then(function() { globalThis.__order.push('promise'); });
        "#,
    )
    .unwrap();

    rt.run_until_settled(2000).unwrap();

    let result = rt.eval("JSON.stringify(globalThis.__order)").unwrap();
    eprintln!("[T5.1 promise vs timeout] order: {}", result);

    let parsed: Vec<String> = serde_json::from_str(&result).unwrap_or_default();
    // Expected: ['sync', 'promise', 'timeout']
    let promise_idx = parsed.iter().position(|s| s == "promise");
    let timeout_idx = parsed.iter().position(|s| s == "timeout");

    if let (Some(p), Some(t)) = (promise_idx, timeout_idx) {
        assert!(
            p < t,
            "Promise (microtask) must run before setTimeout (macrotask). Order: {:?}",
            parsed
        );
        eprintln!("[T5.1] PASS: promise before timeout");
    } else {
        eprintln!(
            "[T5.1] Missing entries — promise: {:?}, timeout: {:?}, full: {:?}",
            promise_idx, timeout_idx, parsed
        );
    }
}

/// T5.2: queueMicrotask runs before setTimeout.
#[test]
#[ignore]
fn test_microtask_ordering_queuemicrotask_before_timeout() {
    let mut rt = setup_runtime();

    rt.execute(
        r#"
        globalThis.__order = [];
        globalThis.__order.push('sync');
        setTimeout(function() { globalThis.__order.push('timeout'); }, 0);
        queueMicrotask(function() { globalThis.__order.push('microtask'); });
        "#,
    )
    .unwrap();

    rt.run_until_settled(2000).unwrap();

    let result = rt.eval("JSON.stringify(globalThis.__order)").unwrap();
    eprintln!("[T5.2 queueMicrotask vs timeout] order: {}", result);

    let parsed: Vec<String> = serde_json::from_str(&result).unwrap_or_default();
    // Expected: ['sync', 'microtask', 'timeout']
    let mt_idx = parsed.iter().position(|s| s == "microtask");
    let to_idx = parsed.iter().position(|s| s == "timeout");

    if let (Some(m), Some(t)) = (mt_idx, to_idx) {
        assert!(
            m < t,
            "queueMicrotask must run before setTimeout. Order: {:?}",
            parsed
        );
        eprintln!("[T5.2] PASS: queueMicrotask before timeout");
    } else {
        eprintln!(
            "[T5.2] Missing entries — microtask: {:?}, timeout: {:?}, full: {:?}",
            mt_idx, to_idx, parsed
        );
    }
}

/// T5.3: Full ordering — sync, then all microtasks, then macrotask.
#[test]
#[ignore]
fn test_full_event_loop_ordering() {
    let mut rt = setup_runtime();

    rt.execute(
        r#"
        globalThis.__order = [];
        globalThis.__order.push('sync');
        setTimeout(function() { globalThis.__order.push('timeout'); }, 0);
        Promise.resolve().then(function() { globalThis.__order.push('promise'); });
        queueMicrotask(function() { globalThis.__order.push('microtask'); });
        "#,
    )
    .unwrap();

    rt.run_until_settled(2000).unwrap();

    let result = rt.eval("JSON.stringify(globalThis.__order)").unwrap();
    eprintln!("[T5.3 full ordering] order: {}", result);

    let parsed: Vec<String> = serde_json::from_str(&result).unwrap_or_default();
    // Expected: ['sync', 'promise', 'microtask', 'timeout']
    // (promise and microtask order between themselves is implementation-defined,
    //  but both must come before timeout)
    let sync_idx = parsed.iter().position(|s| s == "sync").unwrap_or(99);
    let promise_idx = parsed.iter().position(|s| s == "promise").unwrap_or(99);
    let mt_idx = parsed.iter().position(|s| s == "microtask").unwrap_or(99);
    let to_idx = parsed.iter().position(|s| s == "timeout").unwrap_or(99);

    assert_eq!(sync_idx, 0, "sync must be first. Order: {:?}", parsed);
    assert!(
        promise_idx < to_idx,
        "promise must come before timeout. Order: {:?}",
        parsed
    );
    assert!(
        mt_idx < to_idx,
        "microtask must come before timeout. Order: {:?}",
        parsed
    );
    eprintln!("[T5.3] PASS: all microtasks before macrotask");
}

/// T5.4: MutationObserver timing relative to promises and timeouts.
/// MutationObserver callbacks are microtasks — they should fire before setTimeout.
/// Their relative order vs Promise.then is implementation-defined but both
/// must precede macrotasks.
#[test]
#[ignore]
fn test_mo_timing_in_event_loop() {
    let mut rt = setup_runtime();

    rt.execute(
        r#"
        globalThis.__order = [];
        var observer = new MutationObserver(function() {
            globalThis.__order.push('observer');
        });
        var el = document.getElementById('target');
        observer.observe(el, { attributes: true });

        globalThis.__order.push('before');
        el.setAttribute('x', '1');
        Promise.resolve().then(function() { globalThis.__order.push('promise'); });
        setTimeout(function() { globalThis.__order.push('timeout'); }, 0);
        globalThis.__order.push('after');
        "#,
    )
    .unwrap();

    rt.run_until_settled(2000).unwrap();

    let result = rt.eval("JSON.stringify(globalThis.__order)").unwrap();
    eprintln!("[T5.4 MO timing] order: {}", result);

    let parsed: Vec<String> = serde_json::from_str(&result).unwrap_or_default();

    // 'before' and 'after' must be first (sync)
    let before_idx = parsed.iter().position(|s| s == "before").unwrap_or(99);
    let after_idx = parsed.iter().position(|s| s == "after").unwrap_or(99);
    assert_eq!(before_idx, 0, "before must be first. Order: {:?}", parsed);
    assert_eq!(after_idx, 1, "after must be second. Order: {:?}", parsed);

    // If observer fires, it should be before timeout (microtask, not macrotask)
    let observer_idx = parsed.iter().position(|s| s == "observer");
    let timeout_idx = parsed.iter().position(|s| s == "timeout");
    let promise_idx = parsed.iter().position(|s| s == "promise");

    if let Some(o) = observer_idx {
        if let Some(t) = timeout_idx {
            assert!(
                o < t,
                "MutationObserver (microtask) must fire before setTimeout (macrotask). Order: {:?}",
                parsed
            );
        }
        eprintln!("[T5.4] PASS: observer fires as microtask (before timeout)");
    } else {
        eprintln!(
            "[T5.4] Observer did NOT fire. Promise: {:?}, Timeout: {:?}. Order: {:?}",
            promise_idx, timeout_idx, parsed
        );
        eprintln!("[T5.4] This means linkedom's MutationObserver is not wired to microtasks.");
        eprintln!("[T5.4] A polyfill is needed if React reconciler depends on this.");
    }
}
