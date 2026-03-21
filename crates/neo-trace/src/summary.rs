//! Summary builder — produces an [`ExecutionSummary`] from trace entries.

use crate::ExecutionSummary;
use neo_types::{PageState, TraceEntry};

/// Build an execution summary from a slice of trace entries.
///
/// Counts actions, requests, errors, DOM changes, and generates
/// warnings for conditions an AI should know about.
pub fn build_summary(entries: &[TraceEntry]) -> ExecutionSummary {
    let mut total_actions: usize = 0;
    let mut succeeded: usize = 0;
    let mut failed: usize = 0;
    let mut total_requests: usize = 0;
    let mut blocked_requests: usize = 0;
    let mut dom_changes: usize = 0;
    let mut js_errors: usize = 0;
    let mut max_ts: u64 = 0;
    let mut last_state = PageState::Idle;

    for entry in entries {
        if entry.timestamp_ms > max_ts {
            max_ts = entry.timestamp_ms;
        }

        if entry.action.starts_with("action:") {
            total_actions += 1;
            let is_success = entry
                .metadata
                .get("success")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if is_success {
                succeeded += 1;
            } else {
                failed += 1;
            }
        } else if entry.action.starts_with("network:") {
            total_requests += 1;
        } else if entry.action == "resource_blocked" {
            blocked_requests += 1;
        } else if entry.action == "js_exception" || entry.action == "console:error" {
            js_errors += 1;
        }

        dom_changes += entry.dom_mutations;

        if let Some(state) = entry.state_after {
            last_state = state;
        }
    }

    let warnings = build_warnings(blocked_requests, js_errors, failed, last_state);

    ExecutionSummary {
        total_actions,
        succeeded,
        failed,
        total_requests,
        blocked_requests,
        dom_changes,
        js_errors,
        duration_ms: max_ts,
        warnings,
        state: last_state,
    }
}

/// Generate warning strings for notable conditions.
fn build_warnings(
    blocked: usize,
    js_errors: usize,
    failed: usize,
    state: PageState,
) -> Vec<String> {
    let mut warnings = Vec::new();

    if blocked > 0 {
        warnings.push(format!("{blocked} requests blocked"));
    }
    if js_errors > 0 {
        warnings.push(format!("{js_errors} JS errors"));
    }
    if failed > 0 {
        warnings.push(format!("{failed} actions failed"));
    }
    if state == PageState::Failed {
        warnings.push("page in failed state".to_string());
    }
    if state == PageState::Blocked {
        warnings.push("page is blocked".to_string());
    }

    warnings
}
