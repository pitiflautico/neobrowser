//! Pipeline phases — extracted from pipeline.rs for testability.
//!
//! Each phase has clear inputs/outputs and can be tested independently.

use std::time::{Duration, Instant};

use neo_dom::DomEngine;
use neo_runtime::neo_trace;

/// Result of the extended settle phase.
pub struct SettleResult {
    /// Number of micro-task pump rounds executed.
    pub micro_rounds: u32,
    /// Final DOM node count.
    pub final_node_count: usize,
    /// Total time spent settling.
    pub elapsed_ms: u64,
    /// Whether an entry module was loaded post-settle.
    pub entry_module_loaded: bool,
}

/// Result of DOM export validation.
#[allow(dead_code)]
pub enum DomExportDecision {
    /// V8 DOM accepted — richer than original SSR parse.
    Accept { html: String, v8_elements: usize },
    /// V8 DOM rejected — impoverished compared to original.
    Reject { v8_elements: usize, threshold: usize },
    /// Export was empty.
    Empty,
    /// Export or re-parse failed.
    Error(String),
}

/// Run the extended settle loop: repeatedly pump the V8 event loop until DOM stabilizes.
///
/// Extracted from `run_script_pipeline` for clarity and testability.
pub fn run_settle_loop(
    rt: &mut dyn neo_runtime::JsRuntime,
    module_count: usize,
) -> SettleResult {
    let t0 = Instant::now();

    // Dynamic settle timeout: base 3000ms + 100ms per module discovered.
    // Complex SPAs with many modules need more time for hydration chains.
    // Capped at 15000ms. NEORENDER_SETTLE_MS env var overrides if set.
    let dynamic_settle_ms = std::cmp::min(3000 + (module_count as u64 * 100), 15000);
    let settle_budget = Duration::from_millis(
        std::env::var("NEORENDER_SETTLE_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(dynamic_settle_ms),
    );
    neo_trace!(
        "[SETTLE] budget={}ms (modules={}, base=3000)",
        settle_budget.as_millis(),
        module_count
    );

    let nodes_before = rt
        .eval("document.querySelectorAll('*').length")
        .unwrap_or_else(|_| "0".to_string())
        .trim()
        .parse::<usize>()
        .unwrap_or(0);

    let mut rounds = 0u32;
    let mut last_node_count = nodes_before;
    let mut stable_ticks = 0u32;

    while t0.elapsed() < settle_budget && stable_ticks < 5 {
        let remaining = settle_budget.saturating_sub(t0.elapsed());
        let chunk = std::cmp::min(remaining, Duration::from_millis(1000));
        if chunk.is_zero() {
            break;
        }

        let _ = rt.run_until_settled(chunk.as_millis() as u64);
        rounds += 1;

        let current_nodes = rt
            .eval("document.querySelectorAll('*').length")
            .unwrap_or_else(|_| "0".to_string())
            .trim()
            .parse::<usize>()
            .unwrap_or(0);

        let last_mutation_age = rt
            .eval("Date.now() - (window.__neorender_trace && window.__neorender_trace.lastMutationTime || 0)")
            .unwrap_or_else(|_| "9999".to_string())
            .trim()
            .parse::<u64>()
            .unwrap_or(9999);

        if current_nodes == last_node_count && last_mutation_age >= 100 {
            stable_ticks += 1;
        } else {
            stable_ticks = 0;
            if current_nodes != last_node_count {
                neo_trace!(
                    "[SETTLE] DOM changed: {} -> {} nodes (round {})",
                    last_node_count,
                    current_nodes,
                    rounds
                );
            } else {
                neo_trace!(
                    "[SETTLE] mutations still active ({}ms ago, round {})",
                    last_mutation_age,
                    rounds
                );
            }
            last_node_count = current_nodes;
        }
    }

    SettleResult {
        micro_rounds: rounds,
        final_node_count: last_node_count,
        elapsed_ms: t0.elapsed().as_millis() as u64,
        entry_module_loaded: false,
    }
}

/// Attempt to load a React Router / Vite entry module discovered post-settle.
///
/// Returns true if an entry module was found and loaded.
pub fn try_load_entry_module(
    rt: &mut dyn neo_runtime::JsRuntime,
    page_url: &str,
) -> bool {
    let entry_module = rt
        .eval(
            "(window.__reactRouterManifest && window.__reactRouterManifest.entry && window.__reactRouterManifest.entry.module) || ''",
        )
        .unwrap_or_default()
        .trim()
        .replace(['"', '\''], "");

    if entry_module.is_empty() || !entry_module.starts_with('/') {
        return false;
    }

    let full_url = if entry_module.starts_with("http") {
        entry_module.clone()
    } else if let Ok(base_url) = url::Url::parse(page_url) {
        base_url
            .join(&entry_module)
            .map(|u| u.to_string())
            .unwrap_or(entry_module.clone())
    } else {
        let path = if entry_module.starts_with('/') {
            entry_module.clone()
        } else {
            format!("/{}", entry_module)
        };
        format!("{}{}", page_url.trim_end_matches('/'), path)
    };

    neo_trace!("[SETTLE] loading entry module via eval_and_settle: {full_url}");
    let import_code = format!("import('{}')", full_url.replace('\'', "\\'"));
    match rt.eval_and_settle(&import_code, 5000) {
        Ok(result) => {
            neo_trace!(
                "[SETTLE] entry module: promise={}, {}ms, timers={}",
                result.was_promise,
                result.settled_ms,
                result.pending_timers
            );
            let new_nodes = rt
                .eval("document.querySelectorAll('*').length")
                .unwrap_or_default()
                .trim()
                .parse::<usize>()
                .unwrap_or(0);
            neo_trace!("[SETTLE] DOM after entry module: {} nodes", new_nodes);
            true
        }
        Err(e) => {
            neo_trace!("[SETTLE] entry module failed: {e}");
            false
        }
    }
}

/// Validate and decide whether to accept the V8-exported DOM over the original SSR parse.
///
/// Returns the decision with enough info for the caller to act on.
pub fn validate_dom_export(
    rt: &mut dyn neo_runtime::JsRuntime,
    original_elements: usize,
    page_url: &str,
) -> DomExportDecision {
    use super::script_parts::DomExportValidator;

    match rt.export_html() {
        Ok(html) if !html.is_empty() => {
            let node_count_str = rt
                .eval("document.querySelectorAll('*').length")
                .unwrap_or_else(|_| "?".to_string());
            neo_trace!(
                "[DOM_EXPORT] V8: {}B, {} V8 nodes, {} original elements",
                html.len(),
                node_count_str,
                original_elements
            );

            let validator = DomExportValidator::new(original_elements);

            let mut temp_dom = neo_dom::Html5everDom::new();
            match temp_dom.parse_html(&html, page_url) {
                Ok(()) => {
                    let v8_elements = temp_dom.query_selector_all("*").len();
                    if validator.should_accept(v8_elements) {
                        neo_trace!(
                            "[DOM_EXPORT] accepting V8 DOM ({} >= {} threshold)",
                            v8_elements,
                            validator.threshold()
                        );
                        DomExportDecision::Accept { html, v8_elements }
                    } else {
                        neo_trace!(
                            "[DOM_EXPORT] REJECTING V8 DOM ({} < {} threshold) — keeping original SSR parse",
                            v8_elements,
                            validator.threshold()
                        );
                        DomExportDecision::Reject {
                            v8_elements,
                            threshold: validator.threshold(),
                        }
                    }
                }
                Err(e) => DomExportDecision::Error(format!("re-parse probe: {e}")),
            }
        }
        Ok(empty) => {
            neo_trace!("[DOM_EXPORT] empty HTML returned (len={})", empty.len());
            DomExportDecision::Empty
        }
        Err(e) => {
            neo_trace!("[DOM_EXPORT] ERROR: {e}");
            DomExportDecision::Error(format!("export: {e}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_settle_budget_calculation() {
        // 0 modules → 3000ms base
        let budget = std::cmp::min(3000 + (0u64 * 100), 15000);
        assert_eq!(budget, 3000);

        // 50 modules → 8000ms
        let budget = std::cmp::min(3000 + (50u64 * 100), 15000);
        assert_eq!(budget, 8000);

        // 200 modules → capped at 15000ms
        let budget = std::cmp::min(3000 + (200u64 * 100), 15000);
        assert_eq!(budget, 15000);
    }
}
