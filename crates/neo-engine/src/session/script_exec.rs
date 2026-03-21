//! Script fetch and execution helpers — free functions to avoid borrow conflicts.

use std::collections::HashMap;

use neo_http::{HttpClient, HttpError, HttpRequest, RequestContext, RequestKind};

use super::hydration;
use super::scripts::ScriptInfo;

/// Fetch external scripts and modulepreloads, inserting into the runtime store.
pub(crate) fn fetch_external_scripts(
    scripts: &[ScriptInfo],
    page_url: &str,
    rt: &mut dyn neo_runtime::JsRuntime,
    http: &dyn HttpClient,
    errors: &mut Vec<String>,
) {
    for script in scripts {
        let url = match script.url() {
            Some(u) => u,
            None => continue,
        };
        if rt.has_module(url) {
            continue;
        }
        match fetch_single_script(http, url, page_url) {
            Ok(source) => {
                rt.insert_module(url, &source);
            }
            Err(e) => {
                if matches!(script, ScriptInfo::External { .. }) {
                    errors.push(format!("fetch {url}: {e}"));
                }
            }
        }
    }
}

/// Execute scripts in document order (inline, external, skipping preloads).
///
/// Inline modules are converted to async IIFE scripts (R7b) to avoid
/// top-level await blocking. After all scripts run, attempts entry module
/// boot (R7c) if React Router manifest is detected.
pub(crate) fn execute_scripts(
    scripts: &[ScriptInfo],
    page_url: &str,
    rt: &mut dyn neo_runtime::JsRuntime,
    http: &dyn HttpClient,
    tracer: &dyn neo_trace::Tracer,
    trace_id: &str,
    errors: &mut Vec<String>,
) {
    let base = extract_origin(page_url);
    let mut inline_module_sources: Vec<String> = Vec::new();

    for script in scripts {
        match script {
            ScriptInfo::Inline {
                content, is_module, ..
            } => {
                if *is_module {
                    // R7b: Transform inline module to async IIFE.
                    inline_module_sources.push(content.clone());
                    let iife = hydration::transform_inline_module(content, &base);
                    if let Err(e) = rt.execute(&iife) {
                        errors.push(format!("inline module: {e}"));
                    }
                } else if let Err(e) = rt.execute(content) {
                    errors.push(format!("inline script: {e}"));
                }
            }
            ScriptInfo::External {
                url: src,
                is_module,
            } => {
                if *is_module {
                    if let Err(e) = rt.load_module(src) {
                        errors.push(format!(
                            "module {}: {e}",
                            src.rsplit('/').next().unwrap_or(src)
                        ));
                    }
                } else {
                    execute_classic_external(src, page_url, rt, http, tracer, trace_id, errors);
                }
            }
            ScriptInfo::Preload { .. } => {
                // Preloads are in the store — executed when imported by modules.
            }
        }
    }

    // R7c: Entry module boot — load the entry module that inline IIFE fired.
    hydration::boot_entry_module(&inline_module_sources, &base, rt, errors);
}

/// Execute a classic (non-module) external script.
fn execute_classic_external(
    src: &str,
    page_url: &str,
    rt: &mut dyn neo_runtime::JsRuntime,
    http: &dyn HttpClient,
    tracer: &dyn neo_trace::Tracer,
    trace_id: &str,
    errors: &mut Vec<String>,
) {
    let source = rt.get_module_source(src);
    if let Some(code) = source {
        if let Err(e) = rt.execute(&code) {
            errors.push(format!(
                "script {}: {e}",
                src.rsplit('/').next().unwrap_or(src)
            ));
        }
    } else {
        tracer.module_event(src, "on_demand_fetch", trace_id);
        match fetch_single_script(http, src, page_url) {
            Ok(code) => {
                if let Err(e) = rt.execute(&code) {
                    errors.push(format!(
                        "script {}: {e}",
                        src.rsplit('/').next().unwrap_or(src)
                    ));
                }
            }
            Err(e) => {
                errors.push(format!("fetch {src}: {e}"));
            }
        }
    }
}

/// Extract origin (scheme + host) from a page URL.
fn extract_origin(page_url: &str) -> String {
    url::Url::parse(page_url)
        .ok()
        .map(|u| u.origin().ascii_serialization())
        .unwrap_or_default()
}

/// Fetch a single script via HTTP.
pub(crate) fn fetch_single_script(
    http: &dyn HttpClient,
    url: &str,
    page_url: &str,
) -> Result<String, HttpError> {
    let req = HttpRequest {
        method: "GET".to_string(),
        url: url.to_string(),
        headers: HashMap::new(),
        body: None,
        context: RequestContext {
            kind: RequestKind::Subresource,
            initiator: "parser".to_string(),
            referrer: Some(page_url.to_string()),
            frame_id: None,
            top_level_url: Some(page_url.to_string()),
        },
        timeout_ms: 5000,
    };
    let resp = http.request(&req)?;
    Ok(resp.body)
}
