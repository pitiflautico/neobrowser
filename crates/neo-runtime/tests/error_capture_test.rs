//! Error capture tests — verify that JS errors are captured, not silently lost.
//!
//! The core problem: when a SPA's app.js throws, we see NOTHING. A real browser
//! would show the error. These tests verify our error capture pipeline works.
//!
//! All tests are #[ignore] because they need V8 (deno_core compiled).
//! Run with: cargo test -p neo-runtime --test error_capture_test -- --ignored

use neo_runtime::v8::DenoRuntime;
use neo_runtime::{JsRuntime, RuntimeConfig};

fn create_runtime() -> DenoRuntime {
    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.set_document_html("<html><body></body></html>", "https://example.com")
        .unwrap();
    rt
}

fn create_runtime_with_html(html: &str, url: &str) -> DenoRuntime {
    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.set_document_html(html, url).unwrap();
    rt
}

// ═══════════════════════════════════════════════════════════════════
// 1. CONSOLE ERROR CAPTURE
// ═══════════════════════════════════════════════════════════════════

/// console.error() should be routed through op_console_log with [error] prefix.
/// We verify by installing a capture array BEFORE calling console.error.
#[test]
#[ignore]
fn test_console_error_is_captured() {
    let mut rt = create_runtime();

    // Install capture hook that wraps the console.error to also store in a global
    rt.execute(
        r#"
        globalThis.__captured_errors = [];
        const _origError = console.error;
        console.error = function(...args) {
            globalThis.__captured_errors.push(args.map(String).join(' '));
            _origError.apply(console, args);
        };
        "#,
    )
    .unwrap();

    rt.execute("console.error('TEST ERROR MESSAGE')").unwrap();

    let result = rt
        .eval("JSON.stringify(globalThis.__captured_errors)")
        .unwrap();
    assert!(
        result.contains("TEST ERROR MESSAGE"),
        "console.error should be captured, got: {result}"
    );
}

/// Verify console.log, console.warn, console.error all work and produce output.
#[test]
#[ignore]
fn test_console_all_levels_captured() {
    let mut rt = create_runtime();

    rt.execute(
        r#"
        globalThis.__msgs = [];
        const _log = console.log;
        const _warn = console.warn;
        const _err = console.error;
        console.log = function(...a) { globalThis.__msgs.push('LOG:' + a.join(' ')); _log.apply(console, a); };
        console.warn = function(...a) { globalThis.__msgs.push('WARN:' + a.join(' ')); _warn.apply(console, a); };
        console.error = function(...a) { globalThis.__msgs.push('ERR:' + a.join(' ')); _err.apply(console, a); };
        "#,
    )
    .unwrap();

    rt.execute("console.log('LOG1'); console.warn('WARN1'); console.error('ERR1');")
        .unwrap();

    let result = rt.eval("JSON.stringify(globalThis.__msgs)").unwrap();
    assert!(result.contains("LOG:LOG1"), "log missing: {result}");
    assert!(result.contains("WARN:WARN1"), "warn missing: {result}");
    assert!(result.contains("ERR:ERR1"), "error missing: {result}");
}

// ═══════════════════════════════════════════════════════════════════
// 2. UNHANDLED PROMISE REJECTION
// ═══════════════════════════════════════════════════════════════════

/// Promise.reject() with no .catch should be captured by onunhandledrejection.
#[test]
#[ignore]
fn test_unhandled_rejection_captured() {
    let mut rt = create_runtime();

    // Install capture for unhandled rejections
    rt.execute(
        r#"
        globalThis.__rejection_msgs = [];
        const _origHandler = globalThis.onunhandledrejection;
        globalThis.onunhandledrejection = function(event) {
            var r = event?.reason;
            globalThis.__rejection_msgs.push(String(r?.message || r));
            if (_origHandler) _origHandler(event);
        };
        "#,
    )
    .unwrap();

    rt.execute("Promise.reject(new Error('UNHANDLED'))").unwrap();
    rt.pump_event_loop().ok();
    // Give microtasks a chance to process
    rt.run_until_settled(2000).ok();

    let result = rt
        .eval(
            "globalThis.__rejection_msgs.length > 0 ? JSON.stringify(globalThis.__rejection_msgs) : 'NONE'",
        )
        .unwrap();
    assert!(
        result.contains("UNHANDLED"),
        "Unhandled rejection should be captured, got: {result}"
    );
}

// ═══════════════════════════════════════════════════════════════════
// 3. MODULE ERRORS
// ═══════════════════════════════════════════════════════════════════

/// A throw at module top-level should be caught, not crash the runtime.
#[test]
#[ignore]
fn test_throw_in_module_captured() {
    let mut rt = create_runtime();

    rt.insert_module(
        "https://example.com/bad.js",
        "throw new Error('MODULE CRASH');",
    );
    let result = rt.load_module("https://example.com/bad.js");

    // The module load should either return an error or the error should be
    // captured by onerror — but the runtime MUST NOT panic.
    // If load_module returns Ok, check if the error was captured by onerror.
    match result {
        Err(e) => {
            let msg = format!("{e}");
            assert!(
                msg.contains("MODULE CRASH") || msg.contains("error"),
                "Error should mention MODULE CRASH, got: {msg}"
            );
        }
        Ok(()) => {
            // Error was swallowed — this is the bug we're testing for.
            // Check if onerror captured it via the bootstrap handler.
            // The bootstrap sends [uncaught] to op_console_log but doesn't
            // store it in a JS-accessible location, so we can only detect
            // it if a side-effect happened.
            // For now, mark that the error was silently swallowed.
            panic!("Module threw but load_module returned Ok — error was silently swallowed");
        }
    }
}

/// Async errors inside a module (setTimeout throw) should be captured.
#[test]
#[ignore]
fn test_async_error_in_module_captured() {
    let mut rt = create_runtime();

    rt.execute(
        r#"
        globalThis.__async_errors = [];
        const _origOnerror = globalThis.onerror;
        globalThis.onerror = function(msg, url, line, col, error) {
            globalThis.__async_errors.push(String(msg));
            if (_origOnerror) _origOnerror(msg, url, line, col, error);
            return true;
        };
        "#,
    )
    .unwrap();

    rt.insert_module(
        "https://example.com/async_bad.js",
        "setTimeout(() => { throw new Error('ASYNC MODULE CRASH'); }, 10);",
    );
    rt.load_module("https://example.com/async_bad.js").unwrap();
    rt.run_until_settled(2000).ok();

    let result = rt
        .eval(
            "globalThis.__async_errors.length > 0 ? JSON.stringify(globalThis.__async_errors) : 'NONE'",
        )
        .unwrap();
    assert!(
        result.contains("ASYNC MODULE CRASH"),
        "Async error in module should be captured, got: {result}"
    );
}

// ═══════════════════════════════════════════════════════════════════
// 4. FETCH ERRORS
// ═══════════════════════════════════════════════════════════════════

/// fetch() to a non-existent URL should reject with an error, not hang.
#[test]
#[ignore]
fn test_fetch_error_captured() {
    let mut rt = create_runtime();

    rt.execute(
        "fetch('https://nonexistent.invalid/api').catch(e => { globalThis.__fetch_error = e.message; })",
    )
    .unwrap();
    rt.run_until_settled(5000).ok();

    let result = rt
        .eval("globalThis.__fetch_error || 'no error'")
        .unwrap();
    assert!(
        result != "no error",
        "Fetch to invalid URL should produce an error, got: {result}"
    );
}

// ═══════════════════════════════════════════════════════════════════
// 5. XHR ERRORS
// ═══════════════════════════════════════════════════════════════════

/// XHR to a non-existent URL should fire onerror.
#[test]
#[ignore]
fn test_xhr_error_captured() {
    let mut rt = create_runtime();

    rt.execute(
        r#"
        var xhr = new XMLHttpRequest();
        xhr.open('GET', 'https://nonexistent.invalid/api');
        xhr.onerror = function(e) { globalThis.__xhr_error = 'XHR_ERROR'; };
        xhr.send();
        "#,
    )
    .unwrap();
    rt.run_until_settled(5000).ok();

    let result = rt
        .eval("globalThis.__xhr_error || 'no error'")
        .unwrap();
    assert!(
        result.contains("XHR_ERROR"),
        "XHR to invalid URL should fire onerror, got: {result}"
    );
}

// ═══════════════════════════════════════════════════════════════════
// 6. MODULE SIDE-EFFECT ERRORS (the factorial scenario)
// ═══════════════════════════════════════════════════════════════════

/// Module has a side effect that fails (async fetch). The catch should fire
/// and set root.textContent to an error message.
#[test]
#[ignore]
fn test_module_side_effect_error_captured() {
    let mut rt = create_runtime_with_html(
        r#"<html><body><div id="root"></div></body></html>"#,
        "https://example.com",
    );

    rt.insert_module(
        "https://example.com/app.js",
        r#"
        const root = document.getElementById('root');
        if (!root) throw new Error('No root element');
        // Simulate async init that fails
        (async () => {
            const resp = await fetch('https://nonexistent.invalid/config');
            const config = await resp.json();
            root.textContent = config.message;
        })().catch(e => {
            console.error('App init failed:', e.message);
            root.textContent = 'Error: ' + e.message;
        });
        "#,
    );
    rt.load_module("https://example.com/app.js").unwrap();
    rt.run_until_settled(5000).ok();

    let result = rt
        .eval("document.getElementById('root').textContent")
        .unwrap();
    assert!(
        result.contains("Error"),
        "Root should show error, got: {result}"
    );
}

// ═══════════════════════════════════════════════════════════════════
// 7. ASYNC GENERATOR ERROR
// ═══════════════════════════════════════════════════════════════════

/// Errors in async generators (yield) should be captured by try/catch.
#[test]
#[ignore]
fn test_generator_yield_error_captured() {
    let mut rt = create_runtime_with_html(
        r#"<html><body><div id="root"></div></body></html>"#,
        "https://example.com",
    );

    rt.insert_module(
        "https://example.com/gen.js",
        r#"
        async function* loadConfig() {
            const resp = await fetch('https://nonexistent.invalid/config');
            yield await resp.json();
        }

        (async () => {
            try {
                for await (const config of loadConfig()) {
                    document.getElementById('root').textContent = config.message;
                }
            } catch(e) {
                console.error('Generator failed:', e.message);
                document.getElementById('root').textContent = 'GenError: ' + e.message;
            }
        })();
        "#,
    );
    rt.load_module("https://example.com/gen.js").unwrap();
    rt.run_until_settled(5000).ok();

    let result = rt
        .eval("document.getElementById('root').textContent")
        .unwrap();
    assert!(
        result.contains("Error") || result.contains("GenError"),
        "Should show error, got: {result}"
    );
}

// ═══════════════════════════════════════════════════════════════════
// 8. CONSOLE BUFFER ACCESSIBILITY
// ═══════════════════════════════════════════════════════════════════

/// Verify console.log/warn/error actually invoke op_console_log (the Rust op).
/// We can't read ConsoleBuffer from tests since it's in OpState, but we can
/// verify the ops exist and don't throw.
#[test]
#[ignore]
fn test_console_ops_dont_throw() {
    let mut rt = create_runtime();

    // These should not throw — they route through op_console_log
    rt.execute("console.log('LOG1')").unwrap();
    rt.execute("console.warn('WARN1')").unwrap();
    rt.execute("console.error('ERR1')").unwrap();
    rt.execute("console.info('INFO1')").unwrap();

    // If we got here without panicking, the ops work
    let result = rt.eval("'ok'").unwrap();
    assert_eq!(result, "ok");
}

// ═══════════════════════════════════════════════════════════════════
// 9. FACTORIAL MOUNT PATTERN
// ═══════════════════════════════════════════════════════════════════

/// Simulates the factorial app mount pattern: find root, render loading,
/// async fetch for config, handle error.
#[test]
#[ignore]
fn test_factorial_mount_pattern() {
    let mut rt = create_runtime_with_html(
        r#"<html><body><div id="factorialRoot"></div></body></html>"#,
        "https://app.factorialhr.com",
    );

    rt.insert_module(
        "https://app.factorialhr.com/app.js",
        r#"
        const root = document.getElementById('factorialRoot');
        if (!root) throw new Error('No factorialRoot');

        // Simulated createRoot (simplified)
        root.innerHTML = '<div>Loading...</div>';

        // Async init (simulates yield V_())
        (async () => {
            try {
                const resp = await fetch('/api/v2/core/me');
                const text = await resp.text();
                // If response is HTML (not JSON), it means not authenticated
                if (text.startsWith('<!doctype') || text.startsWith('<html')) {
                    root.innerHTML = '<div>Login required</div>';
                } else {
                    const data = JSON.parse(text);
                    root.innerHTML = '<div>Welcome ' + data.name + '</div>';
                }
            } catch(e) {
                console.error('Mount failed:', e.message);
                root.innerHTML = '<div>Error: ' + e.message + '</div>';
            }
        })();
        "#,
    );

    rt.load_module("https://app.factorialhr.com/app.js")
        .unwrap();
    rt.run_until_settled(5000).ok();

    let result = rt
        .eval("document.getElementById('factorialRoot').innerHTML")
        .unwrap();
    // Should have SOMETHING — either Login required, Welcome, or Error
    assert!(
        !result.is_empty() && result != "\"\"",
        "Root should have content: {result}"
    );
    // More specifically, since fetch will fail (no real server), we expect an error
    assert!(
        result.contains("Error") || result.contains("Login") || result.contains("Welcome"),
        "Should show meaningful state, got: {result}"
    );
}

// ═══════════════════════════════════════════════════════════════════
// 10. ONERROR HANDLER WORKS
// ═══════════════════════════════════════════════════════════════════

/// Verify that globalThis.onerror is set up by bootstrap and captures errors.
#[test]
#[ignore]
fn test_onerror_handler_exists() {
    let mut rt = create_runtime();

    let result = rt.eval("typeof globalThis.onerror").unwrap();
    assert_eq!(result, "function", "onerror handler should be set by bootstrap");
}

/// Verify that onunhandledrejection is set up by bootstrap.
#[test]
#[ignore]
fn test_onunhandledrejection_handler_exists() {
    let mut rt = create_runtime();

    let result = rt.eval("typeof globalThis.onunhandledrejection").unwrap();
    assert_eq!(
        result, "function",
        "onunhandledrejection handler should be set by bootstrap"
    );
}
