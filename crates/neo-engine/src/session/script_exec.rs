//! Script fetch and execution helpers — free functions to avoid borrow conflicts.

use std::collections::HashMap;

use neo_http::{HttpClient, HttpError, HttpRequest, RequestContext, RequestKind};

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
pub(crate) fn execute_scripts(
    scripts: &[ScriptInfo],
    page_url: &str,
    rt: &mut dyn neo_runtime::JsRuntime,
    http: &dyn HttpClient,
    tracer: &dyn neo_trace::Tracer,
    trace_id: &str,
    errors: &mut Vec<String>,
) {
    for script in scripts {
        match script {
            ScriptInfo::Inline {
                content, is_module, ..
            } => {
                if *is_module {
                    if let Err(e) = rt.execute(content) {
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
            }
            ScriptInfo::Preload { .. } => {
                // Preloads are in the store — executed when imported by modules.
            }
        }
    }
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
