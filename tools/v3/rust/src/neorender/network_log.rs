//! Network log — reads fetch intercept data from V8 runtime.
//!
//! The JS side (intercept.js) wraps globalThis.fetch and logs every request.
//! This module provides Rust access to that log.

use deno_core::JsRuntime;

/// A single network request entry logged by the fetch interceptor.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct NetworkEntry {
    pub method: String,
    pub url: String,
    pub status: u16,
    pub size: usize,
    #[serde(rename = "duration")]
    pub duration_ms: u64,
}

/// Get network log entries from V8, optionally filtered.
pub fn get_network_log(runtime: &mut JsRuntime, filter: Option<&str>) -> Result<Vec<NetworkEntry>, String> {
    let js = match filter {
        Some(f) => {
            let escaped = f.replace('\\', "\\\\").replace('"', "\\\"");
            format!("globalThis.__neo_get_network_log(\"{}\")", escaped)
        }
        None => "globalThis.__neo_get_network_log()".to_string(),
    };

    let result = runtime.execute_script("<neorender:network_log>", js)
        .map_err(|e| format!("network_log eval error: {e}"))?;

    let scope = &mut runtime.handle_scope();
    let local = deno_core::v8::Local::new(scope, result);
    let json_str = local
        .to_string(scope)
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_else(|| "[]".to_string());

    serde_json::from_str(&json_str).map_err(|e| format!("network_log parse error: {e}"))
}

/// Clear the network log in V8.
pub fn clear_network_log(runtime: &mut JsRuntime) -> Result<(), String> {
    runtime
        .execute_script("<neorender:network_log_clear>", "globalThis.__neo_clear_network_log()".to_string())
        .map_err(|e| format!("network_log clear error: {e}"))?;
    Ok(())
}
