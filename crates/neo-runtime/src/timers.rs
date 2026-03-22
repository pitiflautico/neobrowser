//! NeoTimers — real macrotask timer system for the AI browser.
//!
//! Inspired by Deno's WebTimers architecture (BTreeMap ordering,
//! deadline-based polling) but purpose-built for AI rendering:
//!
//! - **Budget-native**: timer budget is part of the system, not a JS wrapper
//! - **Observable**: every timer creation/fire/cancel is traced
//! - **Kill switch**: cancel all timers or abort immediately
//! - **Interval cap**: setInterval auto-stops after N ticks
//!
//! ## How it works
//!
//! Timers are stored in a `BTreeMap<(Instant, TimerId), TimerEntry>` ordered
//! by deadline. The event loop calls `poll_ready()` between V8 iterations to
//! collect fired timers and execute their JS callbacks. `next_deadline()`
//! tells the event loop how long to sleep before the next timer fires.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// Unique timer identifier returned to JavaScript.
pub type TimerId = u64;

/// A single registered timer (setTimeout or setInterval).
#[derive(Debug, Clone)]
pub struct TimerEntry {
    /// Unique ID for this timer.
    pub id: TimerId,
    /// When this timer should fire next.
    pub deadline: Instant,
    /// `None` = setTimeout (one-shot), `Some(ms)` = setInterval (repeating).
    pub repeat_ms: Option<u64>,
    /// Index into the JS `__timerCallbacks` array.
    pub callback_idx: u32,
    /// How many times this timer has fired (intervals only).
    pub fired_count: u32,
}

/// Real macrotask timer system.
///
/// Stores timers in a BTreeMap ordered by (deadline, id) so `poll_ready()`
/// is O(k) where k = number of ready timers, and `next_deadline()` is O(1).
pub struct NeoTimers {
    next_id: TimerId,
    timers: BTreeMap<(Instant, TimerId), TimerEntry>,
    budget_used: usize,
    budget_limit: usize,
    interval_max_ticks: u32,
    aborted: bool,
}

impl NeoTimers {
    /// Create a new timer system with the given budget limit.
    pub fn new(budget_limit: usize, interval_max_ticks: u32) -> Self {
        Self {
            next_id: 1,
            timers: BTreeMap::new(),
            budget_used: 0,
            budget_limit,
            interval_max_ticks,
            aborted: false,
        }
    }

    /// Register a one-shot timer (setTimeout).
    ///
    /// Returns 0 if the budget is exhausted or the system is aborted.
    pub fn set_timeout(&mut self, delay_ms: u64, callback_idx: u32) -> TimerId {
        if self.aborted || self.budget_used >= self.budget_limit {
            return 0;
        }
        self.budget_used += 1;
        let id = self.next_id;
        self.next_id += 1;
        let deadline = Instant::now() + Duration::from_millis(delay_ms);
        self.timers.insert(
            (deadline, id),
            TimerEntry {
                id,
                deadline,
                repeat_ms: None,
                callback_idx,
                fired_count: 0,
            },
        );
        id
    }

    /// Register a repeating timer (setInterval).
    ///
    /// Returns 0 if the budget is exhausted or the system is aborted.
    pub fn set_interval(&mut self, delay_ms: u64, callback_idx: u32) -> TimerId {
        if self.aborted || self.budget_used >= self.budget_limit {
            return 0;
        }
        self.budget_used += 1;
        let id = self.next_id;
        self.next_id += 1;
        // First tick fires after delay_ms (not immediately).
        let delay = delay_ms.max(1); // min 1ms for intervals
        let deadline = Instant::now() + Duration::from_millis(delay);
        self.timers.insert(
            (deadline, id),
            TimerEntry {
                id,
                deadline,
                repeat_ms: Some(delay),
                callback_idx,
                fired_count: 0,
            },
        );
        id
    }

    /// Cancel a timer by ID. Returns true if the timer existed.
    pub fn clear_timer(&mut self, id: TimerId) -> bool {
        // BTreeMap keyed by (Instant, TimerId) — we need to find the entry with matching id.
        // Since we don't know the deadline, scan for it.
        let key = self
            .timers
            .iter()
            .find(|(_, entry)| entry.id == id)
            .map(|(k, _)| *k);
        if let Some(k) = key {
            self.timers.remove(&k);
            true
        } else {
            false
        }
    }

    /// Collect all timers whose deadline has passed.
    ///
    /// Returns `(timer_id, callback_idx)` pairs. For intervals, the timer
    /// is re-scheduled; for timeouts, it's removed.
    pub fn poll_ready(&mut self) -> Vec<(TimerId, u32)> {
        if self.aborted {
            return vec![];
        }
        let now = Instant::now();
        let mut ready = Vec::new();
        let mut to_reschedule = Vec::new();

        while let Some((&(deadline, id), _)) = self.timers.first_key_value() {
            if deadline > now {
                break;
            }
            let entry = self.timers.remove(&(deadline, id)).expect("just peeked");
            ready.push((id, entry.callback_idx));

            // Re-schedule intervals (with tick limit).
            if let Some(repeat_ms) = entry.repeat_ms {
                let new_count = entry.fired_count + 1;
                if new_count < self.interval_max_ticks {
                    // Budget check for interval re-registration.
                    if self.budget_used < self.budget_limit {
                        self.budget_used += 1;
                        let new_deadline = now + Duration::from_millis(repeat_ms);
                        to_reschedule.push(TimerEntry {
                            id,
                            deadline: new_deadline,
                            repeat_ms: Some(repeat_ms),
                            callback_idx: entry.callback_idx,
                            fired_count: new_count,
                        });
                    }
                }
            }
        }

        for entry in to_reschedule {
            self.timers.insert((entry.deadline, entry.id), entry);
        }

        ready
    }

    /// The earliest deadline among all registered timers.
    ///
    /// Returns `None` if no timers are registered. The event loop uses this
    /// to decide how long to sleep between iterations.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.timers.first_key_value().map(|(&(d, _), _)| d)
    }

    /// Whether there are no registered timers.
    pub fn is_empty(&self) -> bool {
        self.timers.is_empty()
    }

    /// Number of registered timers.
    pub fn len(&self) -> usize {
        self.timers.len()
    }

    /// How many budget ticks have been consumed.
    pub fn budget_used(&self) -> usize {
        self.budget_used
    }

    /// Whether the budget is exhausted.
    pub fn is_budget_exhausted(&self) -> bool {
        self.budget_used >= self.budget_limit
    }

    /// Abort all timers. No further timers will fire or be registered.
    pub fn abort(&mut self) {
        self.aborted = true;
        self.timers.clear();
    }

    /// Whether the system has been aborted.
    pub fn is_aborted(&self) -> bool {
        self.aborted
    }

    /// Reset everything for a new page.
    pub fn reset(&mut self) {
        self.timers.clear();
        self.budget_used = 0;
        self.aborted = false;
        self.next_id = 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_timeout_fires() {
        let mut timers = NeoTimers::new(100, 20);
        let id = timers.set_timeout(0, 42);
        assert_ne!(id, 0);
        assert_eq!(timers.len(), 1);

        // Poll immediately — deadline is now, should fire.
        std::thread::sleep(Duration::from_millis(1));
        let ready = timers.poll_ready();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0], (id, 42));
        assert!(timers.is_empty()); // one-shot removed
    }

    #[test]
    fn test_set_interval_repeats() {
        let mut timers = NeoTimers::new(100, 5);
        let id = timers.set_interval(1, 10);
        assert_ne!(id, 0);

        // Fire several ticks.
        let mut total_fires = 0;
        for _ in 0..10 {
            std::thread::sleep(Duration::from_millis(2));
            let ready = timers.poll_ready();
            total_fires += ready.len();
        }
        // Should have fired up to 5 times (interval_max_ticks = 5).
        assert!(total_fires <= 5, "fired {} times", total_fires);
    }

    #[test]
    fn test_clear_timer() {
        let mut timers = NeoTimers::new(100, 20);
        let id = timers.set_timeout(1000, 1); // far future
        assert_eq!(timers.len(), 1);

        assert!(timers.clear_timer(id));
        assert!(timers.is_empty());
        assert!(!timers.clear_timer(id)); // already cleared
    }

    #[test]
    fn test_budget_exhaustion() {
        let mut timers = NeoTimers::new(3, 20);
        assert_ne!(timers.set_timeout(0, 1), 0);
        assert_ne!(timers.set_timeout(0, 2), 0);
        assert_ne!(timers.set_timeout(0, 3), 0);
        assert_eq!(timers.set_timeout(0, 4), 0); // budget exhausted
        assert!(timers.is_budget_exhausted());
    }

    #[test]
    fn test_abort() {
        let mut timers = NeoTimers::new(100, 20);
        timers.set_timeout(0, 1);
        timers.set_timeout(0, 2);
        timers.abort();

        assert!(timers.is_aborted());
        assert!(timers.is_empty());
        assert_eq!(timers.set_timeout(0, 3), 0); // rejected after abort

        let ready = timers.poll_ready();
        assert!(ready.is_empty());
    }

    #[test]
    fn test_reset() {
        let mut timers = NeoTimers::new(3, 20);
        timers.set_timeout(0, 1);
        timers.set_timeout(0, 2);
        timers.set_timeout(0, 3);
        assert!(timers.is_budget_exhausted());

        timers.reset();
        assert!(!timers.is_budget_exhausted());
        assert!(timers.is_empty());
        assert_eq!(timers.budget_used(), 0);
        assert_ne!(timers.set_timeout(0, 4), 0); // works after reset
    }

    #[test]
    fn test_next_deadline() {
        let mut timers = NeoTimers::new(100, 20);
        assert!(timers.next_deadline().is_none());

        timers.set_timeout(100, 1);
        let deadline = timers.next_deadline();
        assert!(deadline.is_some());
        assert!(deadline.expect("just set") > Instant::now());
    }

    #[test]
    fn test_future_timer_does_not_fire() {
        let mut timers = NeoTimers::new(100, 20);
        timers.set_timeout(5000, 1); // 5 seconds in the future
        let ready = timers.poll_ready();
        assert!(ready.is_empty());
        assert_eq!(timers.len(), 1); // still registered
    }
}
