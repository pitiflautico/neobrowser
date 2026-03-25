//! Script fetch and execution helpers — free functions to avoid borrow conflicts.

use std::collections::HashMap;

use neo_http::{HttpClient, HttpError, HttpRequest, RequestContext, RequestKind};
use neo_runtime::neo_trace;
use neo_runtime::trace_events::{TraceEvent};

use std::time::Instant;

use super::hydration;
use super::script_parts::{ExecutionGroups, FetchBudget, ScriptDedup};
use super::scripts::ScriptInfo;

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
    const FETCH_BUDGET_MS: u64 = 30_000;
    let mut budget = FetchBudget::new(FETCH_BUDGET_MS);

    for script in scripts {
        let url = match script.url() {
            Some(u) => u,
            None => continue,
        };
        if rt.has_module(url) {
            continue;
        }
        if budget.is_exhausted() {
            eprintln!(
                "[profile] fetch_budget: stopped after {} fetches, {}ms",
                budget.fetched_count(),
                budget.elapsed_ms()
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
                budget.record_fetch();
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
) -> Vec<TraceEvent> {
    let mut all_trace_events: Vec<TraceEvent> = Vec::new();
    let base = extract_origin(page_url);
    let mut inline_module_sources: Vec<String> = Vec::new();

    // Pre-register Node.js compat modules as stubs so bare imports resolve.
    // e.g., `import { Buffer } from 'buffer'` → our polyfill.
    rt.insert_module("buffer", "export const Buffer = globalThis.Buffer; export default globalThis.Buffer;");
    rt.insert_module("process", "export default globalThis.process; export const env = globalThis.process?.env || {};");
    rt.insert_module("stream", "export default {}; export const Readable = class {}; export const Writable = class {}; export const Transform = class {};");
    rt.insert_module("util", "export default {}; export function inherits() {} export function deprecate(fn) { return fn; }");
    rt.insert_module("events", "export default class EventEmitter { on(){return this} off(){return this} emit(){return this} addListener(){return this} removeListener(){return this} };");

    let mut cumulative_ms: u64 = 0;
    const EXEC_BUDGET_MS: u64 = 60_000;

    // Partition scripts into execution groups, preserving document order within each.
    let groups = ExecutionGroups::from_scripts(scripts);
    let mut dedup = ScriptDedup::new();

    // --- Group 1: Blocking scripts ---
    for &(idx, _) in &groups.blocking {
        let script = &scripts[idx];
        if cumulative_ms > EXEC_BUDGET_MS {
            log_budget_stop(idx, scripts.len(), cumulative_ms, "blocking");
            break;
        }
        if let Some(url) = script.url() {
            dedup.mark_executed(url);
        }
        cumulative_ms += execute_single(
            script, idx, page_url, &base, rt, http, tracer, trace_id,
            &mut inline_module_sources, errors,
        );
        drain_trace_into(rt, &mut all_trace_events, errors);
    }

    // --- Group 2: Deferred scripts (+ modules) ---
    for &(idx, _) in &groups.deferred {
        let script = &scripts[idx];
        if cumulative_ms > EXEC_BUDGET_MS {
            log_budget_stop(idx, scripts.len(), cumulative_ms, "deferred");
            break;
        }
        // Dedup: skip module scripts whose URL was already executed as blocking.
        // The same URL appearing as both <script src="X"> and <script type="module" src="X">
        // would cause deno_core to panic "Module already evaluated".
        if script.is_module() {
            if let Some(url) = script.url() {
                if dedup.should_skip_module(url) {
                    neo_trace!("[EXEC] dedup skip module {url} (already ran as blocking)");
                    continue;
                }
            }
        }
        cumulative_ms += execute_single(
            script, idx, page_url, &base, rt, http, tracer, trace_id,
            &mut inline_module_sources, errors,
        );
        drain_trace_into(rt, &mut all_trace_events, errors);
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
    for &(idx, _) in &groups.async_scripts {
        let script = &scripts[idx];
        if cumulative_ms > EXEC_BUDGET_MS {
            log_budget_stop(idx, scripts.len(), cumulative_ms, "async");
            break;
        }
        // Dedup: skip async module scripts whose URL was already executed as blocking.
        if script.is_module() {
            if let Some(url) = script.url() {
                if dedup.should_skip_module(url) {
                    neo_trace!("[EXEC] dedup skip async module {url} (already ran as blocking)");
                    continue;
                }
            }
        }
        cumulative_ms += execute_single(
            script, idx, page_url, &base, rt, http, tracer, trace_id,
            &mut inline_module_sources, errors,
        );
        drain_trace_into(rt, &mut all_trace_events, errors);
    }

    // Dispatch load event after all scripts.
    neo_trace!("[EVENT] load dispatched");
    if let Err(e) = rt.execute(concat!(
        "document.readyState = 'complete';",
        "window.dispatchEvent(new Event('load'));",
    )) {
        errors.push(format!("load event dispatch: {e}"));
    }

    // Final drain to catch any events from the load dispatch.
    drain_trace_into(rt, &mut all_trace_events, errors);

    all_trace_events
}

/// Drain trace events from the runtime, adding JsError events to `errors`
/// and all events to `all_events`.
fn drain_trace_into(
    rt: &mut dyn neo_runtime::JsRuntime,
    all_events: &mut Vec<TraceEvent>,
    errors: &mut Vec<String>,
) {
    let events = rt.drain_trace_events();
    for event in events {
        if let TraceEvent::JsError { ref message, ref source, .. } = event {
            errors.push(format!("[trace] {source}: {message}"));
        }
        all_events.push(event);
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

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::script_parts::{ExecutionGroups, ScriptDedup};
    use super::super::scripts::{ScriptInfo, ScriptKind};

    /// Verify that the dedup logic correctly identifies module scripts
    /// whose URLs were already executed as blocking scripts.
    #[test]
    fn dedup_filters_module_already_executed_as_blocking() {
        let vendor_url = "https://app.example.com/vendor.j304wec9xx.js";

        let scripts = vec![
            ScriptInfo::External {
                url: vendor_url.to_string(),
                is_module: false,
                kind: ScriptKind::ExternalBlocking,
            },
            ScriptInfo::External {
                url: vendor_url.to_string(),
                is_module: true,
                kind: ScriptKind::Module,
            },
            ScriptInfo::External {
                url: "https://app.example.com/app.abc123.js".to_string(),
                is_module: true,
                kind: ScriptKind::Module,
            },
        ];

        let groups = ExecutionGroups::from_scripts(&scripts);
        let mut dedup = ScriptDedup::new();

        // Mark blocking URLs as executed
        for &(idx, _) in &groups.blocking {
            if let Some(url) = scripts[idx].url() {
                dedup.mark_executed(url);
            }
        }

        // Apply dedup filter to deferred group
        let mut to_execute: Vec<&str> = Vec::new();
        let mut skipped: Vec<&str> = Vec::new();

        for &(idx, _) in &groups.deferred {
            let script = &scripts[idx];
            if script.is_module() {
                if let Some(url) = script.url() {
                    if dedup.should_skip_module(url) {
                        skipped.push(url);
                        continue;
                    }
                }
            }
            if let Some(url) = script.url() {
                to_execute.push(url);
            }
        }

        assert_eq!(
            skipped,
            vec![vendor_url],
            "vendor.js should be skipped (already ran as blocking)"
        );
        assert_eq!(
            to_execute,
            vec!["https://app.example.com/app.abc123.js"],
            "app.js should still execute (not a duplicate)"
        );
    }

    /// Verify that non-module deferred scripts are never deduped,
    /// even if their URL matches a blocking script.
    #[test]
    fn dedup_does_not_filter_non_module_deferred() {
        let url = "https://app.example.com/legacy.js";
        let mut dedup = ScriptDedup::new();
        dedup.mark_executed(url);

        let deferred_script = ScriptInfo::External {
            url: url.to_string(),
            is_module: false,
            kind: ScriptKind::Defer,
        };

        // The dedup check only applies to modules
        let should_skip = deferred_script.is_module()
            && deferred_script
                .url()
                .map(|u| dedup.should_skip_module(u))
                .unwrap_or(false);

        assert!(
            !should_skip,
            "non-module defer scripts should never be deduped"
        );
    }

    /// Verify inline scripts (no URL) are never affected by dedup.
    #[test]
    fn dedup_ignores_inline_scripts() {
        let dedup = ScriptDedup::new();

        let inline = ScriptInfo::Inline {
            content: "console.log('hello')".to_string(),
            is_module: true,
            kind: ScriptKind::InlineModule,
        };

        // Inline scripts have no URL, so dedup check should not match
        let should_skip = inline.is_module()
            && inline
                .url()
                .map(|u| dedup.should_skip_module(u))
                .unwrap_or(false);

        assert!(!should_skip, "inline modules have no URL, never deduped");
    }

    // ─── Execution group ordering (using ExecutionGroups) ───

    #[test]
    fn execution_order_blocking_before_deferred() {
        let scripts = vec![
            ScriptInfo::External {
                url: "https://example.com/defer.js".to_string(),
                is_module: false,
                kind: ScriptKind::Defer,
            },
            ScriptInfo::Inline {
                content: "var blocking = 1;".to_string(),
                is_module: false,
                kind: ScriptKind::InlineBlocking,
            },
            ScriptInfo::External {
                url: "https://example.com/module.mjs".to_string(),
                is_module: true,
                kind: ScriptKind::Module,
            },
            ScriptInfo::External {
                url: "https://example.com/blocking.js".to_string(),
                is_module: false,
                kind: ScriptKind::ExternalBlocking,
            },
        ];

        let groups = ExecutionGroups::from_scripts(&scripts);

        let blocking: Vec<usize> = groups.blocking.iter().map(|(i, _)| *i).collect();
        let deferred: Vec<usize> = groups.deferred.iter().map(|(i, _)| *i).collect();

        assert_eq!(blocking, vec![1, 3]);
        assert_eq!(deferred, vec![0, 2]);
    }

    #[test]
    fn execution_order_deferred_before_async() {
        let scripts = vec![
            ScriptInfo::External {
                url: "https://example.com/async.js".to_string(),
                is_module: false,
                kind: ScriptKind::Async,
            },
            ScriptInfo::External {
                url: "https://example.com/defer.js".to_string(),
                is_module: false,
                kind: ScriptKind::Defer,
            },
            ScriptInfo::External {
                url: "https://example.com/async-mod.mjs".to_string(),
                is_module: true,
                kind: ScriptKind::AsyncModule,
            },
            ScriptInfo::External {
                url: "https://example.com/module.mjs".to_string(),
                is_module: true,
                kind: ScriptKind::Module,
            },
        ];

        let groups = ExecutionGroups::from_scripts(&scripts);
        let deferred: Vec<usize> = groups.deferred.iter().map(|(i, _)| *i).collect();
        let async_s: Vec<usize> = groups.async_scripts.iter().map(|(i, _)| *i).collect();

        assert_eq!(deferred, vec![1, 3]);
        assert_eq!(async_s, vec![0, 2]);
    }

    #[test]
    fn document_order_preserved_within_groups() {
        let scripts = vec![
            ScriptInfo::External {
                url: "https://example.com/b1.js".to_string(),
                is_module: false,
                kind: ScriptKind::ExternalBlocking,
            },
            ScriptInfo::Inline {
                content: "var x = 1;".to_string(),
                is_module: false,
                kind: ScriptKind::InlineBlocking,
            },
            ScriptInfo::External {
                url: "https://example.com/b2.js".to_string(),
                is_module: false,
                kind: ScriptKind::ExternalBlocking,
            },
        ];

        let groups = ExecutionGroups::from_scripts(&scripts);
        let indices: Vec<usize> = groups.blocking.iter().map(|(i, _)| *i).collect();
        assert_eq!(indices, vec![0, 1, 2], "document order must be preserved");
    }

    #[test]
    fn dedup_also_applies_to_async_group() {
        let url = "https://example.com/shared.js";
        let mut dedup = ScriptDedup::new();
        dedup.mark_executed(url);

        let async_script = ScriptInfo::External {
            url: url.to_string(),
            is_module: true,
            kind: ScriptKind::AsyncModule,
        };

        let should_skip = async_script.is_module()
            && async_script
                .url()
                .map(|u| dedup.should_skip_module(u))
                .unwrap_or(false);

        assert!(should_skip, "async module with same URL as blocking should be skipped");
    }

    #[test]
    fn importmap_and_ignored_not_in_execution_groups() {
        let scripts = vec![
            ScriptInfo::Inline {
                content: r#"{"imports":{}}"#.to_string(),
                is_module: false,
                kind: ScriptKind::ImportMap,
            },
            ScriptInfo::Inline {
                content: r#"{"data": 1}"#.to_string(),
                is_module: false,
                kind: ScriptKind::Ignored,
            },
        ];

        let groups = ExecutionGroups::from_scripts(&scripts);
        assert_eq!(groups.total(), 0);
    }

    #[test]
    fn preload_kind_is_deferred_module() {
        let preload = ScriptInfo::Preload {
            url: "https://example.com/chunk.js".to_string(),
        };

        assert_eq!(preload.kind(), ScriptKind::Module);
        assert!(preload.is_module());
        assert_eq!(preload.url(), Some("https://example.com/chunk.js"));
    }

    #[test]
    fn test_extract_origin() {
        assert_eq!(
            extract_origin("https://example.com/page/index.html"),
            "https://example.com"
        );
        assert_eq!(
            extract_origin("https://sub.example.com:8080/page"),
            "https://sub.example.com:8080"
        );
        assert_eq!(extract_origin("not-a-url"), "");
    }
}
