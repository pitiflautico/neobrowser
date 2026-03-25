//! R7b-R7c: React hydration support — inline module transform and entry boot.
//!
//! Inline `<script type="module">` tags are transformed to async IIFE scripts
//! to avoid top-level await blocking V8 module evaluation. After execution,
//! the entry module is loaded directly for React Router hydration.

/// R7b: Transform an inline `<script type="module">` into an async IIFE.
///
/// Static imports are rewritten to dynamic `import()` calls. The IIFE is
/// wrapped in try/catch for error resilience. Preserves import order.
pub(crate) fn transform_inline_module(content: &str, base: &str) -> String {
    use regex_lite::Regex;

    let mut code = content.to_string();

    // Normalize single quotes to double quotes for import specifiers
    // to simplify the regexes below (handles both `from 'path'` and `from "path"`).
    let re_single_quote = Regex::new(r#"from\s*'([^']+)'"#).expect("valid regex");
    code = re_single_quote
        .replace_all(&code, |caps: &regex_lite::Captures| {
            format!("from \"{}\"", &caps[1])
        })
        .to_string();
    // Also normalize bare side-effect imports: import 'path' → import "path"
    let re_bare_single = Regex::new(r#"import\s*'([^']+)'"#).expect("valid regex");
    code = re_bare_single
        .replace_all(&code, |caps: &regex_lite::Captures| {
            format!("import \"{}\"", &caps[1])
        })
        .to_string();

    // Default import: import Name from "/path" → const {default: Name} = await import("BASE/path")
    // Must run BEFORE side-effect import to avoid partial match.
    let re_default =
        Regex::new(r#"import\s+(\w+)\s+from\s*"([^"]+)""#).expect("valid regex");
    code = re_default
        .replace_all(&code, |caps: &regex_lite::Captures| {
            let name = &caps[1];
            let path = &caps[2];
            let full = resolve_import_path(path, base);
            format!("const {name} = (await import(\"{full}\")).default")
        })
        .to_string();

    // Side-effect import: import "/path" → try/catch wrapped await import
    let re_bare = Regex::new(r#"import\s*"([^"]+)""#).expect("valid regex");
    code = re_bare
        .replace_all(&code, |caps: &regex_lite::Captures| {
            let path = &caps[1];
            let full = resolve_import_path(path, base);
            format!("try {{ await import(\"{full}\") }} catch(_ie) {{ console.error('[import-error] {full}: ' + _ie.message) }}")
        })
        .to_string();

    // Namespace import: import * as name from "/path" — with fallback to empty module
    let re_star =
        Regex::new(r#"import\s*\*\s*as\s+(\w+)\s+from\s*"([^"]+)""#).expect("valid regex");
    code = re_star
        .replace_all(&code, |caps: &regex_lite::Captures| {
            let name = &caps[1];
            let path = &caps[2];
            let full = resolve_import_path(path, base);
            format!("let {name}; try {{ {name} = await import(\"{full}\") }} catch(_ie) {{ {name} = {{}}; console.error('[import-error] {full}: ' + _ie.message) }}")
        })
        .to_string();

    // Named import: import { a as b, c } from "/path"
    let re_named = Regex::new(r#"import\s*\{([^}]+)\}\s*from\s*"([^"]+)""#).expect("valid regex");
    code = re_named
        .replace_all(&code, |caps: &regex_lite::Captures| {
            let imports = caps[1].replace(" as ", ": ");
            let path = &caps[2];
            let full = resolve_import_path(path, base);
            format!("const {{{imports}}} = await import(\"{full}\")")
        })
        .to_string();

    // Dynamic import(): add base URL, await (not fire-and-forget).
    // The outer async IIFE try/catch handles errors.
    let re_dynamic = Regex::new(r#"import\("(/[^"]+)"\)"#).expect("valid regex");
    code = re_dynamic
        .replace_all(&code, |caps: &regex_lite::Captures| {
            let path = &caps[1];
            format!("await import(\"{base}{path}\")")
        })
        .to_string();

    // Rewrite Promise.allSettled (polyfill doesn't work in deno_core modules).
    if code.contains("Promise.allSettled(") {
        code = code.replace(
            "Promise.allSettled(",
            "((ps)=>Promise.all([...ps].map(p=>Promise.resolve(p)\
             .then(v=>({status:'fulfilled',value:v}),\
             r=>({status:'rejected',reason:r})))))(",
        );
    }

    // Wrap in async IIFE — execute as script (not module).
    // Each await import is individually resilient (top-level errors in imported modules
    // are caught but don't abort subsequent code). The outer try/catch is a safety net.
    format!("(async () => {{ try {{ {code} }} catch(e) {{ console.error('[inline-module-error] ' + (e.message || e) + ' @ ' + (e.stack || '').split('\\n')[1]); }} }})();")
}

/// R7c: After inline scripts execute, find and load the entry module.
///
/// Extracts entry URL from inline module content (last `import("...")` call)
/// or from `__reactRouterManifest.entry.module` if available.
pub(crate) fn boot_entry_module(
    inline_sources: &[String],
    base: &str,
    rt: &mut dyn neo_runtime::JsRuntime,
    errors: &mut Vec<String>,
) {
    // Strategy 1: Extract from inline module content (last import() call).
    // Only runs if we have inline module sources.
    let re = regex_lite::Regex::new(r#"import\("([^"]+)"\)"#).expect("valid regex");
    let mut entry_path = String::new();
    for source in inline_sources {
        let mut last_match = None;
        for caps in re.captures_iter(source) {
            if let Some(m) = caps.get(1) {
                last_match = Some(m.as_str().to_string());
            }
        }
        if let Some(m) = last_match {
            entry_path = m;
        }
    }

    // Strategy 2: Check __reactRouterManifest via eval.
    if entry_path.is_empty() {
        if let Ok(val) = rt.eval("window.__reactRouterManifest?.entry?.module || ''") {
            let trimmed = val.trim().trim_matches('"').trim_matches('\'');
            if !trimmed.is_empty() && trimmed != "undefined" {
                entry_path = trimmed.to_string();
            }
        }
    }

    // Strategy 3: Check for a Vite-style entry (last <script type="module"> named app.*.js).
    if entry_path.is_empty() {
        eprintln!("[hydration] Strategy 3: searching for Vite entry...");
        if let Ok(val) = rt.eval(
            "(function(){ var scripts = document.querySelectorAll('script[type=module][src]'); \
             for (var i = scripts.length - 1; i >= 0; i--) { \
               var src = scripts[i].getAttribute('src') || ''; \
               if (src.match(/\\/app[._]/)) return src; \
             } return ''; })()"
        ) {
            eprintln!("[hydration] Strategy 3 raw val: '{val}'");
            let trimmed = val.trim().trim_matches('"').trim_matches('\'');
            if !trimmed.is_empty() && trimmed != "undefined" {
                entry_path = trimmed.to_string();
                eprintln!("[hydration] Vite entry found: {entry_path}");
            }
        }
    }

    if entry_path.is_empty() {
        return;
    }

    // Resolve to absolute URL.
    let entry_url = if entry_path.starts_with('/') {
        format!("{base}{entry_path}")
    } else if entry_path.starts_with("http") {
        entry_path
    } else {
        return;
    };

    // Boot entry module via dynamic import() + settle.
    // We can't use load_module() because the URL was already loaded during
    // the script execution phase and ModuleEvaluator would skip it.
    // Dynamic import() triggers re-evaluation of the cached module graph.
    eprintln!("[hydration] Booting entry module: {entry_url}");
    let import_js = format!(
        "import('{}').then(function(m){{globalThis.__neo_entry_loaded=true}}).catch(function(e){{console.error('[ENTRY-ERROR] '+e.message)}})",
        entry_url.replace('\'', "\\'")
    );
    if let Err(e) = rt.execute(&import_js) {
        let msg = e.to_string();
        errors.push(format!(
            "entry {}: {msg}",
            entry_url.rsplit('/').next().unwrap_or(&entry_url)
        ));
    }

    // Settle to let the dynamic import resolve and React's createRoot/render fire.
    // Heavy SPAs (factorial=17MB entry) need generous settle time.
    if let Err(e) = rt.run_until_settled(15000) {
        let msg = e.to_string();
        if !msg.contains("timeout") {
            errors.push(format!("entry settle: {e}"));
        }
    }

    // Pump event loop once more to process any pending React renders.
    let _ = rt.pump_event_loop();
}

/// Resolve an import path against a base origin.
fn resolve_import_path(path: &str, base: &str) -> String {
    if path.starts_with("http") {
        path.to_string()
    } else if path.starts_with('/') {
        format!("{base}{path}")
    } else {
        path.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transform_bare_import() {
        let code = r#"import "/manifest.js";"#;
        let result = transform_inline_module(code, "https://example.com");
        assert!(result.contains(r#"await import("https://example.com/manifest.js")"#));
        assert!(result.starts_with("(async () =>"));
    }

    #[test]
    fn test_transform_star_import() {
        let code = r#"import * as route0 from "/route0.js";"#;
        let result = transform_inline_module(code, "https://example.com");
        // Star imports get try/catch wrapping with fallback to empty object.
        assert!(
            result.contains(r#"route0 = await import("https://example.com/route0.js")"#),
            "star import should resolve URL: {result}"
        );
        assert!(
            result.contains("let route0"),
            "star import should use let for try/catch: {result}"
        );
    }

    #[test]
    fn test_transform_named_import() {
        let code = r#"import { a as b, c } from "/utils.js";"#;
        let result = transform_inline_module(code, "https://example.com");
        assert!(
            result.contains(r#"const { a: b, c } = await import("https://example.com/utils.js")"#)
        );
    }

    #[test]
    fn test_transform_dynamic_import() {
        let code = r#"import("/entry.js");"#;
        let result = transform_inline_module(code, "https://example.com");
        // Dynamic imports get base URL prepended and are awaited.
        assert!(
            result.contains(r#"import("https://example.com/entry.js")"#),
            "dynamic import should resolve URL: {result}"
        );
    }

    #[test]
    fn test_transform_default_import() {
        let code = r#"import React from "https://esm.sh/react@18";"#;
        let result = transform_inline_module(code, "https://example.com");
        assert!(result.contains(r#"const React = (await import("https://esm.sh/react@18")).default"#));
    }

    #[test]
    fn test_transform_single_quote_imports() {
        let code = "import React from 'https://esm.sh/react@18';\nimport { useState } from 'https://esm.sh/react@18';";
        let result = transform_inline_module(code, "https://example.com");
        assert!(result.contains(r#"const React = (await import("https://esm.sh/react@18")).default"#));
        assert!(result.contains(r#"const { useState } = await import("https://esm.sh/react@18")"#));
    }

    #[test]
    fn test_transform_preserves_order() {
        let code = "import \"/a.js\";\nimport * as r from \"/b.js\";\nimport(\"/c.js\");";
        let result = transform_inline_module(code, "https://x.com");
        let pos_a = result.find("/a.js").unwrap();
        let pos_b = result.find("/b.js").unwrap();
        let pos_c = result.find("/c.js").unwrap();
        assert!(pos_a < pos_b);
        assert!(pos_b < pos_c);
    }

    // ─── Additional hydration pipeline tests ───

    #[test]
    fn test_transform_wraps_in_async_iife() {
        let code = "console.log('hello');";
        let result = transform_inline_module(code, "https://example.com");
        assert!(
            result.starts_with("(async () =>"),
            "Should wrap in async IIFE, got: {result}"
        );
        assert!(
            result.ends_with("})();"),
            "Should close IIFE, got: {result}"
        );
    }

    #[test]
    fn test_transform_has_try_catch() {
        let code = "console.log('hello');";
        let result = transform_inline_module(code, "https://example.com");
        assert!(
            result.contains("try {"),
            "Should have try/catch wrapper: {result}"
        );
        assert!(
            result.contains("catch(e)"),
            "Should have catch block: {result}"
        );
    }

    #[test]
    fn test_transform_multiple_imports() {
        let code = r#"import { a } from "/a.js";
import { b } from "/b.js";
console.log(a, b);"#;
        let result = transform_inline_module(code, "https://example.com");
        assert!(result.contains(r#"await import("https://example.com/a.js")"#));
        assert!(result.contains(r#"await import("https://example.com/b.js")"#));
        assert!(result.contains("console.log(a, b)"));
    }

    #[test]
    fn test_transform_full_url_passthrough() {
        let code = r#"import { x } from "https://cdn.example.com/lib.js";"#;
        let result = transform_inline_module(code, "https://example.com");
        assert!(
            result.contains(r#"await import("https://cdn.example.com/lib.js")"#),
            "Full URLs should pass through, got: {result}"
        );
    }

    #[test]
    fn test_transform_relative_path_not_resolved_without_slash() {
        // Relative paths like "./foo.js" should pass through as-is
        // (not prefixed with base, since base is just the origin)
        let code = r#"import { x } from "./foo.js";"#;
        let result = transform_inline_module(code, "https://example.com");
        // "./foo.js" doesn't start with "/" or "http", so resolve_import_path
        // returns it as-is.
        assert!(
            result.contains("./foo.js"),
            "Relative path should be kept: {result}"
        );
    }

    #[test]
    fn test_transform_promise_allsettled_rewrite() {
        let code = r#"const results = Promise.allSettled([p1, p2]);"#;
        let result = transform_inline_module(code, "https://example.com");
        assert!(
            !result.contains("Promise.allSettled("),
            "Promise.allSettled should be rewritten: {result}"
        );
        assert!(
            result.contains("Promise.all"),
            "Should use Promise.all polyfill: {result}"
        );
    }

    #[test]
    fn test_transform_bare_import_has_try_catch_wrapper() {
        let code = r#"import "/manifest.js";"#;
        let result = transform_inline_module(code, "https://example.com");
        // Bare imports should be wrapped in individual try/catch for resilience
        assert!(
            result.contains("try {") && result.contains("[import-error]"),
            "Bare import should have individual error handling: {result}"
        );
    }

    #[test]
    fn test_transform_star_import_has_fallback() {
        let code = r#"import * as mod from "/mod.js";"#;
        let result = transform_inline_module(code, "https://example.com");
        // Star imports should fall back to empty object on error
        assert!(
            result.contains("mod = {}"),
            "Star import should have empty object fallback: {result}"
        );
    }

    #[test]
    fn test_transform_empty_code() {
        let result = transform_inline_module("", "https://example.com");
        assert!(
            result.starts_with("(async () =>"),
            "Empty code should still produce valid IIFE: {result}"
        );
    }

    #[test]
    fn test_transform_non_import_code_preserved() {
        let code = r#"const x = 42;
document.getElementById('root').textContent = x;"#;
        let result = transform_inline_module(code, "https://example.com");
        assert!(result.contains("const x = 42"));
        assert!(result.contains("getElementById"));
    }

    // ─── resolve_import_path tests ───

    #[test]
    fn test_resolve_import_path_full_url() {
        let result = resolve_import_path("https://cdn.com/lib.js", "https://example.com");
        assert_eq!(result, "https://cdn.com/lib.js");
    }

    #[test]
    fn test_resolve_import_path_absolute() {
        let result = resolve_import_path("/assets/app.js", "https://example.com");
        assert_eq!(result, "https://example.com/assets/app.js");
    }

    #[test]
    fn test_resolve_import_path_relative() {
        let result = resolve_import_path("./local.js", "https://example.com");
        assert_eq!(result, "./local.js", "Relative paths passed through as-is");
    }

    // ─── boot_entry_module tests ───

    #[test]
    fn test_boot_entry_module_empty_sources() {
        use neo_runtime::mock::MockRuntime;
        let mut rt = MockRuntime::new();
        rt.set_default_eval(""); // Strategy 2+3 eval returns empty
        let mut errors = Vec::new();
        boot_entry_module(&[], "https://example.com", &mut rt, &mut errors);
        // With empty sources and no app.* in DOM, no entry module should be loaded
        assert!(errors.is_empty());
        // Strategy 2+3 will eval but find nothing → no import() call
        assert!(
            !rt.eval_calls.iter().any(|c| c.contains("import(")),
            "Should not call import(), eval_calls: {:?}",
            rt.eval_calls
        );
    }

    #[test]
    fn test_boot_entry_module_extracts_entry_from_inline() {
        use neo_runtime::mock::MockRuntime;
        let mut rt = MockRuntime::new();
        rt.set_default_eval(""); // __reactRouterManifest fallback returns empty
        let mut errors = Vec::new();
        let sources = vec![r#"import("/entry-client.js")"#.to_string()];
        boot_entry_module(&sources, "https://example.com", &mut rt, &mut errors);
        // Should have attempted to execute import() for the entry module
        assert!(
            rt.eval_calls.iter().any(|c| c.contains("entry-client")),
            "Should execute import() for entry module, eval_calls: {:?}",
            rt.eval_calls
        );
    }

    #[test]
    fn test_boot_entry_module_uses_first_import_per_source() {
        // BUG FOUND: boot_entry_module uses re.captures() which only
        // returns the FIRST match in each source string, not the last.
        // If two import() calls are in the SAME source string, only
        // the first is found. This is probably a bug — the comment says
        // "last import()" but captures() returns the first.
        //
        // With MULTIPLE source strings, the last source's first match wins
        // (because entry_path is overwritten per source).
        use neo_runtime::mock::MockRuntime;
        let mut rt = MockRuntime::new();
        rt.set_default_eval("");
        let mut errors = Vec::new();
        // Two separate sources: should use the match from the LAST source
        let sources = vec![
            r#"import("/first.js")"#.to_string(),
            r#"import("/second.js")"#.to_string(),
        ];
        boot_entry_module(&sources, "https://example.com", &mut rt, &mut errors);
        assert!(
            rt.eval_calls.iter().any(|c| c.contains("second.js")),
            "Should use last source's import, calls: {:?}",
            rt.eval_calls
        );
    }

    #[test]
    fn test_boot_entry_module_single_source_uses_last_match() {
        // Fixed: captures_iter() finds ALL matches, we take the last one.
        // Two import() in same source → last one wins (the entry module).
        use neo_runtime::mock::MockRuntime;
        let mut rt = MockRuntime::new();
        rt.set_default_eval("");
        let mut errors = Vec::new();
        let sources = vec![
            r#"import("/first.js"); import("/second.js")"#.to_string(),
        ];
        boot_entry_module(&sources, "https://example.com", &mut rt, &mut errors);
        assert!(
            rt.eval_calls.iter().any(|c| c.contains("second.js")),
            "captures_iter() should return last match: {:?}",
            rt.eval_calls
        );
    }
}
