//! Script fetch and execution helpers — free functions to avoid borrow conflicts.

use std::collections::HashMap;

use neo_http::{HttpClient, HttpError, HttpRequest, RequestContext, RequestKind};
use neo_runtime::neo_trace;

use std::time::Instant;

use super::hydration;
use super::scripts::{ScriptInfo, ScriptKind};

/// Fetch external scripts and modulepreloads, inserting into the runtime store.
///
/// Enforces a cumulative 3s fetch budget — if total fetch time exceeds
/// the budget, remaining scripts are skipped.
pub(crate) fn fetch_external_scripts(
    scripts: &[ScriptInfo],
    page_url: &str,
    rt: &mut dyn neo_runtime::JsRuntime,
    http: &dyn HttpClient,
    errors: &mut Vec<String>,
) {
    const FETCH_BUDGET_MS: u64 = 2_000;
    let budget_start = Instant::now();
    let mut fetched = 0usize;

    for script in scripts {
        let url = match script.url() {
            Some(u) => u,
            None => continue,
        };
        if rt.has_module(url) {
            continue;
        }
        if budget_start.elapsed().as_millis() as u64 > FETCH_BUDGET_MS {
            eprintln!(
                "[profile] fetch_budget: stopped after {fetched} fetches, {}ms",
                budget_start.elapsed().as_millis()
            );
            break;
        }
        let ft = Instant::now();
        match fetch_single_script(http, url, page_url) {
            Ok(source) => {
                let name = url.rsplit('/').next().unwrap_or(url);
                let size_kb = source.len() / 1024;
                let fetch_ms = ft.elapsed().as_millis();
                neo_trace!("[SCRIPT] fetch {url} -> 200 ({size_kb}KB, {fetch_ms}ms)");
                if size_kb > 50 {
                    eprintln!("[profile]   fetched: {name} ({size_kb}KB)");
                }
                rt.insert_module(url, &source);
                fetched += 1;
            }
            Err(e) => {
                let fetch_ms = ft.elapsed().as_millis();
                neo_trace!("[SCRIPT] fetch {url} -> error ({fetch_ms}ms): {e}");
                if matches!(script, ScriptInfo::External { .. }) {
                    errors.push(format!("fetch {url}: {e}"));
                }
            }
        }
    }
}

/// Execute scripts in browser-approximated order:
///
/// **Group 1** — Blocking: `InlineBlocking` + `ExternalBlocking` (document order)
///   These simulate "parse-blocking" scripts that run during parsing.
///
/// **Group 2** — Deferred: `Defer` + `Module` + `InlineModule` (document order)
///   These run after parsing but before DOMContentLoaded.
///   After this group, dispatches `DOMContentLoaded` and sets `readyState = 'interactive'`.
///
/// **Group 3** — Async: `Async` + `AsyncModule` (document order, approximating fetch-completion)
///   These run whenever ready. After this group, dispatches `load` event
///   and sets `readyState = 'complete'`.
///
/// `ImportMap` and `Preload` entries are skipped (not executed as JS).
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

    let mut cumulative_ms: u64 = 0;
    const EXEC_BUDGET_MS: u64 = 2_000;

    // Partition scripts into execution groups, preserving document order within each.
    let mut blocking: Vec<(usize, &ScriptInfo)> = Vec::new();
    let mut deferred: Vec<(usize, &ScriptInfo)> = Vec::new();
    let mut async_scripts: Vec<(usize, &ScriptInfo)> = Vec::new();

    for (idx, script) in scripts.iter().enumerate() {
        match script.kind() {
            ScriptKind::InlineBlocking | ScriptKind::ExternalBlocking => {
                blocking.push((idx, script));
            }
            ScriptKind::Defer | ScriptKind::Module | ScriptKind::InlineModule => {
                deferred.push((idx, script));
            }
            ScriptKind::Async | ScriptKind::AsyncModule => {
                async_scripts.push((idx, script));
            }
            ScriptKind::ImportMap | ScriptKind::Ignored => {
                // Not executed as JS.
            }
        }
    }

    // --- Group 1: Blocking scripts ---
    for &(idx, script) in &blocking {
        if cumulative_ms > EXEC_BUDGET_MS {
            log_budget_stop(idx, scripts.len(), cumulative_ms, "blocking");
            break;
        }
        cumulative_ms += execute_single(
            script, idx, page_url, &base, rt, http, tracer, trace_id,
            &mut inline_module_sources, errors,
        );
    }

    // --- Group 2: Deferred scripts (+ modules) ---
    for &(idx, script) in &deferred {
        if cumulative_ms > EXEC_BUDGET_MS {
            log_budget_stop(idx, scripts.len(), cumulative_ms, "deferred");
            break;
        }
        cumulative_ms += execute_single(
            script, idx, page_url, &base, rt, http, tracer, trace_id,
            &mut inline_module_sources, errors,
        );
    }

    // R7c: Entry module boot — load the entry module that inline IIFE fired.
    hydration::boot_entry_module(&inline_module_sources, &base, rt, errors);

    // Dispatch DOMContentLoaded after deferred scripts.
    neo_trace!("[EVENT] DOMContentLoaded dispatched");
    if let Err(e) = rt.execute(concat!(
        "document.readyState = 'interactive';",
        "document.dispatchEvent(new Event('DOMContentLoaded', {bubbles: true}));",
    )) {
        errors.push(format!("DOMContentLoaded dispatch: {e}"));
    }

    // --- Group 3: Async scripts ---
    for &(idx, script) in &async_scripts {
        if cumulative_ms > EXEC_BUDGET_MS {
            log_budget_stop(idx, scripts.len(), cumulative_ms, "async");
            break;
        }
        cumulative_ms += execute_single(
            script, idx, page_url, &base, rt, http, tracer, trace_id,
            &mut inline_module_sources, errors,
        );
    }

    // Dispatch load event after all scripts.
    neo_trace!("[EVENT] load dispatched");
    if let Err(e) = rt.execute(concat!(
        "document.readyState = 'complete';",
        "window.dispatchEvent(new Event('load'));",
    )) {
        errors.push(format!("load event dispatch: {e}"));
    }
}

/// Execute a single script, returning elapsed milliseconds.
#[allow(clippy::too_many_arguments)]
fn execute_single(
    script: &ScriptInfo,
    idx: usize,
    page_url: &str,
    base: &str,
    rt: &mut dyn neo_runtime::JsRuntime,
    http: &dyn HttpClient,
    tracer: &dyn neo_trace::Tracer,
    trace_id: &str,
    inline_module_sources: &mut Vec<String>,
    errors: &mut Vec<String>,
) -> u64 {
    let t = Instant::now();
    match script {
        ScriptInfo::Inline {
            content, is_module, ..
        } => {
            let label = if *is_module { "inline-module" } else { "inline-script" };
            if *is_module {
                // R7b: Transform inline module to async IIFE with await import().
                // execute() starts the IIFE, then run_until_settled() drives the
                // event loop so the dynamic imports inside actually resolve.
                inline_module_sources.push(content.clone());
                let iife = hydration::transform_inline_module(content, base);
                match rt.execute(&iife) {
                    Ok(()) => {
                        // Drive the event loop to resolve the await import() calls.
                        let settle_ms = 10000u64;
                        let _ = rt.run_until_settled(settle_ms);
                        let ms = t.elapsed().as_millis() as u64;
                        neo_trace!("[EXEC] {label}#{idx} -> ok ({ms}ms, settled)");
                    }
                    Err(e) => {
                        let ms = t.elapsed().as_millis() as u64;
                        neo_trace!("[EXEC] {label}#{idx} -> error: {e} ({ms}ms)");
                        errors.push(format!("inline module: {e}"));
                    }
                }
            } else {
                match rt.execute(content) {
                    Ok(()) => {
                        let ms = t.elapsed().as_millis() as u64;
                        neo_trace!("[EXEC] {label}#{idx} -> ok ({ms}ms)");
                    }
                    Err(e) => {
                        let ms = t.elapsed().as_millis() as u64;
                        neo_trace!("[EXEC] {label}#{idx} -> error: {e} ({ms}ms)");
                        errors.push(format!("inline script: {e}"));
                    }
                }
            }
            let ms = t.elapsed().as_millis() as u64;
            if ms > 50 {
                let preview = &content[..content.len().min(80)];
                eprintln!("[profile]   inline#{idx}: {ms}ms | {preview}...");
            }
            ms
        }
        ScriptInfo::External {
            url: src,
            is_module,
            ..
        } => {
            let name = src.rsplit('/').next().unwrap_or(src);
            if *is_module {
                match rt.load_module(src) {
                    Ok(()) => {
                        let ms = t.elapsed().as_millis() as u64;
                        neo_trace!("[EXEC] {name} -> ok ({ms}ms)");
                    }
                    Err(e) => {
                        let ms = t.elapsed().as_millis() as u64;
                        neo_trace!("[EXEC] {name} -> error: {e} ({ms}ms)");
                        errors.push(format!("module {name}: {e}"));
                    }
                }
            } else {
                execute_classic_external(src, page_url, rt, http, tracer, trace_id, errors);
                let ms = t.elapsed().as_millis() as u64;
                neo_trace!("[EXEC] {name} -> ok ({ms}ms)");
            }
            let ms = t.elapsed().as_millis() as u64;
            if ms > 50 {
                eprintln!("[profile]   ext#{idx}: {ms}ms | {name}");
            }
            ms
        }
        ScriptInfo::Preload { .. } => 0,
    }
}

/// Log when the execution budget is exhausted.
fn log_budget_stop(idx: usize, total: usize, cumulative_ms: u64, group: &str) {
    eprintln!(
        "[profile] script_exec_budget: stopped {group} at script {idx}/{total} after {cumulative_ms}ms",
    );
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
        timeout_ms: 2000,
    };
    let resp = http.request(&req)?;
    Ok(resp.body)
}
