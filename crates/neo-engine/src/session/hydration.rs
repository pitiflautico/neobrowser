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
    if inline_sources.is_empty() {
        return;
    }

    // Strategy 1: Extract from inline module content (last import() call).
    let re = regex_lite::Regex::new(r#"import\("([^"]+)"\)"#).expect("valid regex");
    let mut entry_path = String::new();
    for source in inline_sources {
        if let Some(caps) = re.captures(source) {
            entry_path = caps[1].to_string();
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

    // Load entry module directly.
    if let Err(e) = rt.load_module(&entry_url) {
        errors.push(format!(
            "entry {}: {e}",
            entry_url.rsplit('/').next().unwrap_or(&entry_url)
        ));
    }

    // Brief settle to let hydrateRoot fire.
    if let Err(e) = rt.run_until_settled(500) {
        let msg = e.to_string();
        if !msg.contains("timeout") {
            errors.push(format!("entry settle: {e}"));
        }
    }
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
}
