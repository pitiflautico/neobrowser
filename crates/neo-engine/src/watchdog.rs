//! Watchdog — per-script timeout, page budget, and abort propagation.
//!
//! Each script execution gets a configurable timeout (default 3s). If exceeded,
//! the V8 isolate is terminated via `terminate_execution`. When the watchdog fires:
//!
//! 1. Timers abort flag is set (R8a) — all pending timers cancelled
//! 2. Fetches abort flag is set (R8b) — all pending fetches rejected
//! 3. An `WatchdogEvent` is recorded with the reason
//!
//! A total page budget (default 6s) caps cumulative script execution time.
//! A microtask starvation guard (default 500ms) catches infinite promise chains.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use neo_runtime::RuntimeHandle;

// ─── WatchdogEvent ───

/// Reason the watchdog triggered abort propagation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum WatchdogAbortReason {
    /// A single script exceeded its per-script timeout.
    WatchdogTimeout {
        /// How long the script ran before termination (ms).
        elapsed_ms: u64,
        /// Configured timeout (ms).
        budget_ms: u64,
    },
    /// Total page script budget exhausted.
    PageBudgetExhausted {
        /// Total elapsed page script time (ms).
        elapsed_ms: u64,
        /// Configured page budget (ms).
        budget_ms: u64,
    },
    /// V8 microtask loop ran too long without yielding to macrotasks.
    MicrotaskStarvation {
        /// How long the microtask loop ran (ms).
        elapsed_ms: u64,
        /// Configured starvation limit (ms).
        limit_ms: u64,
    },
}

/// Recorded when the watchdog triggers abort propagation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WatchdogEvent {
    /// Why the abort was triggered.
    pub reason: WatchdogAbortReason,
    /// Monotonic timestamp (ms since page start).
    pub timestamp_ms: u64,
}

// ─── Watchdog ───

/// Per-page watchdog that enforces script timeouts and propagates aborts.
///
/// Create one per page navigation. Each `start_script()` call returns a
/// [`WatchdogGuard`] that spawns a background thread to terminate V8 if
/// the script exceeds its budget. Dropping the guard cancels the watchdog.
pub struct Watchdog {
    /// Per-script timeout.
    script_timeout: Duration,
    /// Total page budget for all scripts combined.
    page_budget: Duration,
    /// Microtask starvation limit.
    microtask_limit: Duration,
    /// When the page started (for page budget tracking).
    page_start: Instant,
    /// Cumulative script execution time across all scripts.
    page_elapsed: Duration,
    /// Shared abort flag for timers (R8a).
    abort_timers: Option<Arc<AtomicBool>>,
    /// Shared abort flag for fetches (R8b).
    abort_fetches: Option<Arc<AtomicBool>>,
    /// Recorded abort events.
    events: Vec<WatchdogEvent>,
}

impl Watchdog {
    /// Create a new watchdog with default timeouts (3s script, 6s page, 500ms microtask).
    pub fn new() -> Self {
        Self {
            script_timeout: Duration::from_secs(3),
            page_budget: Duration::from_secs(6),
            microtask_limit: Duration::from_millis(500),
            page_start: Instant::now(),
            page_elapsed: Duration::ZERO,
            abort_timers: None,
            abort_fetches: None,
            events: Vec::new(),
        }
    }

    /// Create a watchdog with custom timeouts.
    pub fn with_timeouts(
        script_timeout: Duration,
        page_budget: Duration,
        microtask_limit: Duration,
    ) -> Self {
        Self {
            script_timeout,
            page_budget,
            microtask_limit,
            page_start: Instant::now(),
            page_elapsed: Duration::ZERO,
            abort_timers: None,
            abort_fetches: None,
            events: Vec::new(),
        }
    }

    /// Connect the timer abort flag from R8a's `TimerState`.
    pub fn connect_timer_abort(&mut self, flag: Arc<AtomicBool>) {
        self.abort_timers = Some(flag);
    }

    /// Connect the fetch abort flag from R8b's `FetchBudget`.
    pub fn connect_fetch_abort(&mut self, flag: Arc<AtomicBool>) {
        self.abort_fetches = Some(flag);
    }

    /// Check whether there is remaining page budget for another script.
    pub fn has_budget(&self) -> bool {
        self.page_elapsed < self.page_budget
    }

    /// Remaining page budget.
    pub fn remaining_budget(&self) -> Duration {
        self.page_budget.saturating_sub(self.page_elapsed)
    }

    /// Cumulative script execution time so far.
    pub fn page_elapsed(&self) -> Duration {
        self.page_elapsed
    }

    /// Microtask starvation limit.
    pub fn microtask_limit(&self) -> Duration {
        self.microtask_limit
    }

    /// Recorded abort events from this page.
    pub fn events(&self) -> &[WatchdogEvent] {
        &self.events
    }

    /// Start a watchdog guard for one script execution.
    ///
    /// The guard spawns a background thread that will call
    /// `terminate_execution()` on the V8 isolate if the script runs
    /// longer than `script_timeout`. Dropping the guard cancels the thread.
    ///
    /// Returns `None` if page budget is already exhausted.
    pub fn start_script(&self, handle: RuntimeHandle) -> Option<WatchdogGuard> {
        if !self.has_budget() {
            return None;
        }

        // Use the lesser of script timeout and remaining page budget.
        let effective_timeout = self.script_timeout.min(self.remaining_budget());

        let cancelled = Arc::new(AtomicBool::new(false));
        let cancelled_clone = cancelled.clone();
        let fired = Arc::new(AtomicBool::new(false));
        let fired_clone = fired.clone();

        let thread = std::thread::Builder::new()
            .name("neo-watchdog".into())
            .spawn(move || {
                // Sleep in small increments so we can check the cancel flag.
                let start = Instant::now();
                let check_interval = Duration::from_millis(50);
                while start.elapsed() < effective_timeout {
                    if cancelled_clone.load(Ordering::Acquire) {
                        return;
                    }
                    let remaining = effective_timeout.saturating_sub(start.elapsed());
                    std::thread::sleep(remaining.min(check_interval));
                }
                // Timeout reached — terminate V8.
                if !cancelled_clone.load(Ordering::Acquire) {
                    fired_clone.store(true, Ordering::Release);
                    handle.terminate();
                }
            });

        match thread {
            Ok(join_handle) => Some(WatchdogGuard {
                cancelled,
                fired,
                join_handle: Some(join_handle),
                start: Instant::now(),
                timeout: effective_timeout,
            }),
            Err(_) => {
                // Thread spawn failed — run without watchdog rather than panic.
                None
            }
        }
    }

    /// Record that a script finished execution and update page budget.
    ///
    /// If the watchdog fired, propagates abort to timers and fetches.
    /// Returns the `WatchdogEvent` if one was generated.
    pub fn finish_script(&mut self, guard: WatchdogGuard) -> Option<WatchdogEvent> {
        let elapsed = guard.elapsed();
        let watchdog_fired = guard.fired();

        // Cancel the watchdog thread (no-op if already fired).
        drop(guard);

        self.page_elapsed += elapsed;

        if watchdog_fired {
            let event = WatchdogEvent {
                reason: WatchdogAbortReason::WatchdogTimeout {
                    elapsed_ms: elapsed.as_millis() as u64,
                    budget_ms: self.script_timeout.as_millis() as u64,
                },
                timestamp_ms: self.page_start.elapsed().as_millis() as u64,
            };
            self.propagate_abort();
            self.events.push(event.clone());
            return Some(event);
        }

        // Check if page budget is now exhausted.
        if !self.has_budget() {
            let event = WatchdogEvent {
                reason: WatchdogAbortReason::PageBudgetExhausted {
                    elapsed_ms: self.page_elapsed.as_millis() as u64,
                    budget_ms: self.page_budget.as_millis() as u64,
                },
                timestamp_ms: self.page_start.elapsed().as_millis() as u64,
            };
            self.propagate_abort();
            self.events.push(event.clone());
            return Some(event);
        }

        None
    }

    /// Record a microtask starvation event and propagate abort.
    pub fn record_microtask_starvation(&mut self, elapsed: Duration) {
        let event = WatchdogEvent {
            reason: WatchdogAbortReason::MicrotaskStarvation {
                elapsed_ms: elapsed.as_millis() as u64,
                limit_ms: self.microtask_limit.as_millis() as u64,
            },
            timestamp_ms: self.page_start.elapsed().as_millis() as u64,
        };
        self.propagate_abort();
        self.events.push(event);
    }

    /// Propagate abort to all connected subsystems.
    ///
    /// Cancel order: timers -> fetches (documented contract).
    fn propagate_abort(&self) {
        // 1. Abort timers (includes intervals).
        if let Some(ref flag) = self.abort_timers {
            flag.store(true, Ordering::Release);
        }
        // 2. Abort fetches.
        if let Some(ref flag) = self.abort_fetches {
            flag.store(true, Ordering::Release);
        }
    }

    /// Reset for a new page navigation.
    pub fn reset(&mut self) {
        self.page_start = Instant::now();
        self.page_elapsed = Duration::ZERO;
        self.events.clear();
    }
}

impl Default for Watchdog {
    fn default() -> Self {
        Self::new()
    }
}

// ─── WatchdogGuard ───

/// RAII guard for a single script execution.
///
/// When dropped, cancels the watchdog thread if it hasn't already fired.
/// The guard tracks elapsed time and whether the watchdog triggered.
pub struct WatchdogGuard {
    /// Set to true to cancel the watchdog thread.
    cancelled: Arc<AtomicBool>,
    /// Set to true by the watchdog thread when it fires.
    fired: Arc<AtomicBool>,
    /// The watchdog thread handle.
    join_handle: Option<std::thread::JoinHandle<()>>,
    /// When the script started.
    start: Instant,
    /// Configured timeout for this script.
    timeout: Duration,
}

impl WatchdogGuard {
    /// How long the script has been running.
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    /// Whether the watchdog fired (script was terminated).
    pub fn fired(&self) -> bool {
        self.fired.load(Ordering::Acquire)
    }

    /// The effective timeout for this script.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }
}

impl Drop for WatchdogGuard {
    fn drop(&mut self) {
        // Signal the watchdog thread to stop.
        self.cancelled.store(true, Ordering::Release);
        // Wait for the thread to finish (should be fast once cancelled).
        if let Some(handle) = self.join_handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_watchdog_has_budget_initially() {
        let wd = Watchdog::new();
        assert!(wd.has_budget());
        assert_eq!(wd.remaining_budget(), Duration::from_secs(6));
        assert_eq!(wd.page_elapsed(), Duration::ZERO);
    }

    #[test]
    fn test_watchdog_custom_timeouts() {
        let wd = Watchdog::with_timeouts(
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_millis(200),
        );
        assert!(wd.has_budget());
        assert_eq!(wd.remaining_budget(), Duration::from_secs(2));
    }

    #[test]
    fn test_abort_propagation_sets_flags() {
        let timer_flag = Arc::new(AtomicBool::new(false));
        let fetch_flag = Arc::new(AtomicBool::new(false));

        let mut wd = Watchdog::new();
        wd.connect_timer_abort(timer_flag.clone());
        wd.connect_fetch_abort(fetch_flag.clone());

        assert!(!timer_flag.load(Ordering::Acquire));
        assert!(!fetch_flag.load(Ordering::Acquire));

        // Simulate propagation.
        wd.propagate_abort();

        assert!(timer_flag.load(Ordering::Acquire));
        assert!(fetch_flag.load(Ordering::Acquire));
    }

    #[test]
    fn test_page_budget_tracking() {
        let mut wd = Watchdog::with_timeouts(
            Duration::from_secs(3),
            Duration::from_millis(100),
            Duration::from_millis(500),
        );
        // Simulate consuming budget.
        wd.page_elapsed = Duration::from_millis(50);
        assert!(wd.has_budget());
        assert_eq!(wd.remaining_budget(), Duration::from_millis(50));

        wd.page_elapsed = Duration::from_millis(100);
        assert!(!wd.has_budget());
        assert_eq!(wd.remaining_budget(), Duration::ZERO);
    }

    #[test]
    fn test_microtask_starvation_recorded() {
        let timer_flag = Arc::new(AtomicBool::new(false));
        let fetch_flag = Arc::new(AtomicBool::new(false));

        let mut wd = Watchdog::new();
        wd.connect_timer_abort(timer_flag.clone());
        wd.connect_fetch_abort(fetch_flag.clone());

        wd.record_microtask_starvation(Duration::from_millis(600));

        assert_eq!(wd.events().len(), 1);
        match &wd.events()[0].reason {
            WatchdogAbortReason::MicrotaskStarvation {
                elapsed_ms,
                limit_ms,
            } => {
                assert_eq!(*elapsed_ms, 600);
                assert_eq!(*limit_ms, 500);
            }
            other => panic!("expected MicrotaskStarvation, got {other:?}"),
        }
        // Abort flags should be set.
        assert!(timer_flag.load(Ordering::Acquire));
        assert!(fetch_flag.load(Ordering::Acquire));
    }

    #[test]
    fn test_watchdog_reset() {
        let mut wd = Watchdog::new();
        wd.page_elapsed = Duration::from_secs(7); // exceed 6s page budget
        wd.record_microtask_starvation(Duration::from_millis(600));
        assert!(!wd.has_budget());
        assert!(!wd.events().is_empty());

        wd.reset();
        assert!(wd.has_budget());
        assert!(wd.events().is_empty());
        assert_eq!(wd.page_elapsed(), Duration::ZERO);
    }

    #[test]
    fn test_abort_reason_variants() {
        let r1 = WatchdogAbortReason::WatchdogTimeout {
            elapsed_ms: 3100,
            budget_ms: 3000,
        };
        let r2 = WatchdogAbortReason::PageBudgetExhausted {
            elapsed_ms: 6200,
            budget_ms: 6000,
        };
        let r3 = WatchdogAbortReason::MicrotaskStarvation {
            elapsed_ms: 550,
            limit_ms: 500,
        };
        // Just verify they serialize.
        assert!(serde_json::to_string(&r1).is_ok());
        assert!(serde_json::to_string(&r2).is_ok());
        assert!(serde_json::to_string(&r3).is_ok());
    }

    #[test]
    fn test_no_guard_when_budget_exhausted() {
        let wd = Watchdog::with_timeouts(
            Duration::from_secs(3),
            Duration::ZERO, // no budget
            Duration::from_millis(500),
        );
        // Cannot create a mock RuntimeHandle without V8, but we can verify
        // has_budget returns false.
        assert!(!wd.has_budget());
    }

    #[test]
    fn test_finish_script_page_budget_exhaustion() {
        let timer_flag = Arc::new(AtomicBool::new(false));
        let mut wd = Watchdog::with_timeouts(
            Duration::from_secs(3),
            Duration::from_millis(100),
            Duration::from_millis(500),
        );
        wd.connect_timer_abort(timer_flag.clone());

        // Manually set page_elapsed near limit.
        wd.page_elapsed = Duration::from_millis(99);

        // Simulate a finish_script that pushes over budget.
        // We can't use a real guard without V8, so test the budget logic directly.
        wd.page_elapsed += Duration::from_millis(2);
        assert!(!wd.has_budget());
    }
}
