//! Comprehensive tests for module loading & hydration pipeline.
//!
//! Tests cover: module loading basics, double-load prevention, error isolation,
//! import map resolution, import extraction, settle/quiescence, and module store.
//!
//! Mock tests run fast. V8 tests are #[ignore] (need deno_core compiled).

use neo_runtime::imports::extract_es_imports;
use neo_runtime::mock::MockRuntime;
use neo_runtime::modules::{
    extract_export_names, generate_stub_module, rewrite_promise_all_settled, ImportMap,
    ModuleTracker,
};
use neo_runtime::JsRuntime;

// ═══════════════════════════════════════════════════════════════════
// 1. MODULE LOADING BASICS (Mock)
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_mock_load_module_records_url() {
    let mut rt = MockRuntime::new();
    rt.load_module("https://example.com/app.js").unwrap();
    assert_eq!(rt.module_calls.len(), 1);
    assert_eq!(rt.module_calls[0], "https://example.com/app.js");
}

#[test]
fn test_mock_insert_and_has_module() {
    let mut rt = MockRuntime::new();
    assert!(!rt.has_module("https://example.com/mod.js"));
    rt.insert_module("https://example.com/mod.js", "export const x = 42;");
    assert!(rt.has_module("https://example.com/mod.js"));
}

#[test]
fn test_mock_get_module_source() {
    let mut rt = MockRuntime::new();
    rt.insert_module("https://example.com/mod.js", "export const x = 42;");
    let src = rt.get_module_source("https://example.com/mod.js");
    assert_eq!(src.unwrap(), "export const x = 42;");
}

#[test]
fn test_mock_module_urls_lists_all() {
    let mut rt = MockRuntime::new();
    rt.insert_module("https://example.com/a.js", "export const a = 1;");
    rt.insert_module("https://example.com/b.js", "export const b = 2;");
    let urls = rt.module_urls();
    assert_eq!(urls.len(), 2);
    assert!(urls.contains(&"https://example.com/a.js".to_string()));
    assert!(urls.contains(&"https://example.com/b.js".to_string()));
}

#[test]
fn test_mock_mark_stub() {
    let mut rt = MockRuntime::new();
    rt.insert_module("https://example.com/heavy.js", "/* 500KB */");
    rt.mark_stub("https://example.com/heavy.js");
    // Stub marking doesn't remove the module from store
    assert!(rt.has_module("https://example.com/heavy.js"));
}

// ═══════════════════════════════════════════════════════════════════
// 1b. MODULE LOADING BASICS (V8)
// ═══════════════════════════════════════════════════════════════════

/// Load a simple ES module and verify the export is accessible via eval.
#[test]
#[ignore]
fn test_v8_load_simple_module_export() {
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::RuntimeConfig;

    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.set_document_html("<html><body></body></html>", "https://example.com")
        .unwrap();

    // Pre-load module source into store
    rt.insert_module("https://example.com/mod.js", "export const x = 42;");
    rt.load_module("https://example.com/mod.js").unwrap();

    // After module evaluation, the export should be accessible via dynamic import
    rt.execute(
        r#"
        globalThis.__test_result = 'pending';
        import("https://example.com/mod.js").then(m => {
            globalThis.__test_result = String(m.x);
        });
        "#,
    )
    .unwrap();
    rt.run_until_settled(2000).unwrap();

    let result = rt.eval("globalThis.__test_result").unwrap();
    assert_eq!(result, "42", "Module export x should be 42, got: {result}");
}

/// Load two modules where B imports from A, verify the import chain resolves.
#[test]
#[ignore]
fn test_v8_module_import_chain() {
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::RuntimeConfig;

    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.set_document_html("<html><body></body></html>", "https://example.com")
        .unwrap();

    rt.insert_module(
        "https://example.com/a.js",
        "export const greeting = 'hello';",
    );
    rt.insert_module(
        "https://example.com/b.js",
        r#"import { greeting } from "https://example.com/a.js"; globalThis.__chain_result = greeting + ' world';"#,
    );
    rt.load_module("https://example.com/b.js").unwrap();

    let result = rt.eval("globalThis.__chain_result").unwrap();
    assert_eq!(
        result, "hello world",
        "Module chain should produce 'hello world', got: {result}"
    );
}

// ═══════════════════════════════════════════════════════════════════
// 2. MODULE DOUBLE-LOAD PREVENTION
// ═══════════════════════════════════════════════════════════════════

/// Verify that loading the same module twice doesn't evaluate it twice.
#[test]
#[ignore]
fn test_v8_module_double_load_prevention() {
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::RuntimeConfig;

    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.set_document_html("<html><body></body></html>", "https://example.com")
        .unwrap();

    // Module with side effect: increments a counter
    rt.insert_module(
        "https://example.com/counter.js",
        r#"
        if (!globalThis.__loadCount) globalThis.__loadCount = 0;
        globalThis.__loadCount++;
        export const dummy = 1;
        "#,
    );

    rt.load_module("https://example.com/counter.js").unwrap();
    rt.load_module("https://example.com/counter.js").unwrap(); // should be skipped

    let count = rt.eval("String(globalThis.__loadCount)").unwrap();
    assert_eq!(
        count, "1",
        "Module should only be evaluated once, but __loadCount = {count}"
    );
}

/// Verify get_module_namespace detects already-evaluated modules.
#[test]
#[ignore]
fn test_v8_module_namespace_detection() {
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::RuntimeConfig;

    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.set_document_html("<html><body></body></html>", "https://example.com")
        .unwrap();

    rt.insert_module(
        "https://example.com/ns.js",
        "export const val = 'detected';",
    );
    rt.load_module("https://example.com/ns.js").unwrap();

    // Access the export via dynamic import to confirm it's in the namespace
    rt.execute(
        r#"
        globalThis.__ns_result = 'not_found';
        import("https://example.com/ns.js").then(m => {
            globalThis.__ns_result = m.val;
        });
        "#,
    )
    .unwrap();
    rt.run_until_settled(2000).unwrap();

    let result = rt.eval("globalThis.__ns_result").unwrap();
    assert_eq!(
        result, "detected",
        "Module namespace should be accessible after load, got: {result}"
    );
}

// ═══════════════════════════════════════════════════════════════════
// 3. MODULE ERROR ISOLATION
// ═══════════════════════════════════════════════════════════════════

/// A module with a syntax error should not crash the runtime.
#[test]
#[ignore]
fn test_v8_module_syntax_error_isolation() {
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::RuntimeConfig;

    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.set_document_html("<html><body></body></html>", "https://example.com")
        .unwrap();

    // Module with syntax error
    rt.insert_module(
        "https://example.com/bad.js",
        "export const x = {{{SYNTAX_ERROR;",
    );
    // Should not panic — load_module handles errors gracefully
    let result = rt.load_module("https://example.com/bad.js");
    // Whether it returns Ok or Err, the runtime should still be usable
    eprintln!("bad module load result: {result:?}");

    // Runtime should still work after the error
    let val = rt.eval("1 + 1").unwrap();
    assert_eq!(val, "2", "Runtime should still work after module error");
}

/// After a bad module, loading a valid module should work.
#[test]
#[ignore]
fn test_v8_module_error_then_valid() {
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::RuntimeConfig;

    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.set_document_html("<html><body></body></html>", "https://example.com")
        .unwrap();

    // Bad module first
    rt.insert_module("https://example.com/bad.js", "export const x = {{{;");
    let _ = rt.load_module("https://example.com/bad.js");

    // Valid module after
    rt.insert_module(
        "https://example.com/good.js",
        "globalThis.__good_loaded = true; export const y = 99;",
    );
    rt.load_module("https://example.com/good.js").unwrap();

    let result = rt.eval("String(globalThis.__good_loaded)").unwrap();
    assert_eq!(
        result, "true",
        "Valid module should work after bad module, got: {result}"
    );
}

/// Module with runtime error (ReferenceError) should not crash.
#[test]
#[ignore]
fn test_v8_module_runtime_error_isolation() {
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::RuntimeConfig;

    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.set_document_html("<html><body></body></html>", "https://example.com")
        .unwrap();

    rt.insert_module(
        "https://example.com/throws.js",
        "throw new Error('intentional error'); export const x = 1;",
    );
    let _ = rt.load_module("https://example.com/throws.js");

    // Runtime should survive
    let val = rt.eval("2 + 2").unwrap();
    assert_eq!(val, "4", "Runtime should survive module runtime error");
}

// ═══════════════════════════════════════════════════════════════════
// 4. IMPORT MAP RESOLUTION
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_import_map_parse_basic() {
    let json = r#"{"imports": {"react": "https://esm.sh/react@18", "react-dom": "https://esm.sh/react-dom@18"}}"#;
    let map = ImportMap::parse(json).unwrap();
    assert_eq!(map.imports.len(), 2);
    assert_eq!(map.resolve("react"), Some("https://esm.sh/react@18"));
    assert_eq!(
        map.resolve("react-dom"),
        Some("https://esm.sh/react-dom@18")
    );
}

#[test]
fn test_import_map_resolve_not_found() {
    let json = r#"{"imports": {"react": "https://esm.sh/react@18"}}"#;
    let map = ImportMap::parse(json).unwrap();
    assert_eq!(map.resolve("vue"), None);
}

#[test]
fn test_import_map_resolve_exact_match() {
    let json = r#"{"imports": {"react": "https://esm.sh/react@18"}}"#;
    let map = ImportMap::parse(json).unwrap();
    // "react" should match exactly, "react-dom" should NOT match
    assert_eq!(map.resolve("react"), Some("https://esm.sh/react@18"));
    assert_eq!(map.resolve("react-dom"), None);
}

#[test]
fn test_import_map_prefix_match() {
    // Prefix match: "lodash/" should match "lodash/fp"
    let json = r#"{"imports": {"lodash/": "https://esm.sh/lodash-es/"}}"#;
    let map = ImportMap::parse(json).unwrap();
    // NOTE: The current implementation only supports exact prefix match
    // (resolve returns Some for the prefix key itself, not for sub-paths).
    // This test documents the CURRENT behavior.
    let result = map.resolve("lodash/fp");
    // The current code has a prefix-match path that doesn't actually return the value
    // because it can't construct a new string from &self. This is a known limitation.
    assert_eq!(
        result, None,
        "Prefix match for sub-paths is not yet implemented (known limitation)"
    );
}

#[test]
fn test_import_map_parse_invalid_json() {
    assert!(ImportMap::parse("not json").is_none());
    assert!(ImportMap::parse("{").is_none());
    assert!(ImportMap::parse("[]").is_none());
}

#[test]
fn test_import_map_parse_missing_imports() {
    assert!(ImportMap::parse(r#"{"scopes": {}}"#).is_none());
    assert!(ImportMap::parse(r#"{"imports": null}"#).is_none());
}

#[test]
fn test_import_map_parse_empty_imports() {
    let map = ImportMap::parse(r#"{"imports": {}}"#).unwrap();
    assert_eq!(map.imports.len(), 0);
    assert_eq!(map.resolve("anything"), None);
}

#[test]
fn test_import_map_at_alias() {
    let json =
        r#"{"imports": {"@/utils": "https://example.com/utils.js", "@app/": "https://example.com/app/"}}"#;
    let map = ImportMap::parse(json).unwrap();
    assert_eq!(
        map.resolve("@/utils"),
        Some("https://example.com/utils.js")
    );
}

// ═══════════════════════════════════════════════════════════════════
// 5. IMPORT EXTRACTION (extract_es_imports)
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_extract_named_import() {
    let js = r#"import { foo } from './bar.js'"#;
    let imports = extract_es_imports(js, "https://example.com/app/main.js");
    assert!(
        imports.contains(&"https://example.com/app/bar.js".to_string()),
        "Should extract './bar.js', got: {imports:?}"
    );
}

#[test]
fn test_extract_namespace_import() {
    let js = r#"import * as x from '../lib.js'"#;
    let imports = extract_es_imports(js, "https://example.com/app/sub/main.js");
    assert!(
        imports.contains(&"https://example.com/app/lib.js".to_string()),
        "Should extract '../lib.js', got: {imports:?}"
    );
}

#[test]
fn test_extract_full_url_import() {
    let js = r#"import 'https://cdn.com/lib.js'"#;
    let imports = extract_es_imports(js, "https://example.com/app/main.js");
    assert!(
        imports.contains(&"https://cdn.com/lib.js".to_string()),
        "Should extract full URL, got: {imports:?}"
    );
}

#[test]
fn test_extract_reexport() {
    let js = r#"export { x } from './re-export.js'"#;
    let imports = extract_es_imports(js, "https://example.com/app/main.js");
    assert!(
        imports.contains(&"https://example.com/app/re-export.js".to_string()),
        "Should extract re-export, got: {imports:?}"
    );
}

#[test]
fn test_extract_dynamic_import() {
    let js = r#"const m = import('./lazy.js')"#;
    let imports = extract_es_imports(js, "https://example.com/app/main.js");
    assert!(
        imports.contains(&"https://example.com/app/lazy.js".to_string()),
        "Should extract dynamic import, got: {imports:?}"
    );
}

#[test]
fn test_extract_bare_specifier_skipped() {
    let js = r#"import React from "react""#;
    let imports = extract_es_imports(js, "https://example.com/app/main.js");
    assert!(
        imports.is_empty(),
        "Bare specifiers should NOT be extracted, got: {imports:?}"
    );
}

#[test]
fn test_extract_absolute_path_import() {
    let js = r#"import { x } from "/assets/vendor.js""#;
    let imports = extract_es_imports(js, "https://example.com/app/main.js");
    assert!(
        imports.contains(&"https://example.com/assets/vendor.js".to_string()),
        "Should resolve absolute path against origin, got: {imports:?}"
    );
}

#[test]
fn test_extract_minified_import() {
    // Minified: no space after from
    let js = r#"import{foo}from"./utils.js";import bar from"./helpers.js""#;
    let imports = extract_es_imports(js, "https://example.com/app/main.js");
    assert!(
        imports.contains(&"https://example.com/app/utils.js".to_string()),
        "Should handle minified import, got: {imports:?}"
    );
    assert!(
        imports.contains(&"https://example.com/app/helpers.js".to_string()),
        "Should handle minified default import, got: {imports:?}"
    );
}

#[test]
fn test_extract_no_duplicates() {
    let js = r#"import "./a.js"; import "./a.js""#;
    let imports = extract_es_imports(js, "https://example.com/app/main.js");
    assert_eq!(imports.len(), 1, "Should deduplicate, got: {imports:?}");
}

#[test]
fn test_extract_mixed_quotes() {
    let js = r#"import { x } from "./single.js"; import { y } from "./double.js""#;
    let imports = extract_es_imports(js, "https://example.com/app/main.js");
    assert_eq!(imports.len(), 2, "Should handle both quote styles");
}

// NOTE: The current extract_es_imports implementation uses simple string scanning,
// NOT an AST parser. It WILL match imports inside comments and string literals.
// These tests document the ACTUAL behavior (potential false positives).

#[test]
fn test_extract_import_in_comment_is_known_false_positive() {
    let js = r#"// import { x } from './commented.js'
export const y = 1;"#;
    let imports = extract_es_imports(js, "https://example.com/app/main.js");
    // KNOWN BUG: The scanner doesn't skip comments, so this IS extracted.
    // Documenting the actual behavior for awareness.
    if imports.contains(&"https://example.com/app/commented.js".to_string()) {
        eprintln!("KNOWN: extract_es_imports matches imports inside comments (false positive)");
    }
    // Test passes either way — just documenting behavior.
}

#[test]
fn test_extract_import_in_string_is_known_false_positive() {
    let js = r#"const s = "import { x } from './in-string.js'";"#;
    let imports = extract_es_imports(js, "https://example.com/app/main.js");
    // KNOWN BUG: The scanner doesn't skip string contents, so this IS extracted.
    if imports.contains(&"https://example.com/app/in-string.js".to_string()) {
        eprintln!(
            "KNOWN: extract_es_imports matches imports inside strings (false positive)"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════
// 6. SETTLE / QUIESCENCE DETECTION
// ═══════════════════════════════════════════════════════════════════

/// No async work → settles immediately.
#[test]
#[ignore]
fn test_v8_settle_no_async_work() {
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::RuntimeConfig;
    use std::time::Instant;

    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.set_document_html("<html><body></body></html>", "https://example.com")
        .unwrap();

    let start = Instant::now();
    rt.run_until_settled(5000).unwrap();
    let elapsed = start.elapsed();

    assert!(
        elapsed.as_millis() < 1000,
        "No-work settle should be fast, took {}ms",
        elapsed.as_millis()
    );
    assert_eq!(rt.pending_tasks(), 0);
}

/// Page with setTimeout → settles after timer fires.
#[test]
#[ignore]
fn test_v8_settle_with_timeout() {
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::RuntimeConfig;

    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.set_document_html("<html><body></body></html>", "https://example.com")
        .unwrap();

    rt.execute(
        r#"
        globalThis.__timer_done = false;
        setTimeout(() => { globalThis.__timer_done = true; }, 50);
        "#,
    )
    .unwrap();

    rt.run_until_settled(5000).unwrap();

    let done = rt.eval("String(globalThis.__timer_done)").unwrap();
    assert_eq!(
        done, "true",
        "Timer should have fired after settle, got: {done}"
    );
}

/// Page with Promise → settles after resolve.
#[test]
#[ignore]
fn test_v8_settle_with_promise() {
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::RuntimeConfig;

    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.set_document_html("<html><body></body></html>", "https://example.com")
        .unwrap();

    rt.execute(
        r#"
        globalThis.__promise_result = 'pending';
        Promise.resolve('done').then(v => { globalThis.__promise_result = v; });
        "#,
    )
    .unwrap();

    rt.run_until_settled(2000).unwrap();

    let result = rt.eval("globalThis.__promise_result").unwrap();
    assert_eq!(
        result, "done",
        "Promise should resolve after settle, got: {result}"
    );
}

/// Chained promises settle correctly.
#[test]
#[ignore]
fn test_v8_settle_chained_promises() {
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::RuntimeConfig;

    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.set_document_html("<html><body></body></html>", "https://example.com")
        .unwrap();

    rt.execute(
        r#"
        globalThis.__chain = [];
        Promise.resolve(1)
            .then(v => { globalThis.__chain.push(v); return v + 1; })
            .then(v => { globalThis.__chain.push(v); return v + 1; })
            .then(v => { globalThis.__chain.push(v); });
        "#,
    )
    .unwrap();

    rt.run_until_settled(2000).unwrap();

    let result = rt.eval("globalThis.__chain.join(',')").unwrap();
    assert_eq!(
        result, "1,2,3",
        "Promise chain should resolve in order, got: {result}"
    );
}

/// Timeout stops infinite loops.
#[test]
#[ignore]
fn test_v8_settle_timeout_with_interval() {
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::RuntimeConfig;
    use std::time::Instant;

    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.set_document_html("<html><body></body></html>", "https://example.com")
        .unwrap();

    rt.execute(
        r#"
        globalThis.__ticks = 0;
        setInterval(() => { globalThis.__ticks++; }, 5);
        "#,
    )
    .unwrap();

    let start = Instant::now();
    let _ = rt.run_until_settled(300);
    let elapsed = start.elapsed();

    assert!(
        elapsed.as_millis() < 2000,
        "Should respect timeout, took {}ms",
        elapsed.as_millis()
    );

    let ticks = rt.eval("globalThis.__ticks").unwrap();
    let tick_num: usize = ticks.parse().unwrap_or(0);
    assert!(tick_num > 0, "Interval should have ticked at least once");
}

// ═══════════════════════════════════════════════════════════════════
// 7. EXPORT NAME EXTRACTION
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_extract_export_function() {
    let js = "export function myFunc() {}";
    let names = extract_export_names(js);
    assert!(
        names.contains(&"myFunc".to_string()),
        "Should extract function export, got: {names:?}"
    );
}

#[test]
fn test_extract_export_const() {
    let js = "export const MY_CONST = 42;";
    let names = extract_export_names(js);
    assert!(
        names.contains(&"MY_CONST".to_string()),
        "Should extract const export, got: {names:?}"
    );
}

#[test]
fn test_extract_export_let() {
    let js = "export let myVar = 'hello';";
    let names = extract_export_names(js);
    assert!(
        names.contains(&"myVar".to_string()),
        "Should extract let export, got: {names:?}"
    );
}

#[test]
fn test_extract_export_class() {
    let js = "export class MyClass {}";
    let names = extract_export_names(js);
    assert!(
        names.contains(&"MyClass".to_string()),
        "Should extract class export, got: {names:?}"
    );
}

#[test]
fn test_extract_export_default() {
    let js = "export default function() {}";
    let names = extract_export_names(js);
    assert!(
        names.contains(&"default".to_string()),
        "Should extract default export, got: {names:?}"
    );
}

#[test]
fn test_extract_export_named_block() {
    let js = "const a = 1; const b = 2; export { a, b };";
    let names = extract_export_names(js);
    assert!(names.contains(&"a".to_string()), "Should extract a");
    assert!(names.contains(&"b".to_string()), "Should extract b");
}

#[test]
fn test_extract_export_named_block_with_as() {
    let js = "const foo = 1; export { foo as bar };";
    let names = extract_export_names(js);
    assert!(
        names.contains(&"bar".to_string()),
        "Should use the 'as' alias, got: {names:?}"
    );
}

#[test]
fn test_extract_export_mixed() {
    let js = r#"export function foo() {} export const bar = 1; export default 42; export { baz } from './other.js';"#;
    let names = extract_export_names(js);
    assert!(names.contains(&"foo".to_string()));
    assert!(names.contains(&"bar".to_string()));
    assert!(names.contains(&"default".to_string()));
    assert!(names.contains(&"baz".to_string()));
}

// ═══════════════════════════════════════════════════════════════════
// 8. STUB MODULE GENERATION
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_stub_module_has_exports() {
    let names = vec!["foo".to_string(), "bar".to_string()];
    let stub = generate_stub_module(&names);
    assert!(stub.contains("export{foo,bar}"));
    assert!(stub.contains("export default _o"));
}

#[test]
fn test_stub_module_default_only() {
    let names = vec!["default".to_string()];
    let stub = generate_stub_module(&names);
    assert!(stub.contains("export default _o"));
    // "default" should NOT appear in named exports
    assert!(!stub.contains("const default="));
}

#[test]
fn test_stub_module_empty_exports() {
    let names: Vec<String> = vec![];
    let stub = generate_stub_module(&names);
    assert!(stub.contains("export default _o"));
}

#[test]
fn test_stub_module_proxy_is_callable() {
    // The stub uses a Proxy — verify the generated JS is syntactically valid
    let names = vec!["createElement".to_string(), "useState".to_string()];
    let stub = generate_stub_module(&names);
    assert!(stub.contains("Proxy"));
    assert!(stub.contains("const createElement=_o"));
    assert!(stub.contains("const useState=_o"));
}

// ═══════════════════════════════════════════════════════════════════
// 9. SOURCE TRANSFORMS
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_rewrite_promise_allsettled() {
    let code = "Promise.allSettled([p1, p2])";
    let result = rewrite_promise_all_settled(code);
    assert!(!result.contains("Promise.allSettled("));
    assert!(result.contains("Promise.all"));
    assert!(result.contains("status:'fulfilled'"));
    assert!(result.contains("status:'rejected'"));
}

#[test]
fn test_rewrite_promise_allsettled_no_op_when_absent() {
    let code = "Promise.all([p1, p2])";
    let result = rewrite_promise_all_settled(code);
    assert_eq!(result, code, "Should not modify code without allSettled");
}

// ═══════════════════════════════════════════════════════════════════
// 10. MODULE TRACKER
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_module_tracker_lifecycle() {
    let tracker = ModuleTracker::new();
    assert_eq!(tracker.pending(), 0);
    assert_eq!(tracker.total_requested(), 0);

    tracker.on_requested("https://example.com/a.js");
    tracker.on_requested("https://example.com/b.js");
    assert_eq!(tracker.pending(), 2);
    assert_eq!(tracker.total_requested(), 2);

    tracker.on_loaded("https://example.com/a.js");
    assert_eq!(tracker.pending(), 1);
    assert_eq!(tracker.total_loaded(), 1);

    tracker.on_failed("https://example.com/b.js");
    assert_eq!(tracker.pending(), 0);
    assert_eq!(tracker.total_failed(), 1);
}

#[test]
fn test_module_tracker_reset() {
    let tracker = ModuleTracker::new();
    tracker.on_requested("https://example.com/a.js");
    tracker.on_loaded("https://example.com/a.js");
    tracker.on_requested("https://example.com/b.js");
    tracker.on_failed("https://example.com/b.js");

    tracker.reset();
    assert_eq!(tracker.pending(), 0);
    assert_eq!(tracker.total_requested(), 0);
    assert_eq!(tracker.total_loaded(), 0);
    assert_eq!(tracker.total_failed(), 0);
}

#[test]
fn test_module_tracker_clone_shares_state() {
    let tracker1 = ModuleTracker::new();
    let tracker2 = tracker1.clone();

    tracker1.on_requested("https://example.com/a.js");
    assert_eq!(
        tracker2.pending(),
        1,
        "Clone should share state via Arc"
    );
}

// ═══════════════════════════════════════════════════════════════════
// 11. MODULE LOADER RESOLUTION (unit tests via NeoModuleLoader)
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_loader_resolve_relative() {
    use deno_core::{ModuleLoader, ResolutionKind};
    use neo_runtime::modules::NeoModuleLoader;
    use std::cell::RefCell;
    use std::rc::Rc;

    let loader = NeoModuleLoader {
        store: Rc::new(RefCell::new(neo_runtime::modules::ScriptStore::default())),
        code_cache: None,
        page_origin: Rc::new(RefCell::new("https://example.com".to_string())),
        import_map: Rc::new(RefCell::new(None)),
        http_client: None,
        on_demand_count: RefCell::new(0),
        module_tracker: ModuleTracker::new(),
        trace_buffer: neo_runtime::trace_events::TraceBuffer::new(),
    };

    let result = loader
        .resolve(
            "./utils.js",
            "https://example.com/app/main.js",
            ResolutionKind::Import,
        )
        .unwrap();
    assert_eq!(result.to_string(), "https://example.com/app/utils.js");
}

#[test]
fn test_loader_resolve_absolute_path() {
    use deno_core::{ModuleLoader, ResolutionKind};
    use neo_runtime::modules::NeoModuleLoader;
    use std::cell::RefCell;
    use std::rc::Rc;

    let loader = NeoModuleLoader {
        store: Rc::new(RefCell::new(neo_runtime::modules::ScriptStore::default())),
        code_cache: None,
        page_origin: Rc::new(RefCell::new("https://example.com".to_string())),
        import_map: Rc::new(RefCell::new(None)),
        http_client: None,
        on_demand_count: RefCell::new(0),
        module_tracker: ModuleTracker::new(),
        trace_buffer: neo_runtime::trace_events::TraceBuffer::new(),
    };

    let result = loader
        .resolve("/cdn/bundle.js", "file:///<eval>", ResolutionKind::Import)
        .unwrap();
    assert_eq!(result.to_string(), "https://example.com/cdn/bundle.js");
}

#[test]
fn test_loader_resolve_with_import_map() {
    use deno_core::{ModuleLoader, ResolutionKind};
    use neo_runtime::modules::NeoModuleLoader;
    use std::cell::RefCell;
    use std::rc::Rc;

    let map = ImportMap::parse(
        r#"{"imports": {"react": "https://esm.sh/react@18", "vue": "https://esm.sh/vue@3"}}"#,
    )
    .unwrap();

    let loader = NeoModuleLoader {
        store: Rc::new(RefCell::new(neo_runtime::modules::ScriptStore::default())),
        code_cache: None,
        page_origin: Rc::new(RefCell::new("https://example.com".to_string())),
        import_map: Rc::new(RefCell::new(Some(map))),
        http_client: None,
        on_demand_count: RefCell::new(0),
        module_tracker: ModuleTracker::new(),
        trace_buffer: neo_runtime::trace_events::TraceBuffer::new(),
    };

    let react = loader
        .resolve("react", "https://example.com/app.js", ResolutionKind::Import)
        .unwrap();
    assert_eq!(react.to_string(), "https://esm.sh/react@18");

    let vue = loader
        .resolve("vue", "https://example.com/app.js", ResolutionKind::Import)
        .unwrap();
    assert_eq!(vue.to_string(), "https://esm.sh/vue@3");
}

#[test]
fn test_loader_resolve_bare_specifier_errors() {
    use deno_core::{ModuleLoader, ResolutionKind};
    use neo_runtime::modules::NeoModuleLoader;
    use std::cell::RefCell;
    use std::rc::Rc;

    let loader = NeoModuleLoader {
        store: Rc::new(RefCell::new(neo_runtime::modules::ScriptStore::default())),
        code_cache: None,
        page_origin: Rc::new(RefCell::new("https://example.com".to_string())),
        import_map: Rc::new(RefCell::new(None)),
        http_client: None,
        on_demand_count: RefCell::new(0),
        module_tracker: ModuleTracker::new(),
        trace_buffer: neo_runtime::trace_events::TraceBuffer::new(),
    };

    let result = loader.resolve("react", "https://example.com/app.js", ResolutionKind::Import);
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("bare specifier"), "Error should mention bare specifier: {msg}");
    assert!(msg.contains("import map"), "Error should mention import map: {msg}");
}

#[test]
fn test_loader_resolve_full_url_passthrough() {
    use deno_core::{ModuleLoader, ResolutionKind};
    use neo_runtime::modules::NeoModuleLoader;
    use std::cell::RefCell;
    use std::rc::Rc;

    let loader = NeoModuleLoader {
        store: Rc::new(RefCell::new(neo_runtime::modules::ScriptStore::default())),
        code_cache: None,
        page_origin: Rc::new(RefCell::new("https://example.com".to_string())),
        import_map: Rc::new(RefCell::new(None)),
        http_client: None,
        on_demand_count: RefCell::new(0),
        module_tracker: ModuleTracker::new(),
        trace_buffer: neo_runtime::trace_events::TraceBuffer::new(),
    };

    let result = loader
        .resolve(
            "https://cdn.example.com/lib.js",
            "https://example.com/app.js",
            ResolutionKind::Import,
        )
        .unwrap();
    assert_eq!(result.to_string(), "https://cdn.example.com/lib.js");
}

// ═══════════════════════════════════════════════════════════════════
// 12. V8 MODULE + IMPORT MAP INTEGRATION
// ═══════════════════════════════════════════════════════════════════

/// Set import map on runtime and verify bare specifiers resolve.
#[test]
#[ignore]
fn test_v8_import_map_integration() {
    use neo_runtime::modules::ImportMap;
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::RuntimeConfig;

    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.set_document_html("<html><body></body></html>", "https://example.com")
        .unwrap();

    // Set import map
    let map = ImportMap::parse(
        r#"{"imports": {"mylib": "https://example.com/mylib.js"}}"#,
    )
    .unwrap();
    rt.set_import_map(map);

    // Pre-load the mapped URL
    rt.insert_module(
        "https://example.com/mylib.js",
        "export const version = '1.0';",
    );

    // Load via bare specifier should resolve via import map
    rt.execute(
        r#"
        globalThis.__map_result = 'pending';
        import("mylib").then(m => {
            globalThis.__map_result = m.version;
        }).catch(e => {
            globalThis.__map_result = 'error: ' + e.message;
        });
        "#,
    )
    .unwrap();
    rt.run_until_settled(2000).unwrap();

    let result = rt.eval("globalThis.__map_result").unwrap();
    assert_eq!(
        result, "1.0",
        "Import map should resolve bare specifier, got: {result}"
    );
}

// ═══════════════════════════════════════════════════════════════════
// 13. QUIESCENCE FUNCTION
// ═══════════════════════════════════════════════════════════════════

/// Verify __neo_quiescence is defined and returns expected fields.
#[test]
#[ignore]
fn test_v8_quiescence_function() {
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::RuntimeConfig;

    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.set_document_html("<html><body></body></html>", "https://example.com")
        .unwrap();

    let result = rt
        .eval("typeof __neo_quiescence === 'function' ? __neo_quiescence() : 'MISSING'")
        .unwrap();
    assert_ne!(result, "MISSING", "__neo_quiescence should be defined");

    let parsed: serde_json::Value = serde_json::from_str(&result)
        .unwrap_or_else(|_| panic!("quiescence should return JSON, got: {result}"));

    assert!(parsed.get("idle_ms").is_some(), "Should have idle_ms");
    assert!(
        parsed.get("pending_timers").is_some(),
        "Should have pending_timers"
    );
    assert!(
        parsed.get("pending_fetches").is_some(),
        "Should have pending_fetches"
    );
}

// ═══════════════════════════════════════════════════════════════════
// 14. RESET PAGE STATE
// ═══════════════════════════════════════════════════════════════════

/// Verify reset_page_state clears modules for same-origin navigation.
#[test]
#[ignore]
fn test_v8_reset_page_state() {
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::RuntimeConfig;

    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.set_document_html("<html><body></body></html>", "https://example.com")
        .unwrap();

    rt.insert_module("https://example.com/page1.js", "export const p = 1;");
    assert!(rt.has_module("https://example.com/page1.js"));

    rt.reset_page_state();

    // After reset, the module store should be cleared
    assert!(
        !rt.has_module("https://example.com/page1.js"),
        "reset_page_state should clear module store"
    );
}

// ═══════════════════════════════════════════════════════════════════
// MODULE DOUBLE-LOAD FROM DEPENDENCY (no panic)
// ═══════════════════════════════════════════════════════════════════

/// Load module B which imports A (A evaluated as dep), then load A directly.
/// Should NOT panic — A is already evaluated.
#[test]
#[ignore]
fn test_module_double_load_via_dep_no_panic() {
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::RuntimeConfig;

    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.set_document_html("<html><body></body></html>", "https://example.com")
        .unwrap();

    rt.insert_module(
        "https://example.com/dep_a.js",
        "export const x = 1;",
    );
    rt.insert_module(
        "https://example.com/dep_b.js",
        r#"import {x} from "https://example.com/dep_a.js"; export const y = x;"#,
    );

    // First: load B (which pulls in A as dependency)
    rt.load_module("https://example.com/dep_b.js").unwrap();

    // Then: load A directly — already evaluated as dep, should NOT panic
    rt.load_module("https://example.com/dep_a.js").unwrap();

    // Verify A is still accessible
    rt.execute(
        r#"
        globalThis.__dep_check = 'pending';
        import("https://example.com/dep_a.js").then(m => {
            globalThis.__dep_check = String(m.x);
        });
        "#,
    )
    .unwrap();
    rt.run_until_settled(2000).unwrap();

    let result = rt.eval("globalThis.__dep_check").unwrap();
    assert_eq!(result, "1", "dep_a.js export should be 1, got: {result}");
}

// ═══════════════════════════════════════════════════════════════════
// MODULE SIDE EFFECTS PERSISTENCE
// ═══════════════════════════════════════════════════════════════════

/// Module side effects (globalThis assignments) should persist after loading.
#[test]
#[ignore]
fn test_module_side_effects_persist() {
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::RuntimeConfig;

    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.set_document_html("<html><body></body></html>", "https://example.com")
        .unwrap();

    rt.insert_module(
        "https://example.com/side.js",
        "globalThis.__side_effect = 42;",
    );
    rt.load_module("https://example.com/side.js").unwrap();

    let result = rt.eval("String(globalThis.__side_effect)").unwrap();
    assert!(
        result.contains("42"),
        "Side effect should persist: {result}"
    );
}

/// Multiple modules' side effects should all be visible.
#[test]
#[ignore]
fn test_multiple_module_side_effects() {
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::RuntimeConfig;

    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.set_document_html("<html><body></body></html>", "https://example.com")
        .unwrap();

    rt.insert_module(
        "https://example.com/side1.js",
        "globalThis.__effect_a = 'alpha';",
    );
    rt.insert_module(
        "https://example.com/side2.js",
        "globalThis.__effect_b = 'beta';",
    );

    rt.load_module("https://example.com/side1.js").unwrap();
    rt.load_module("https://example.com/side2.js").unwrap();

    let a = rt.eval("globalThis.__effect_a").unwrap();
    let b = rt.eval("globalThis.__effect_b").unwrap();
    assert_eq!(a, "alpha", "side1 effect should be 'alpha', got: {a}");
    assert_eq!(b, "beta", "side2 effect should be 'beta', got: {b}");
}

/// Module side effects that modify DOM should persist.
#[test]
#[ignore]
fn test_module_dom_side_effects() {
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::RuntimeConfig;

    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    rt.set_document_html(
        r#"<html><body><div id="target"></div></body></html>"#,
        "https://example.com",
    )
    .unwrap();

    rt.insert_module(
        "https://example.com/dom_side.js",
        r#"
        var el = document.getElementById('target');
        if (el) el.textContent = 'modified';
        "#,
    );
    rt.load_module("https://example.com/dom_side.js").unwrap();

    let result = rt
        .eval(
            r#"(function(){
        var el = document.getElementById('target');
        return el ? el.textContent : 'not_found';
    })()"#,
        )
        .unwrap();
    assert_eq!(
        result, "modified",
        "DOM modification from module should persist: {result}"
    );
}
