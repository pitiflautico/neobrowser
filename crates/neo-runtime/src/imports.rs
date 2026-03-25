//! ES import extraction from JavaScript source.
//!
//! Scans minified JS for static and dynamic import specifiers,
//! resolving relative paths against the script's URL.

/// Extract ES module import URLs from JavaScript source.
///
/// Handles minified forms: `from"./path.js"`, `import("./path.js")`,
/// `from '/path.js'`, `from "https://..."`, etc.
/// Resolves relative (`./`, `../`) and absolute (`/`) paths against the script URL.
/// Full URLs (`http://`, `https://`) are kept as-is.
pub fn extract_es_imports(js: &str, script_url: &str) -> Vec<String> {
    let mut imports = Vec::new();
    let base = if let Some(pos) = script_url.rfind('/') {
        &script_url[..=pos]
    } else {
        script_url
    };
    let origin = extract_origin(script_url);

    // Scan for from"..." and from'...' patterns (covers import/export from).
    extract_quoted_paths(js, "from", base, &origin, &mut imports);
    // Scan for import"..." (bare side-effect imports).
    extract_quoted_paths(js, "import", base, &origin, &mut imports);
    // Scan for import("..." (dynamic imports).
    extract_dynamic_imports(js, base, &origin, &mut imports);

    imports
}

/// Extract origin (scheme + host) from a URL.
fn extract_origin(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .map(|u| u.origin().ascii_serialization())
        .unwrap_or_default()
}

/// Scan for `keyword"path"` or `keyword 'path'` patterns.
/// Resolves relative (`./`, `../`), absolute (`/`), and full URL imports.
fn extract_quoted_paths(js: &str, keyword: &str, base: &str, origin: &str, out: &mut Vec<String>) {
    let bytes = js.as_bytes();
    let kw_bytes = keyword.as_bytes();
    let kw_len = kw_bytes.len();

    let mut i = 0;
    while i + kw_len <= bytes.len() {
        if &bytes[i..i + kw_len] != kw_bytes {
            i += 1;
            continue;
        }
        // Skip whitespace after keyword.
        let mut j = i + kw_len;
        while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
            j += 1;
        }
        if j >= bytes.len() {
            break;
        }
        let quote = bytes[j];
        if quote != b'"' && quote != b'\'' {
            i = j;
            continue;
        }
        j += 1;
        let start = j;
        while j < bytes.len() && bytes[j] != quote {
            j += 1;
        }
        if j < bytes.len() {
            let path = &js[start..j];
            if let Some(resolved) = resolve_import_path(path, base, origin) {
                if !out.contains(&resolved) {
                    out.push(resolved);
                }
            }
        }
        i = j + 1;
    }
}

/// Scan for `import("path")` or `import('path')` (dynamic imports).
/// Resolves relative, absolute, and full URL imports.
fn extract_dynamic_imports(js: &str, base: &str, origin: &str, out: &mut Vec<String>) {
    let needle = "import(";
    let mut search_from = 0;
    while let Some(pos) = js[search_from..].find(needle) {
        let abs = search_from + pos + needle.len();
        search_from = abs;
        if abs >= js.len() {
            break;
        }
        // Skip whitespace.
        let trimmed = js[abs..].trim_start();
        let quote = trimmed.as_bytes().first().copied().unwrap_or(0);
        if quote != b'"' && quote != b'\'' {
            continue;
        }
        let inner = &trimmed[1..];
        if let Some(end) = inner.find(quote as char) {
            let path = &inner[..end];
            if let Some(resolved) = resolve_import_path(path, base, origin) {
                if !out.contains(&resolved) {
                    out.push(resolved);
                }
            }
        }
    }
}

/// Resolve an import specifier to a full URL.
/// Handles: `./relative`, `../parent`, `/absolute`, `https://full-url`.
/// Returns `None` for bare specifiers (e.g. `react`, `lodash`).
fn resolve_import_path(path: &str, base: &str, origin: &str) -> Option<String> {
    if path.starts_with("http://") || path.starts_with("https://") {
        // Full URL — use as-is.
        Some(path.to_string())
    } else if let Some(rel) = path.strip_prefix("./") {
        // Relative import: resolve against base directory.
        Some(format!("{}{}", base, rel))
    } else if path.starts_with("../") {
        // Parent-relative: use url::Url for proper resolution.
        if let Ok(base_url) = url::Url::parse(base) {
            base_url.join(path).ok().map(|u| u.to_string())
        } else {
            None
        }
    } else if path.starts_with('/') {
        // Absolute path: resolve against origin.
        if !origin.is_empty() {
            Some(format!("{}{}", origin, path))
        } else {
            None
        }
    } else {
        // Bare specifier (e.g. "react") — skip, handled by import maps at load time.
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_es_imports_from_clause() {
        let js = r#"import{foo}from"./utils.js";import bar from"./helpers.js""#;
        let imports = extract_es_imports(js, "https://example.com/app/main.js");
        assert!(imports.contains(&"https://example.com/app/utils.js".to_string()));
        assert!(imports.contains(&"https://example.com/app/helpers.js".to_string()));
    }

    #[test]
    fn test_extract_es_imports_dynamic() {
        let js = r#"const m = import("./chunk-abc.js")"#;
        let imports = extract_es_imports(js, "https://example.com/app/main.js");
        assert!(imports.contains(&"https://example.com/app/chunk-abc.js".to_string()));
    }

    #[test]
    fn test_extract_es_imports_no_duplicates() {
        let js = r#"import"./a.js";import"./a.js""#;
        let imports = extract_es_imports(js, "https://example.com/app/main.js");
        assert_eq!(imports.len(), 1);
    }

    #[test]
    fn test_extract_es_imports_absolute_path() {
        let js = r#"import{x}from"/assets/vendor.js""#;
        let imports = extract_es_imports(js, "https://example.com/app/main.js");
        assert!(imports.contains(&"https://example.com/assets/vendor.js".to_string()));
    }

    #[test]
    fn test_extract_es_imports_full_url() {
        let js = r#"import{y}from"https://cdn.example.com/lib.js""#;
        let imports = extract_es_imports(js, "https://example.com/app/main.js");
        assert!(imports.contains(&"https://cdn.example.com/lib.js".to_string()));
    }

    #[test]
    fn test_extract_es_imports_parent_relative() {
        let js = r#"import{z}from"../shared/util.js""#;
        let imports = extract_es_imports(js, "https://example.com/app/sub/main.js");
        assert!(imports.contains(&"https://example.com/app/shared/util.js".to_string()));
    }

    #[test]
    fn test_extract_es_imports_dynamic_absolute() {
        let js = r#"const m = import("/chunks/abc.js")"#;
        let imports = extract_es_imports(js, "https://example.com/app/main.js");
        assert!(imports.contains(&"https://example.com/chunks/abc.js".to_string()));
    }

    #[test]
    fn test_extract_es_imports_bare_specifier_skipped() {
        let js = r#"import{React}from"react""#;
        let imports = extract_es_imports(js, "https://example.com/app/main.js");
        assert!(imports.is_empty(), "bare specifiers should be skipped");
    }

    // ─── Additional edge cases ───

    #[test]
    fn test_extract_es_imports_single_quotes() {
        let js = r#"import{x}from'./util.js'"#;
        let imports = extract_es_imports(js, "https://example.com/app/main.js");
        assert!(imports.contains(&"https://example.com/app/util.js".to_string()));
    }

    #[test]
    fn test_extract_es_imports_minified_no_spaces() {
        let js = r#"import{a}from"./a.js";import{b}from"./b.js";import{c}from"./c.js""#;
        let imports = extract_es_imports(js, "https://example.com/app/main.js");
        assert_eq!(imports.len(), 3);
    }

    #[test]
    fn test_extract_es_imports_side_effect_import() {
        let js = r#"import"./polyfill.js""#;
        let imports = extract_es_imports(js, "https://example.com/app/main.js");
        assert!(imports.contains(&"https://example.com/app/polyfill.js".to_string()));
    }

    #[test]
    fn test_extract_es_imports_re_export() {
        let js = r#"export{foo}from"./lib.js""#;
        let imports = extract_es_imports(js, "https://example.com/app/main.js");
        assert!(imports.contains(&"https://example.com/app/lib.js".to_string()));
    }

    #[test]
    fn test_extract_es_imports_dynamic_with_spaces() {
        let js = r#"const m = import( "./lazy.js" )"#;
        let imports = extract_es_imports(js, "https://example.com/app/main.js");
        assert!(imports.contains(&"https://example.com/app/lazy.js".to_string()));
    }

    #[test]
    fn test_extract_es_imports_mixed_static_and_dynamic() {
        let js = r#"import{a}from"./a.js";const b=import("./b.js")"#;
        let imports = extract_es_imports(js, "https://example.com/app/main.js");
        assert_eq!(imports.len(), 2);
        assert!(imports.contains(&"https://example.com/app/a.js".to_string()));
        assert!(imports.contains(&"https://example.com/app/b.js".to_string()));
    }

    #[test]
    fn test_extract_es_imports_empty_source() {
        let imports = extract_es_imports("", "https://example.com/app/main.js");
        assert!(imports.is_empty());
    }

    #[test]
    fn test_extract_es_imports_no_imports() {
        let js = "const x = 1; function foo() { return x; }";
        let imports = extract_es_imports(js, "https://example.com/app/main.js");
        assert!(imports.is_empty());
    }

    #[test]
    fn test_extract_es_imports_cross_origin_full_url() {
        let js = r#"import{lib}from"https://cdn.other.com/lib.js""#;
        let imports = extract_es_imports(js, "https://example.com/app/main.js");
        assert!(imports.contains(&"https://cdn.other.com/lib.js".to_string()));
    }

    #[test]
    fn test_extract_es_imports_parent_relative_deep() {
        let js = r#"import{u}from"../../shared/util.js""#;
        let imports = extract_es_imports(js, "https://example.com/app/sub/deep/main.js");
        assert!(imports.contains(&"https://example.com/app/shared/util.js".to_string()));
    }

    #[test]
    fn test_resolve_import_path_full_url() {
        assert_eq!(
            resolve_import_path("https://cdn.com/lib.js", "https://example.com/app/", "https://example.com"),
            Some("https://cdn.com/lib.js".to_string())
        );
    }

    #[test]
    fn test_resolve_import_path_relative() {
        assert_eq!(
            resolve_import_path("./util.js", "https://example.com/app/", "https://example.com"),
            Some("https://example.com/app/util.js".to_string())
        );
    }

    #[test]
    fn test_resolve_import_path_absolute() {
        assert_eq!(
            resolve_import_path("/assets/lib.js", "https://example.com/app/", "https://example.com"),
            Some("https://example.com/assets/lib.js".to_string())
        );
    }

    #[test]
    fn test_resolve_import_path_bare_returns_none() {
        assert_eq!(
            resolve_import_path("react", "https://example.com/app/", "https://example.com"),
            None
        );
    }

    #[test]
    fn test_resolve_import_path_absolute_no_origin() {
        assert_eq!(
            resolve_import_path("/lib.js", "not-a-url", ""),
            None
        );
    }
}
