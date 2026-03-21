//! Event loop management — tracks pending tasks and drives V8 to settlement.
//!
//! Handles microtasks (promises), macrotasks (timers), and fetch operations.
//! The scheduler polls V8's event loop and checks pending counts with timeout.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

// ─── Scheduler Configuration ───

/// Configurable limits for the event loop scheduler.
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// Max ticks per setInterval call before auto-clear.
    pub interval_max_ticks: usize,
    /// Total timer ticks allowed per page before budget exhaustion.
    pub timer_budget: usize,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            interval_max_ticks: 20,
            timer_budget: 200,
        }
    }
}

// ─── Timer Budget ───

/// Tracks total timer ticks to enforce per-page budget.
///
/// When the budget is exhausted, new timer ticks are rejected
/// (setTimeout callbacks silently dropped, setInterval auto-cleared).
#[derive(Debug, Clone)]
pub struct TimerBudget {
    /// Total ticks consumed so far.
    used: Arc<AtomicUsize>,
    /// Maximum allowed ticks.
    limit: usize,
    /// Whether budget has been exhausted (sticky flag for fast checks).
    exhausted: Arc<AtomicBool>,
}

impl TimerBudget {
    /// Create a new timer budget with the given limit.
    pub fn new(limit: usize) -> Self {
        Self {
            used: Arc::new(AtomicUsize::new(0)),
            limit,
            exhausted: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Try to consume one tick. Returns true if within budget.
    pub fn tick(&self) -> bool {
        if self.exhausted.load(Ordering::Relaxed) {
            return false;
        }
        let prev = self.used.fetch_add(1, Ordering::Relaxed);
        if prev >= self.limit {
            self.exhausted.store(true, Ordering::Relaxed);
            false
        } else {
            true
        }
    }

    /// How many ticks remain.
    pub fn remaining(&self) -> usize {
        let used = self.used.load(Ordering::Relaxed);
        self.limit.saturating_sub(used)
    }

    /// Whether the budget is exhausted.
    pub fn is_exhausted(&self) -> bool {
        self.exhausted.load(Ordering::Relaxed)
    }

    /// Reset the budget (for reuse across pages).
    pub fn reset(&self) {
        self.used.store(0, Ordering::Relaxed);
        self.exhausted.store(false, Ordering::Relaxed);
    }
}

// ─── Task Tracker ───

/// Tracks pending async tasks in the runtime.
///
/// Each category is independently counted. The scheduler considers
/// the runtime "settled" when all categories reach zero.
#[derive(Debug, Clone)]
pub struct TaskTracker {
    /// Outstanding promise continuations.
    promises: Arc<AtomicUsize>,
    /// Active timers (setTimeout/setInterval).
    timers: Arc<AtomicUsize>,
    /// In-flight fetch requests.
    fetches: Arc<AtomicUsize>,
}

impl Default for TaskTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskTracker {
    /// Create a new task tracker with zero pending tasks.
    pub fn new() -> Self {
        Self {
            promises: Arc::new(AtomicUsize::new(0)),
            timers: Arc::new(AtomicUsize::new(0)),
            fetches: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Total number of pending tasks across all categories.
    pub fn pending(&self) -> usize {
        self.promises.load(Ordering::Relaxed)
            + self.timers.load(Ordering::Relaxed)
            + self.fetches.load(Ordering::Relaxed)
    }

    /// Whether the runtime is settled (no pending tasks).
    pub fn is_settled(&self) -> bool {
        self.pending() == 0
    }

    /// Increment the promise counter.
    pub fn add_promise(&self) {
        self.promises.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrement the promise counter.
    pub fn resolve_promise(&self) {
        let prev = self.promises.load(Ordering::Relaxed);
        if prev > 0 {
            self.promises.fetch_sub(1, Ordering::Relaxed);
        }
    }

    /// Increment the timer counter.
    pub fn add_timer(&self) {
        self.timers.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrement the timer counter.
    pub fn resolve_timer(&self) {
        let prev = self.timers.load(Ordering::Relaxed);
        if prev > 0 {
            self.timers.fetch_sub(1, Ordering::Relaxed);
        }
    }

    /// Number of active timers.
    pub fn timer_count(&self) -> usize {
        self.timers.load(Ordering::Relaxed)
    }

    /// Increment the fetch counter.
    pub fn add_fetch(&self) {
        self.fetches.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrement the fetch counter.
    pub fn resolve_fetch(&self) {
        let prev = self.fetches.load(Ordering::Relaxed);
        if prev > 0 {
            self.fetches.fetch_sub(1, Ordering::Relaxed);
        }
    }

    /// Reset all counters to zero.
    pub fn reset(&self) {
        self.promises.store(0, Ordering::Relaxed);
        self.timers.store(0, Ordering::Relaxed);
        self.fetches.store(0, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tracker_lifecycle() {
        let tracker = TaskTracker::new();
        assert!(tracker.is_settled());
        assert_eq!(tracker.pending(), 0);

        tracker.add_promise();
        tracker.add_timer();
        tracker.add_fetch();
        assert_eq!(tracker.pending(), 3);
        assert!(!tracker.is_settled());

        tracker.resolve_promise();
        tracker.resolve_timer();
        tracker.resolve_fetch();
        assert!(tracker.is_settled());
    }

    #[test]
    fn test_tracker_no_underflow() {
        let tracker = TaskTracker::new();
        tracker.resolve_promise();
        tracker.resolve_timer();
        tracker.resolve_fetch();
        assert_eq!(tracker.pending(), 0);
    }

    #[test]
    fn test_timer_budget_lifecycle() {
        let budget = TimerBudget::new(3);
        assert_eq!(budget.remaining(), 3);
        assert!(!budget.is_exhausted());

        assert!(budget.tick());
        assert!(budget.tick());
        assert!(budget.tick());
        assert!(!budget.tick()); // 4th tick exceeds budget
        assert!(budget.is_exhausted());
        assert_eq!(budget.remaining(), 0);
    }

    #[test]
    fn test_timer_budget_reset() {
        let budget = TimerBudget::new(2);
        assert!(budget.tick());
        assert!(budget.tick());
        assert!(!budget.tick());

        budget.reset();
        assert!(!budget.is_exhausted());
        assert_eq!(budget.remaining(), 2);
        assert!(budget.tick());
    }

    #[test]
    fn test_scheduler_config_defaults() {
        let cfg = SchedulerConfig::default();
        assert_eq!(cfg.interval_max_ticks, 20);
        assert_eq!(cfg.timer_budget, 200);
    }
}
