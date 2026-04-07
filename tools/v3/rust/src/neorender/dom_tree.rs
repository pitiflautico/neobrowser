//! DOM Tree extraction — pieces 2, 3, 9 from PDR v3.
//!
//! Calls into V8 JS functions (__neo_dom_tree, __neo_get_mutations, __neo_get_diff)
//! loaded from js/dom_tree.js and js/observer.js.

use deno_core::JsRuntime;

/// Extract DOM tree as JSON from V8 (calls __neo_dom_tree).
/// Returns the full DOM as a nested JSON tree, skipping script/style/svg/noscript.
pub fn extract_dom_tree(
    runtime: &mut JsRuntime,
    max_depth: Option<u32>,
) -> Result<serde_json::Value, String> {
    let depth = max_depth.unwrap_or(50);
    let js = format!("__neo_dom_tree({})", depth);

    let result = runtime
        .execute_script("<neorender:dom_tree>", js)
        .map_err(|e| format!("DOM tree extraction error: {e}"))?;

    let scope = &mut runtime.handle_scope();
    let local = deno_core::v8::Local::new(scope, result);
    let json_str = local
        .to_string(scope)
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_else(|| "{}".to_string());

    serde_json::from_str(&json_str).map_err(|e| format!("DOM tree JSON parse error: {e}"))
}

/// Extract mutations since last call (calls __neo_get_mutations).
/// Returns accumulated MutationObserver records (or snapshot-based diffs as fallback).
pub fn extract_mutations(runtime: &mut JsRuntime) -> Result<Vec<serde_json::Value>, String> {
    let result = runtime
        .execute_script(
            "<neorender:mutations>",
            "__neo_get_mutations()".to_string(),
        )
        .map_err(|e| format!("Mutations extraction error: {e}"))?;

    let scope = &mut runtime.handle_scope();
    let local = deno_core::v8::Local::new(scope, result);
    let json_str = local
        .to_string(scope)
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_else(|| "[]".to_string());

    serde_json::from_str(&json_str).map_err(|e| format!("Mutations JSON parse error: {e}"))
}

/// Extract page diff summary (calls __neo_get_diff).
/// Returns {nodesAdded, nodesRemoved, attrsChanged, textChanged, details}.
pub fn extract_diff(runtime: &mut JsRuntime) -> Result<serde_json::Value, String> {
    let result = runtime
        .execute_script("<neorender:diff>", "__neo_get_diff()".to_string())
        .map_err(|e| format!("Diff extraction error: {e}"))?;

    let scope = &mut runtime.handle_scope();
    let local = deno_core::v8::Local::new(scope, result);
    let json_str = local
        .to_string(scope)
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_else(|| "{}".to_string());

    serde_json::from_str(&json_str).map_err(|e| format!("Diff JSON parse error: {e}"))
}
