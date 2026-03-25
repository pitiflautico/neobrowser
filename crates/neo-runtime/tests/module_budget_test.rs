//! Module loader budget integration tests.
//!
//! Verify that module loading and fetch budget are independent —
//! loading modules should not exhaust the fetch budget used by
//! application code (fetch(), XHR, etc.).
//!
//! All tests are #[ignore] because they need V8 (deno_core compiled).
//! Run with: cargo test -p neo-runtime -- --ignored module_budget

use neo_runtime::v8::DenoRuntime;
use neo_runtime::{JsRuntime, RuntimeConfig};

fn create_runtime() -> DenoRuntime {
    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.set_document_html("<html><body></body></html>", "https://example.com")
        .unwrap();
    rt
}

// ═══════════════════════════════════════════════════════════════════
// Budget survival after module loading
// ═══════════════════════════════════════════════════════════════════

#[test]
#[ignore]
fn test_fetch_exists_after_module_loading() {
    let mut rt = create_runtime();

    rt.insert_module("https://example.com/a.js", "export const x = 1;");
    rt.insert_module(
        "https://example.com/b.js",
        r#"import {x} from "https://example.com/a.js"; export const y = x + 1;"#,
    );
    rt.load_module("https://example.com/b.js").unwrap();

    let result = rt.eval("typeof fetch").unwrap();
    assert_eq!(result, "function", "fetch should still exist after module loading");
}

#[test]
#[ignore]
fn test_xhr_exists_after_module_loading() {
    let mut rt = create_runtime();

    rt.insert_module("https://example.com/a.js", "export const x = 1;");
    rt.load_module("https://example.com/a.js").unwrap();

    let result = rt.eval("typeof XMLHttpRequest").unwrap();
    assert_eq!(
        result, "function",
        "XMLHttpRequest should still exist after module loading"
    );
}

#[test]
#[ignore]
fn test_fetch_callable_after_many_modules() {
    let mut rt = create_runtime();

    // Load 10 modules — none should exhaust a fetch budget
    for i in 0..10 {
        rt.insert_module(
            &format!("https://example.com/mod{i}.js"),
            &format!("export const val{i} = {i};"),
        );
    }
    for i in 0..10 {
        rt.load_module(&format!("https://example.com/mod{i}.js"))
            .unwrap();
    }

    // fetch should still be callable (even though it will fail on network)
    let result = rt.eval("typeof fetch").unwrap();
    assert_eq!(result, "function");

    // Attempting a fetch should not throw "budget exceeded"
    // It will fail with a network error (no real server), but NOT a budget error
    let result = rt
        .eval(
            r#"(function(){
            try {
                fetch('https://example.com/api');
                return 'initiated';
            } catch(e) {
                return 'error:' + e.message;
            }
        })()"#,
        )
        .unwrap();
    assert!(
        !result.contains("budget"),
        "fetch should not fail with budget error after module loading: {result}"
    );
}

// ═══════════════════════════════════════════════════════════════════
// Budget reset
// ═══════════════════════════════════════════════════════════════════

#[test]
#[ignore]
fn test_reset_budgets_does_not_break_fetch() {
    let mut rt = create_runtime();

    rt.insert_module("https://example.com/a.js", "export const x = 1;");
    rt.load_module("https://example.com/a.js").unwrap();

    // Reset budgets (normally called between script exec and settle)
    rt.reset_budgets();

    let result = rt.eval("typeof fetch").unwrap();
    assert_eq!(result, "function", "fetch should survive budget reset");
}

#[test]
#[ignore]
fn test_module_loading_after_reset_budgets() {
    let mut rt = create_runtime();

    rt.reset_budgets();

    rt.insert_module(
        "https://example.com/late.js",
        "globalThis.__late_loaded = true; export const z = 99;",
    );
    rt.load_module("https://example.com/late.js").unwrap();

    let result = rt.eval("String(globalThis.__late_loaded)").unwrap();
    assert_eq!(result, "true", "Module should load after budget reset");
}

// ═══════════════════════════════════════════════════════════════════
// Module chain + budget
// ═══════════════════════════════════════════════════════════════════

#[test]
#[ignore]
fn test_deep_import_chain_does_not_exhaust_budget() {
    let mut rt = create_runtime();

    // Create a 5-deep import chain: e -> d -> c -> b -> a
    rt.insert_module("https://example.com/chain/a.js", "export const a = 1;");
    rt.insert_module(
        "https://example.com/chain/b.js",
        r#"import {a} from "https://example.com/chain/a.js"; export const b = a + 1;"#,
    );
    rt.insert_module(
        "https://example.com/chain/c.js",
        r#"import {b} from "https://example.com/chain/b.js"; export const c = b + 1;"#,
    );
    rt.insert_module(
        "https://example.com/chain/d.js",
        r#"import {c} from "https://example.com/chain/c.js"; export const d = c + 1;"#,
    );
    rt.insert_module(
        "https://example.com/chain/e.js",
        r#"import {d} from "https://example.com/chain/d.js"; globalThis.__chain_val = d + 1; export const e = d + 1;"#,
    );

    rt.load_module("https://example.com/chain/e.js").unwrap();

    let val = rt.eval("String(globalThis.__chain_val)").unwrap();
    assert_eq!(val, "5", "Chain should produce 5, got: {val}");

    // fetch should still work
    let result = rt.eval("typeof fetch").unwrap();
    assert_eq!(result, "function");
}
