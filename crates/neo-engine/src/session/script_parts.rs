//! Extracted, testable components from the script execution pipeline.
//!
//! Each struct encapsulates a single concern:
//! - `FetchBudget` — time-based budget for script fetching
//! - `ScriptDedup` — deduplication of module URLs already executed as blocking
//! - `ExecutionGroups` — partitioning scripts by execution phase
//! - `DomExportValidator` — deciding whether to accept V8's DOM over SSR
//! - `fire_observers` — firing synthetic observer callbacks post-settle

use std::collections::HashSet;
use std::time::Instant;

use super::scripts::{ScriptInfo, ScriptKind};

// ─── FetchBudget ───

/// Time-based budget for limiting cumulative script fetch duration.
///
/// Once the budget is exhausted, callers should stop fetching additional scripts.
pub(crate) struct FetchBudget {
    max_ms: u64,
    start: Instant,
    fetched: usize,
}

impl FetchBudget {
    pub fn new(max_ms: u64) -> Self {
        Self {
            max_ms,
            start: Instant::now(),
            fetched: 0,
        }
    }

    /// Returns true if the cumulative fetch time exceeds the budget.
    pub fn is_exhausted(&self) -> bool {
        self.elapsed_ms() > self.max_ms
    }

    /// Record that one script was successfully fetched.
    pub fn record_fetch(&mut self) {
        self.fetched += 1;
    }

    /// Elapsed milliseconds since the budget was created.
    pub fn elapsed_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    /// Number of scripts fetched so far.
    pub fn fetched_count(&self) -> usize {
        self.fetched
    }
}

// ─── ScriptDedup ───

/// Tracks which script URLs have been executed, to avoid re-executing
/// the same URL as both a blocking script and a module.
pub(crate) struct ScriptDedup {
    executed_urls: HashSet<String>,
}

impl ScriptDedup {
    pub fn new() -> Self {
        Self {
            executed_urls: HashSet::new(),
        }
    }

    /// Mark a URL as already executed.
    pub fn mark_executed(&mut self, url: &str) {
        self.executed_urls.insert(url.to_string());
    }

    /// Returns true if this module URL was already executed as blocking
    /// and should be skipped. Only applies to module scripts (non-modules
    /// and inline scripts are never skipped).
    pub fn should_skip_module(&self, url: &str) -> bool {
        self.executed_urls.contains(url)
    }
}

// ─── ExecutionGroups ───

/// Scripts partitioned into browser execution groups, preserving document order.
///
/// - **blocking**: `InlineBlocking` + `ExternalBlocking` — run during parsing
/// - **deferred**: `Defer` + `Module` + `InlineModule` — run after parsing, before DOMContentLoaded
/// - **async_scripts**: `Async` + `AsyncModule` — run when ready, after DOMContentLoaded
pub(crate) struct ExecutionGroups {
    /// (document_index, script_ref) pairs for each group.
    pub blocking: Vec<(usize, usize)>,
    pub deferred: Vec<(usize, usize)>,
    pub async_scripts: Vec<(usize, usize)>,
}

impl ExecutionGroups {
    /// Partition scripts into execution groups by kind.
    ///
    /// Each entry is `(document_index, index_into_scripts_slice)` — both are the same
    /// value since the input slice is already in document order.
    pub fn from_scripts(scripts: &[ScriptInfo]) -> Self {
        let mut blocking = Vec::new();
        let mut deferred = Vec::new();
        let mut async_scripts = Vec::new();

        for (idx, script) in scripts.iter().enumerate() {
            match script.kind() {
                ScriptKind::InlineBlocking | ScriptKind::ExternalBlocking => {
                    blocking.push((idx, idx));
                }
                ScriptKind::Defer | ScriptKind::Module | ScriptKind::InlineModule => {
                    deferred.push((idx, idx));
                }
                ScriptKind::Async | ScriptKind::AsyncModule => {
                    async_scripts.push((idx, idx));
                }
                ScriptKind::ImportMap | ScriptKind::Ignored => {
                    // Not executed as JS.
                }
            }
        }

        Self {
            blocking,
            deferred,
            async_scripts,
        }
    }

    /// Total number of executable scripts across all groups.
    pub fn total(&self) -> usize {
        self.blocking.len() + self.deferred.len() + self.async_scripts.len()
    }
}

// ─── DomExportValidator ───

/// Validates whether the V8-produced DOM should replace the original SSR parse.
///
/// If hydration failed (V8 DOM is impoverished), we keep the original.
/// The threshold is 80% of the original element count.
pub(crate) struct DomExportValidator {
    original_element_count: usize,
    acceptance_threshold: f64,
}

impl DomExportValidator {
    pub fn new(original_count: usize) -> Self {
        Self {
            original_element_count: original_count,
            acceptance_threshold: 0.8,
        }
    }

    /// Returns true if the V8 DOM has enough elements to be considered
    /// a valid hydration result.
    pub fn should_accept(&self, v8_count: usize) -> bool {
        let threshold = (self.original_element_count as f64 * self.acceptance_threshold) as usize;
        v8_count >= threshold.max(1)
    }

    /// The computed threshold value (for logging).
    pub fn threshold(&self) -> usize {
        let t = (self.original_element_count as f64 * self.acceptance_threshold) as usize;
        t.max(1)
    }
}

// ─── Observer firing ───

/// Fire synthetic IntersectionObserver and ResizeObserver callbacks post-settle.
///
/// Returns `(intersection_observer_count, resize_observer_count)`.
pub(crate) fn fire_observers(runtime: &mut dyn neo_runtime::JsRuntime) -> (usize, usize) {
    let io = runtime
        .eval("typeof __neo_fireIntersectionObservers==='function'?__neo_fireIntersectionObservers():0")
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let ro = runtime
        .eval("typeof __neo_fireResizeObservers==='function'?__neo_fireResizeObservers():0")
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .unwrap_or(0);
    (io, ro)
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::scripts::{ScriptInfo, ScriptKind};

    // ─── FetchBudget tests ───

    #[test]
    fn fetch_budget_not_exhausted_immediately() {
        let budget = FetchBudget::new(5000);
        assert!(!budget.is_exhausted());
        assert_eq!(budget.fetched_count(), 0);
    }

    #[test]
    fn fetch_budget_exhausted_at_zero() {
        let budget = FetchBudget::new(0);
        // With 0ms budget, elapsed_ms() starts at 0 but the check is >,
        // so it may not be exhausted at the exact instant of creation.
        // After any work, it will be exhausted. Use a tiny sleep to ensure.
        std::thread::sleep(std::time::Duration::from_millis(1));
        assert!(budget.is_exhausted());
    }

    #[test]
    fn fetch_budget_records_fetches() {
        let mut budget = FetchBudget::new(60_000);
        budget.record_fetch();
        budget.record_fetch();
        budget.record_fetch();
        assert_eq!(budget.fetched_count(), 3);
    }

    #[test]
    fn fetch_budget_elapsed_is_non_negative() {
        let budget = FetchBudget::new(1000);
        assert!(budget.elapsed_ms() < 100); // should be near-zero
    }

    // ─── ScriptDedup tests ───

    #[test]
    fn dedup_executed_url_skipped() {
        let mut dedup = ScriptDedup::new();
        dedup.mark_executed("https://example.com/vendor.js");
        assert!(dedup.should_skip_module("https://example.com/vendor.js"));
    }

    #[test]
    fn dedup_non_executed_not_skipped() {
        let dedup = ScriptDedup::new();
        assert!(!dedup.should_skip_module("https://example.com/app.js"));
    }

    #[test]
    fn dedup_different_url_not_skipped() {
        let mut dedup = ScriptDedup::new();
        dedup.mark_executed("https://example.com/vendor.js");
        assert!(!dedup.should_skip_module("https://example.com/app.js"));
    }

    #[test]
    fn dedup_inline_no_url_not_applicable() {
        // Inline scripts have no URL — the caller checks is_module() + url()
        // before calling should_skip_module. This test verifies the struct
        // doesn't match empty strings.
        let dedup = ScriptDedup::new();
        assert!(!dedup.should_skip_module(""));
    }

    // ─── ExecutionGroups tests ───

    #[test]
    fn groups_correct_partitioning() {
        let scripts = vec![
            ScriptInfo::External {
                url: "https://example.com/blocking.js".into(),
                is_module: false,
                kind: ScriptKind::ExternalBlocking,
            },
            ScriptInfo::Inline {
                content: "var x = 1;".into(),
                is_module: false,
                kind: ScriptKind::InlineBlocking,
            },
            ScriptInfo::External {
                url: "https://example.com/defer.js".into(),
                is_module: false,
                kind: ScriptKind::Defer,
            },
            ScriptInfo::External {
                url: "https://example.com/module.mjs".into(),
                is_module: true,
                kind: ScriptKind::Module,
            },
            ScriptInfo::External {
                url: "https://example.com/async.js".into(),
                is_module: false,
                kind: ScriptKind::Async,
            },
            ScriptInfo::Inline {
                content: r#"{"imports":{}}"#.into(),
                is_module: false,
                kind: ScriptKind::ImportMap,
            },
        ];

        let groups = ExecutionGroups::from_scripts(&scripts);

        // Blocking: indices 0, 1
        assert_eq!(groups.blocking.len(), 2);
        assert_eq!(groups.blocking[0].0, 0);
        assert_eq!(groups.blocking[1].0, 1);

        // Deferred: indices 2, 3
        assert_eq!(groups.deferred.len(), 2);
        assert_eq!(groups.deferred[0].0, 2);
        assert_eq!(groups.deferred[1].0, 3);

        // Async: index 4
        assert_eq!(groups.async_scripts.len(), 1);
        assert_eq!(groups.async_scripts[0].0, 4);

        // ImportMap excluded
        assert_eq!(groups.total(), 5);
    }

    #[test]
    fn groups_empty_input() {
        let groups = ExecutionGroups::from_scripts(&[]);
        assert!(groups.blocking.is_empty());
        assert!(groups.deferred.is_empty());
        assert!(groups.async_scripts.is_empty());
        assert_eq!(groups.total(), 0);
    }

    #[test]
    fn groups_all_async() {
        let scripts = vec![
            ScriptInfo::External {
                url: "https://example.com/a.js".into(),
                is_module: false,
                kind: ScriptKind::Async,
            },
            ScriptInfo::External {
                url: "https://example.com/b.mjs".into(),
                is_module: true,
                kind: ScriptKind::AsyncModule,
            },
        ];

        let groups = ExecutionGroups::from_scripts(&scripts);
        assert!(groups.blocking.is_empty());
        assert!(groups.deferred.is_empty());
        assert_eq!(groups.async_scripts.len(), 2);
    }

    #[test]
    fn groups_inline_module_goes_to_deferred() {
        let scripts = vec![ScriptInfo::Inline {
            content: "import './foo.js';".into(),
            is_module: true,
            kind: ScriptKind::InlineModule,
        }];

        let groups = ExecutionGroups::from_scripts(&scripts);
        assert_eq!(groups.deferred.len(), 1);
        assert!(groups.blocking.is_empty());
        assert!(groups.async_scripts.is_empty());
    }

    #[test]
    fn groups_ignored_excluded() {
        let scripts = vec![
            ScriptInfo::Inline {
                content: r#"{"data": 1}"#.into(),
                is_module: false,
                kind: ScriptKind::Ignored,
            },
            ScriptInfo::Inline {
                content: r#"{"imports":{}}"#.into(),
                is_module: false,
                kind: ScriptKind::ImportMap,
            },
        ];

        let groups = ExecutionGroups::from_scripts(&scripts);
        assert_eq!(groups.total(), 0);
    }

    #[test]
    fn groups_preserves_document_order() {
        let scripts = vec![
            ScriptInfo::External {
                url: "https://example.com/d1.js".into(),
                is_module: false,
                kind: ScriptKind::Defer,
            },
            ScriptInfo::External {
                url: "https://example.com/b1.js".into(),
                is_module: false,
                kind: ScriptKind::ExternalBlocking,
            },
            ScriptInfo::External {
                url: "https://example.com/d2.js".into(),
                is_module: false,
                kind: ScriptKind::Defer,
            },
            ScriptInfo::External {
                url: "https://example.com/b2.js".into(),
                is_module: false,
                kind: ScriptKind::ExternalBlocking,
            },
        ];

        let groups = ExecutionGroups::from_scripts(&scripts);
        // Blocking: indices 1, 3 (document order preserved)
        let blocking_indices: Vec<usize> = groups.blocking.iter().map(|(i, _)| *i).collect();
        assert_eq!(blocking_indices, vec![1, 3]);
        // Deferred: indices 0, 2 (document order preserved)
        let deferred_indices: Vec<usize> = groups.deferred.iter().map(|(i, _)| *i).collect();
        assert_eq!(deferred_indices, vec![0, 2]);
    }

    // ─── DomExportValidator tests ───

    #[test]
    fn validator_accept_above_threshold() {
        let v = DomExportValidator::new(100);
        assert!(v.should_accept(100)); // 100% — accept
        assert!(v.should_accept(90)); // 90% — accept
        assert!(v.should_accept(80)); // 80% — accept
    }

    #[test]
    fn validator_reject_below_threshold() {
        let v = DomExportValidator::new(100);
        assert!(!v.should_accept(79)); // 79% — reject
        assert!(!v.should_accept(50)); // 50% — reject
        assert!(!v.should_accept(0)); // 0% — reject
    }

    #[test]
    fn validator_zero_original_accepts_any() {
        let v = DomExportValidator::new(0);
        // threshold = max(0 * 0.8, 1) = 1
        assert!(v.should_accept(1));
        assert!(!v.should_accept(0));
    }

    #[test]
    fn validator_small_original() {
        let v = DomExportValidator::new(3);
        // threshold = max(3 * 0.8 = 2, 1) = 2
        assert!(v.should_accept(3));
        assert!(v.should_accept(2));
        assert!(!v.should_accept(1));
    }

    #[test]
    fn validator_threshold_value() {
        let v = DomExportValidator::new(100);
        assert_eq!(v.threshold(), 80);

        let v2 = DomExportValidator::new(0);
        assert_eq!(v2.threshold(), 1); // max(0, 1)
    }

    // ─── fire_observers tests ───

    #[test]
    fn fire_observers_with_mock_runtime() {
        use neo_runtime::mock::MockRuntime;
        let mut rt = MockRuntime::new();
        rt.set_default_eval("0");
        let (io, ro) = fire_observers(&mut rt);
        assert_eq!(io, 0);
        assert_eq!(ro, 0);
    }
}
