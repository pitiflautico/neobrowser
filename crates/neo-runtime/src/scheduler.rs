//! Event loop management — tracks pending tasks and drives V8 to settlement.
//!
//! Handles microtasks (promises), macrotasks (timers), and fetch operations.
//! The scheduler polls V8's event loop and checks pending counts with timeout.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

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
}
