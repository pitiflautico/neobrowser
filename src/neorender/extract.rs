//! Auto-extraction — tables, articles, form schemas, structured data.
//!
//! Calls JS helpers injected via js/extract.js and returns parsed JSON.

use deno_core::JsRuntime;
use serde_json::Value;

/// Extract all HTML tables as structured JSON.
/// Returns: `[{headers: [string], rows: [[string]]}]`
pub fn extract_tables(runtime: &mut JsRuntime) -> Result<Vec<Value>, String> {
    let json_str = eval_string(runtime, "__neo_extract_tables()")?;
    serde_json::from_str(&json_str)
        .map_err(|e| format!("extract_tables parse error: {e}"))
}

/// Extract article content: title, author, date, body text.
/// Returns: `{title, author, date, body}`
pub fn extract_article(runtime: &mut JsRuntime) -> Result<Value, String> {
    let json_str = eval_string(runtime, "__neo_extract_article()")?;
    serde_json::from_str(&json_str)
        .map_err(|e| format!("extract_article parse error: {e}"))
}

/// Extract form schema for the first (or specified) form.
/// Returns: `{action, method, fields: [{name, type, required, placeholder, value, options?}]}`
/// Returns `null` Value if no form found.
pub fn extract_form_schema(runtime: &mut JsRuntime, selector: Option<&str>) -> Result<Value, String> {
    let js = match selector {
        Some(sel) => {
            let escaped = sel.replace('\'', "\\'").replace('\\', "\\\\");
            format!("__neo_extract_form_schema('{}')", escaped)
        }
        None => "__neo_extract_form_schema()".to_string(),
    };
    let json_str = eval_string(runtime, &js)?;
    serde_json::from_str(&json_str)
        .map_err(|e| format!("extract_form_schema parse error: {e}"))
}

/// Extract structured data: JSON-LD and Open Graph meta.
/// Returns: `{jsonld: [object], og: {property: value}}`
pub fn extract_structured(runtime: &mut JsRuntime) -> Result<Value, String> {
    let json_str = eval_string(runtime, "__neo_extract_structured()")?;
    serde_json::from_str(&json_str)
        .map_err(|e| format!("extract_structured parse error: {e}"))
}

// ─── Helpers ───

fn eval_string(runtime: &mut JsRuntime, js: &str) -> Result<String, String> {
    let result = runtime.execute_script("<neo:extract>", js.to_string())
        .map_err(|e| format!("extract eval error: {e}"))?;
    let scope = &mut runtime.handle_scope();
    let local = deno_core::v8::Local::new(scope, result);
    if let Some(s) = local.to_string(scope) {
        Ok(s.to_rust_string_lossy(scope))
    } else {
        Ok("null".to_string())
    }
}
