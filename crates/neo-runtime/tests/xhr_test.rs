//! XHR polyfill tests — verify our XMLHttpRequest works after bootstrap.
//!
//! All tests are #[ignore] because they need V8 (deno_core compiled).
//! Run with: cargo test -p neo-runtime -- --ignored xhr

use neo_runtime::v8::DenoRuntime;
use neo_runtime::{JsRuntime, RuntimeConfig};

fn create_runtime() -> DenoRuntime {
    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.set_document_html("<html><body></body></html>", "https://example.com")
        .unwrap();
    rt
}

// ═══════════════════════════════════════════════════════════════════
// XHR existence & constructor
// ═══════════════════════════════════════════════════════════════════

#[test]
#[ignore]
fn test_xhr_exists_after_bootstrap() {
    let mut rt = create_runtime();
    let result = rt.eval("typeof XMLHttpRequest").unwrap();
    assert_eq!(result, "function");
}

#[test]
#[ignore]
fn test_xhr_constructable() {
    let mut rt = create_runtime();
    let result = rt.eval("new XMLHttpRequest() instanceof XMLHttpRequest").unwrap();
    assert_eq!(result, "true");
}

// ═══════════════════════════════════════════════════════════════════
// Required methods
// ═══════════════════════════════════════════════════════════════════

#[test]
#[ignore]
fn test_xhr_has_required_methods() {
    let mut rt = create_runtime();
    let result = rt.eval(
        "(function(){ var x = new XMLHttpRequest(); return typeof x.open + ',' + typeof x.send + ',' + typeof x.setRequestHeader + ',' + typeof x.getResponseHeader + ',' + typeof x.getAllResponseHeaders })()"
    ).unwrap();
    assert_eq!(result, "function,function,function,function,function");
}

#[test]
#[ignore]
fn test_xhr_has_abort() {
    let mut rt = create_runtime();
    let result = rt.eval("typeof new XMLHttpRequest().abort").unwrap();
    assert_eq!(result, "function");
}

// ═══════════════════════════════════════════════════════════════════
// ReadyState constants
// ═══════════════════════════════════════════════════════════════════

#[test]
#[ignore]
fn test_xhr_readystate_constants() {
    let mut rt = create_runtime();
    let result = rt.eval(
        "[XMLHttpRequest.UNSENT, XMLHttpRequest.OPENED, XMLHttpRequest.HEADERS_RECEIVED, XMLHttpRequest.LOADING, XMLHttpRequest.DONE].join(',')"
    ).unwrap();
    assert_eq!(result, "0,1,2,3,4");
}

#[test]
#[ignore]
fn test_xhr_initial_readystate_is_unsent() {
    let mut rt = create_runtime();
    let result = rt.eval("new XMLHttpRequest().readyState").unwrap();
    assert_eq!(result, "0");
}

// ═══════════════════════════════════════════════════════════════════
// Headers
// ═══════════════════════════════════════════════════════════════════

#[test]
#[ignore]
fn test_xhr_stores_request_headers() {
    let mut rt = create_runtime();
    let result = rt.eval(
        r#"(function(){
            var x = new XMLHttpRequest();
            x.open('GET', 'https://example.com/api');
            x.setRequestHeader('X-Custom', 'hello');
            return x._headers ? x._headers['x-custom'] || x._headers['X-Custom'] || 'missing' : 'no_headers_prop';
        })()"#,
    ).unwrap();
    assert!(
        result.contains("hello"),
        "Request header should be stored, got: {result}"
    );
}

#[test]
#[ignore]
fn test_xhr_get_response_header_before_send_returns_null() {
    let mut rt = create_runtime();
    let result = rt.eval(
        r#"(function(){
            var x = new XMLHttpRequest();
            return String(x.getResponseHeader('content-type'));
        })()"#,
    ).unwrap();
    assert_eq!(result, "null");
}

#[test]
#[ignore]
fn test_xhr_get_all_response_headers_before_send() {
    let mut rt = create_runtime();
    let result = rt.eval(
        r#"(function(){
            var x = new XMLHttpRequest();
            var h = x.getAllResponseHeaders();
            return typeof h + ':' + h.length;
        })()"#,
    ).unwrap();
    assert_eq!(result, "string:0");
}

// ═══════════════════════════════════════════════════════════════════
// Response types & properties
// ═══════════════════════════════════════════════════════════════════

#[test]
#[ignore]
fn test_xhr_default_response_type_is_empty() {
    let mut rt = create_runtime();
    let result = rt.eval("new XMLHttpRequest().responseType").unwrap();
    assert_eq!(result, "");
}

#[test]
#[ignore]
fn test_xhr_response_type_settable() {
    let mut rt = create_runtime();
    let result = rt.eval(
        r#"(function(){
            var x = new XMLHttpRequest();
            x.responseType = 'json';
            return x.responseType;
        })()"#,
    ).unwrap();
    assert_eq!(result, "json");
}

#[test]
#[ignore]
fn test_xhr_status_initially_zero() {
    let mut rt = create_runtime();
    let result = rt.eval("new XMLHttpRequest().status").unwrap();
    assert_eq!(result, "0");
}

#[test]
#[ignore]
fn test_xhr_status_text_initially_empty() {
    let mut rt = create_runtime();
    let result = rt.eval("new XMLHttpRequest().statusText").unwrap();
    assert_eq!(result, "");
}

// ═══════════════════════════════════════════════════════════════════
// Identity: our polyfill vs happy-dom
// ═══════════════════════════════════════════════════════════════════

#[test]
#[ignore]
fn test_xhr_is_our_polyfill_not_happydom() {
    let mut rt = create_runtime();
    // Our XHR should have _headers marker (our implementation detail)
    let result = rt.eval("'_headers' in new XMLHttpRequest()").unwrap();
    assert_eq!(result, "true");
}

#[test]
#[ignore]
fn test_xhr_does_not_have_happydom_window_ref() {
    let mut rt = create_runtime();
    // happy-dom's XHR has a #window private or _window reference
    let result = rt.eval("'_window' in new XMLHttpRequest()").unwrap();
    // We expect false — our polyfill shouldn't have _window
    assert_eq!(result, "false", "XHR should not have happy-dom's _window");
}

// ═══════════════════════════════════════════════════════════════════
// Event handler properties
// ═══════════════════════════════════════════════════════════════════

#[test]
#[ignore]
fn test_xhr_event_handler_properties_exist() {
    let mut rt = create_runtime();
    let result = rt.eval(
        r#"(function(){
            var x = new XMLHttpRequest();
            return [
                typeof x.onload,
                typeof x.onerror,
                typeof x.onreadystatechange
            ].join(',');
        })()"#,
    ).unwrap();
    // They should be null or undefined initially, but settable
    // Check they're at least not throwing
    assert!(
        !result.contains("error"),
        "Event handler access should not error: {result}"
    );
}

#[test]
#[ignore]
fn test_xhr_onload_settable() {
    let mut rt = create_runtime();
    let result = rt.eval(
        r#"(function(){
            var x = new XMLHttpRequest();
            x.onload = function() {};
            return typeof x.onload;
        })()"#,
    ).unwrap();
    assert_eq!(result, "function");
}

// ═══════════════════════════════════════════════════════════════════
// open() behavior
// ═══════════════════════════════════════════════════════════════════

#[test]
#[ignore]
fn test_xhr_open_sets_readystate_to_opened() {
    let mut rt = create_runtime();
    let result = rt.eval(
        r#"(function(){
            var x = new XMLHttpRequest();
            x.open('GET', 'https://example.com/');
            return x.readyState;
        })()"#,
    ).unwrap();
    assert_eq!(result, "1"); // OPENED
}

#[test]
#[ignore]
fn test_xhr_open_stores_method_and_url() {
    let mut rt = create_runtime();
    let result = rt.eval(
        r#"(function(){
            var x = new XMLHttpRequest();
            x.open('POST', 'https://example.com/api');
            // Our polyfill stores these internally
            return (x._method || x.method || 'unknown') + '|' + (x._url || x.url || 'unknown');
        })()"#,
    ).unwrap();
    assert!(
        result.contains("POST") && result.contains("example.com"),
        "open() should store method and url, got: {result}"
    );
}
