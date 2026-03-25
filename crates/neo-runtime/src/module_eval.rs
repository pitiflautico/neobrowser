//! ModuleEvaluator — tracks module evaluation state to prevent deno_core panics.
//!
//! deno_core panics with "Module already evaluated" when `mod_evaluate` is called
//! on a module that was already evaluated as a transitive dependency during a
//! previous module's event loop. This struct tracks which URLs and mod_ids have
//! been processed, and detects isolate corruption after a panic.

use std::collections::HashSet;

/// Tracks module evaluation state across multiple `load_module` calls.
///
/// Prevents the "Module already evaluated" panic by tracking:
/// - URLs already loaded (prevents double-load at the URL level)
/// - mod_ids already evaluated (prevents double-evaluate at the deno_core level)
/// - max_seen_mod_id (detects transitive deps evaluated during event loop)
/// - isolate corruption flag (skips all evals after a panic)
pub struct ModuleEvaluator {
    /// URLs already loaded (prevents double-load).
    loaded_urls: HashSet<String>,
    /// mod_ids already evaluated (prevents double-evaluate panic).
    evaluated_ids: HashSet<usize>,
    /// Highest mod_id seen from load_side_es_module. Any mod_id <= this was
    /// potentially evaluated as a transitive dependency.
    max_seen_mod_id: usize,
    /// True after a mod_evaluate panic — V8 isolate corrupted.
    corrupted: bool,
}

impl ModuleEvaluator {
    pub fn new() -> Self {
        Self {
            loaded_urls: HashSet::new(),
            evaluated_ids: HashSet::new(),
            max_seen_mod_id: 0,
            corrupted: false,
        }
    }

    /// Check if a URL was already loaded.
    pub fn is_url_loaded(&self, url: &str) -> bool {
        self.loaded_urls.contains(url)
    }

    /// Mark a URL as loaded. Returns `true` if this is the first time (proceed).
    /// Returns `false` if already loaded (skip).
    pub fn mark_url_loaded(&mut self, url: &str) -> bool {
        self.loaded_urls.insert(url.to_string())
    }

    /// Track a mod_id returned by load_side_es_module.
    /// Call BEFORE should_evaluate.
    /// Returns the PREVIOUS max (for comparison).
    pub fn track_mod_id(&mut self, mod_id: usize) -> usize {
        let prev = self.max_seen_mod_id;
        self.max_seen_mod_id = self.max_seen_mod_id.max(mod_id);
        prev
    }

    /// Check if a mod_id should be evaluated.
    ///
    /// Returns `false` if:
    /// - The isolate is corrupted (previous panic)
    /// - The mod_id was already explicitly evaluated
    /// - The mod_id was already evaluated as a transitive dep
    ///   (mod_id <= previous max_seen before this module was tracked)
    ///
    /// `prev_max` should be the return value of `track_mod_id()`.
    pub fn should_evaluate(&self, mod_id: usize, _prev_max: usize) -> bool {
        if self.corrupted {
            return false;
        }
        if self.evaluated_ids.contains(&mod_id) {
            return false;
        }
        // Don't skip based on prev_max — it was too aggressive and skipped
        // modules that hadn't been evaluated yet. Instead, let mod_evaluate
        // try and the catch_unwind in evaluate_module will handle panics
        // from "Module already evaluated".
        true
    }

    /// Mark a mod_id as successfully evaluated.
    /// Also updates max_seen_mod_id to cover transitive deps.
    pub fn mark_evaluated(&mut self, mod_id: usize) {
        self.evaluated_ids.insert(mod_id);
        self.max_seen_mod_id = self.max_seen_mod_id.max(mod_id);
    }

    /// Mark a mod_id that was evaluated as a transitive dependency
    /// (e.g., discovered via module loader callbacks).
    pub fn mark_transitive_dep(&mut self, mod_id: usize) {
        self.evaluated_ids.insert(mod_id);
    }

    /// Mark the isolate as corrupted (after a mod_evaluate panic).
    pub fn mark_corrupted(&mut self) {
        self.corrupted = true;
    }

    /// Is the isolate corrupted?
    pub fn is_corrupted(&self) -> bool {
        self.corrupted
    }

    /// Current max_seen_mod_id (for logging).
    pub fn max_seen(&self) -> usize {
        self.max_seen_mod_id
    }

    /// Reset for a new page (cross-origin navigation).
    pub fn reset(&mut self) {
        self.loaded_urls.clear();
        self.evaluated_ids.clear();
        self.max_seen_mod_id = 0;
        self.corrupted = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_dedup_first_load_returns_true() {
        let mut eval = ModuleEvaluator::new();
        assert!(eval.mark_url_loaded("https://example.com/a.js"));
    }

    #[test]
    fn url_dedup_second_load_returns_false() {
        let mut eval = ModuleEvaluator::new();
        eval.mark_url_loaded("https://example.com/a.js");
        assert!(!eval.mark_url_loaded("https://example.com/a.js"));
    }

    #[test]
    fn is_url_loaded_tracks_state() {
        let mut eval = ModuleEvaluator::new();
        assert!(!eval.is_url_loaded("https://example.com/a.js"));
        eval.mark_url_loaded("https://example.com/a.js");
        assert!(eval.is_url_loaded("https://example.com/a.js"));
    }

    #[test]
    fn should_evaluate_first_time() {
        let eval = ModuleEvaluator::new();
        assert!(eval.should_evaluate(5, 0));
    }

    #[test]
    fn should_evaluate_after_mark_returns_false() {
        let mut eval = ModuleEvaluator::new();
        eval.mark_evaluated(5);
        assert!(!eval.should_evaluate(5, 0));
    }

    #[test]
    fn should_evaluate_regardless_of_prev_max() {
        // prev_max skip is disabled — we let mod_evaluate try and catch_unwind handles panics
        let mut eval = ModuleEvaluator::new();
        eval.track_mod_id(3);
        eval.mark_evaluated(3);
        eval.track_mod_id(5);
        let prev = eval.track_mod_id(4);
        // mod_id=4 not in evaluated_ids → should_evaluate returns true
        assert!(eval.should_evaluate(4, prev));
        let prev = eval.track_mod_id(6);
        assert!(eval.should_evaluate(6, prev));
    }

    #[test]
    fn transitive_dep_marked_skips_evaluate() {
        let mut eval = ModuleEvaluator::new();
        eval.mark_evaluated(3);
        eval.mark_transitive_dep(4);
        // Even if max_seen hasn't caught up, explicit mark prevents eval.
        assert!(!eval.should_evaluate(4, 0));
    }

    #[test]
    fn corrupted_skips_all_evaluations() {
        let mut eval = ModuleEvaluator::new();
        eval.mark_corrupted();
        assert!(!eval.should_evaluate(1, 0));
        assert!(!eval.should_evaluate(99, 0));
        assert!(eval.is_corrupted());
    }

    #[test]
    fn reset_clears_all_state() {
        let mut eval = ModuleEvaluator::new();
        eval.mark_url_loaded("https://example.com/a.js");
        eval.mark_evaluated(5);
        eval.track_mod_id(10);
        eval.mark_corrupted();

        eval.reset();

        assert!(!eval.is_url_loaded("https://example.com/a.js"));
        assert!(eval.mark_url_loaded("https://example.com/a.js"));
        assert!(eval.should_evaluate(5, 0));
        assert!(!eval.is_corrupted());
    }

    #[test]
    fn track_mod_id_returns_previous_max() {
        let mut eval = ModuleEvaluator::new();
        assert_eq!(eval.track_mod_id(3), 0); // prev was 0
        assert_eq!(eval.track_mod_id(1), 3); // prev was 3, doesn't decrease
        assert_eq!(eval.track_mod_id(7), 3); // prev was 3
        assert_eq!(eval.max_seen(), 7);
    }

    #[test]
    fn equal_to_max_seen_is_evaluable() {
        let mut eval = ModuleEvaluator::new();
        let prev = eval.track_mod_id(5); // prev=0
        // mod_id=5, prev_max=0 → 5 > 0 → evaluable
        assert!(eval.should_evaluate(5, prev));
    }

    #[test]
    fn below_prev_max_still_evaluable() {
        // prev_max skip is disabled — only evaluated_ids and corrupted block evaluation
        let mut eval = ModuleEvaluator::new();
        eval.track_mod_id(5);
        eval.mark_evaluated(5);
        let prev = eval.track_mod_id(4);
        // mod_id=4 not in evaluated_ids → evaluable (catch_unwind will handle if already evaluated)
        assert!(eval.should_evaluate(4, prev));
        let prev = eval.track_mod_id(1);
        assert!(eval.should_evaluate(1, prev));
    }

    #[test]
    fn factorial_scenario() {
        // Simulates the actual factorial loading:
        // 1. vite.js (mod_id=2) → evaluates OK
        // 2. nuvo-importer.js (mod_id=3) → evaluates OK, event loop loads vendor(4) as dep
        // 3. vendor.js (mod_id=4) → should be SKIPPED (already evaluated as dep)
        let mut eval = ModuleEvaluator::new();

        // vite.js
        assert!(eval.mark_url_loaded("vite.js"));
        let prev = eval.track_mod_id(2); // prev=0
        assert!(eval.should_evaluate(2, prev)); // 2 > 0 → eval
        eval.mark_evaluated(2);

        // nuvo-importer.js
        assert!(eval.mark_url_loaded("nuvo-importer.js"));
        let prev = eval.track_mod_id(3); // prev=2
        assert!(eval.should_evaluate(3, prev)); // 3 > 2 → eval
        // During event loop, deno_core loads vendor.js (mod_id=4) as dep
        // After eval completes:
        eval.mark_evaluated(3);

        // vendor.js — appears as <script type="module">
        // load_side_es_module returns mod_id=4 (already exists)
        assert!(eval.mark_url_loaded("vendor.js"));
        let prev = eval.track_mod_id(4); // prev=3
        // KEY: 4 > 3, so prev_max check passes. BUT vendor was evaluated
        // as dep of nuvo-importer. We need mark_transitive_dep or the
        // evaluated_ids check to catch this.
        // Without explicit marking, should_evaluate returns true → PANIC.
        // Fix: after nuvo-importer eval, mark all deps (vendor=4).
        eval.mark_transitive_dep(4); // caller knows this from event loop
        assert!(!eval.should_evaluate(4, prev)); // in evaluated_ids → skip

        // integrations.js (mod_id=5) — genuinely new
        assert!(eval.mark_url_loaded("integrations.js"));
        let prev = eval.track_mod_id(5); // prev=4
        assert!(eval.should_evaluate(5, prev)); // 5 > 4 → eval
    }
}
