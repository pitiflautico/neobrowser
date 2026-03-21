//! ES import extraction from JavaScript source.
//!
//! Scans minified JS for static and dynamic import specifiers,
//! resolving relative paths against the script's URL.

/// Extract ES module import URLs from JavaScript source.
///
/// Handles minified forms: `from"./path.js"`, `import("./path.js")`,
/// `from './path.js'`, etc. Only resolves relative imports (`./`).
pub fn extract_es_imports(js: &str, script_url: &str) -> Vec<String> {
    let mut imports = Vec::new();
    let base = if let Some(pos) = script_url.rfind('/') {
        &script_url[..=pos]
    } else {
        script_url
    };

    // Scan for from"./..." and from'./...' patterns (covers import/export from).
    extract_quoted_paths(js, "from", base, &mut imports);
    // Scan for import"./..." (bare side-effect imports).
    extract_quoted_paths(js, "import", base, &mut imports);
    // Scan for import("./..." (dynamic imports).
    extract_dynamic_imports(js, base, &mut imports);

    imports
}

/// Scan for `keyword"./path"` or `keyword './path'` patterns.
fn extract_quoted_paths(js: &str, keyword: &str, base: &str, out: &mut Vec<String>) {
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
            if let Some(rel) = path.strip_prefix("./") {
                let full = format!("{}{}", base, rel);
                if !out.contains(&full) {
                    out.push(full);
                }
            }
        }
        i = j + 1;
    }
}

/// Scan for `import("./path")` or `import('./path')` (dynamic imports).
fn extract_dynamic_imports(js: &str, base: &str, out: &mut Vec<String>) {
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
            if let Some(rel) = path.strip_prefix("./") {
                let full = format!("{}{}", base, rel);
                if !out.contains(&full) {
                    out.push(full);
                }
            }
        }
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
}
