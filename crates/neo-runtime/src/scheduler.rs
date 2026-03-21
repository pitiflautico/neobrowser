//! Event loop management — tracks pending tasks and drives V8 to settlement.
//!
//! Handles microtasks (promises), macrotasks (timers), and fetch operations.
//! The scheduler polls V8's event loop and checks pending counts with timeout.
//!
//! ## Task ordering
//!
//! V8 guarantees that **microtasks** (promise continuations) drain completely
//! before any **macrotask** (timer callback, fetch completion) runs. This
//! module does not enforce that ordering itself — it relies on V8's built-in
//! microtask checkpoint. The [`TaskSnapshot`] exposes per-category counts so
//! callers can observe the ordering without touching atomics directly.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};

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

// ─── Fetch Budget (R8b) ───

/// Per-page fetch budget with concurrency limiting and abort support.
///
/// Enforces max concurrency (browser standard: 6), configurable per-request
/// timeout, and an abort flag that a watchdog can set to reject new fetches.
#[derive(Debug, Clone)]
pub struct FetchBudget {
    /// Maximum concurrent in-flight fetches (browser standard: 6).
    max_concurrent: usize,
    /// Timeout per individual fetch request in milliseconds.
    per_request_timeout_ms: u32,
    /// Total fetches initiated (lifetime counter, not decremented).
    total_fetches: Arc<AtomicUsize>,
    /// Currently in-flight fetches (incremented on start, decremented on finish).
    in_flight: Arc<AtomicUsize>,
    /// When true, all new fetches are immediately rejected.
    abort_flag: Arc<AtomicBool>,
    /// Network idle signal — true when in-flight reaches 0.
    idle: Arc<AtomicBool>,
}

impl FetchBudget {
    /// Create a new fetch budget with the given concurrency limit and timeout.
    pub fn new(max_concurrent: usize, per_request_timeout_ms: u32) -> Self {
        Self {
            max_concurrent,
            per_request_timeout_ms,
            total_fetches: Arc::new(AtomicUsize::new(0)),
            in_flight: Arc::new(AtomicUsize::new(0)),
            abort_flag: Arc::new(AtomicBool::new(false)),
            idle: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Check whether a new fetch can proceed (within concurrency budget and not aborted).
    pub fn can_fetch(&self) -> bool {
        if self.abort_flag.load(Ordering::Acquire) {
            return false;
        }
        self.in_flight.load(Ordering::Relaxed) < self.max_concurrent
    }

    /// Register a new fetch starting. Returns false if over budget or aborted.
    pub fn start_fetch(&self) -> bool {
        if self.abort_flag.load(Ordering::Acquire) {
            return false;
        }
        let current = self.in_flight.fetch_add(1, Ordering::Relaxed);
        if current >= self.max_concurrent {
            // Over budget — roll back.
            self.in_flight.fetch_sub(1, Ordering::Relaxed);
            return false;
        }
        self.total_fetches.fetch_add(1, Ordering::Relaxed);
        self.idle.store(false, Ordering::Release);
        true
    }

    /// Signal that a fetch has completed.
    pub fn finish_fetch(&self) {
        let prev = self.in_flight.load(Ordering::Relaxed);
        if prev > 0 {
            let before_sub = self.in_flight.fetch_sub(1, Ordering::Relaxed);
            // fetch_sub returns the value *before* subtraction.
            if before_sub == 1 {
                self.idle.store(true, Ordering::Release);
            }
        }
    }

    /// Per-request timeout in milliseconds.
    pub fn per_request_timeout_ms(&self) -> u32 {
        self.per_request_timeout_ms
    }

    /// Maximum concurrent fetches allowed.
    pub fn max_concurrent(&self) -> usize {
        self.max_concurrent
    }

    /// Total fetches initiated (lifetime counter).
    pub fn total_fetches(&self) -> usize {
        self.total_fetches.load(Ordering::Relaxed)
    }

    /// Currently in-flight fetch count.
    pub fn in_flight(&self) -> usize {
        self.in_flight.load(Ordering::Relaxed)
    }

    /// Whether the network is idle (no in-flight fetches).
    pub fn is_network_idle(&self) -> bool {
        self.idle.load(Ordering::Acquire)
    }

    /// Signal all fetches to abort. New fetches will be rejected immediately.
    pub fn abort(&self) {
        self.abort_flag.store(true, Ordering::Release);
    }

    /// Check whether the abort flag has been set.
    pub fn is_aborted(&self) -> bool {
        self.abort_flag.load(Ordering::Acquire)
    }

    /// Get a shared reference to the abort flag for external watchers (e.g. watchdog).
    pub fn abort_flag(&self) -> Arc<AtomicBool> {
        self.abort_flag.clone()
    }

    /// Reset the budget for reuse across pages.
    pub fn reset(&self) {
        self.total_fetches.store(0, Ordering::Relaxed);
        self.in_flight.store(0, Ordering::Relaxed);
        self.abort_flag.store(false, Ordering::Release);
        self.idle.store(true, Ordering::Release);
    }
}

impl Default for FetchBudget {
    /// Default: 6 concurrent, 5000ms per-request timeout.
    fn default() -> Self {
        Self::new(6, 5000)
    }
}

// ─── Timer State (R8a) ───

/// Shared timer state for nested clamping, abort, and monotonic timing.
///
/// Implements the HTML spec's nested timer clamping: when nesting depth >= 5,
/// the minimum delay is clamped to 4 ms. Also provides an abort flag that
/// a watchdog can set to cancel all pending timers immediately.
#[derive(Debug, Clone)]
pub struct TimerState {
    /// Current nesting depth of setTimeout/setInterval callbacks.
    nesting_depth: Arc<AtomicU32>,
    /// When set to `true`, all pending timers should bail out immediately.
    abort_flag: Arc<AtomicBool>,
    /// Monotonic epoch — single source of truth for all timer deadline math.
    epoch: Instant,
}

impl TimerState {
    /// Create a new timer state. The epoch is set to `Instant::now()`.
    pub fn new() -> Self {
        Self {
            nesting_depth: Arc::new(AtomicU32::new(0)),
            abort_flag: Arc::new(AtomicBool::new(false)),
            epoch: Instant::now(),
        }
    }

    /// Enter a nested timer callback. Returns the depth *before* incrementing.
    ///
    /// The caller must pair this with [`exit_nesting`] when the callback finishes.
    pub fn enter_nesting(&self) -> u32 {
        self.nesting_depth.fetch_add(1, Ordering::Relaxed)
    }

    /// Exit a nested timer callback (decrements depth, saturating at 0).
    pub fn exit_nesting(&self) {
        // Saturating: guard against mismatched enter/exit pairs.
        let prev = self.nesting_depth.load(Ordering::Relaxed);
        if prev > 0 {
            self.nesting_depth.fetch_sub(1, Ordering::Relaxed);
        }
    }

    /// Current nesting depth.
    pub fn nesting_depth(&self) -> u32 {
        self.nesting_depth.load(Ordering::Relaxed)
    }

    /// Compute effective delay per the HTML spec clamping rule.
    ///
    /// When `depth >= 5` and `requested_ms < 4`, returns 4.
    /// Otherwise returns `requested_ms` unchanged.
    pub fn effective_delay(&self, requested_ms: u32, depth: u32) -> u32 {
        if depth >= 5 && requested_ms < 4 {
            4
        } else {
            requested_ms
        }
    }

    /// Signal all timers to abort. Idempotent.
    pub fn abort(&self) {
        self.abort_flag.store(true, Ordering::Release);
    }

    /// Check whether the abort flag has been set.
    pub fn is_aborted(&self) -> bool {
        self.abort_flag.load(Ordering::Acquire)
    }

    /// Clear the abort flag (e.g., when reusing state across pages).
    pub fn clear_abort(&self) {
        self.abort_flag.store(false, Ordering::Release);
    }

    /// Get a shared reference to the abort flag for external watchers (e.g. watchdog).
    pub fn abort_flag(&self) -> Arc<AtomicBool> {
        self.abort_flag.clone()
    }

    /// Milliseconds elapsed since the epoch (monotonic).
    pub fn elapsed_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }

    /// The monotonic epoch instant.
    pub fn epoch(&self) -> Instant {
        self.epoch
    }

    /// Reset all state for reuse (depth = 0, abort = false, epoch = now).
    pub fn reset(&mut self) {
        self.nesting_depth.store(0, Ordering::Relaxed);
        self.abort_flag.store(false, Ordering::Release);
        self.epoch = Instant::now();
    }
}

impl Default for TimerState {
    fn default() -> Self {
        Self::new()
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

    /// Produce a structured snapshot of the current task state.
    ///
    /// The `long_tasks` and `is_network_idle` fields must be supplied by the
    /// caller because `TaskTracker` does not own the [`LongTaskDetector`] or
    /// network-idle state.
    pub fn snapshot(
        &self,
        long_tasks: Vec<LongTaskRecord>,
        is_network_idle: bool,
    ) -> TaskSnapshot {
        TaskSnapshot {
            promises: self.promises.load(Ordering::Relaxed),
            timers: self.timers.load(Ordering::Relaxed),
            fetches: self.fetches.load(Ordering::Relaxed),
            long_tasks,
            is_settled: self.is_settled(),
            is_network_idle,
        }
    }
}

// ─── Task Snapshot ───

/// Point-in-time view of the scheduler's task queues.
///
/// Microtasks (promises) always drain before macrotasks (timers) — this is
/// guaranteed by V8's event loop semantics. The snapshot exposes counts so
/// callers can observe ordering without poking atomics directly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSnapshot {
    /// Outstanding promise continuations (microtasks).
    pub promises: usize,
    /// Active timers — setTimeout / setInterval (macrotasks).
    pub timers: usize,
    /// In-flight fetch requests.
    pub fetches: usize,
    /// Tasks whose execution exceeded [`LONG_TASK_THRESHOLD_MS`].
    pub long_tasks: Vec<LongTaskRecord>,
    /// True when all categories are zero.
    pub is_settled: bool,
    /// True when no fetches are pending and the network-idle heuristic fires.
    pub is_network_idle: bool,
}

// ─── Long Task Detection ───

/// Threshold in milliseconds above which a task is considered "long".
///
/// Matches the W3C Long Tasks API definition (50 ms).
pub const LONG_TASK_THRESHOLD_MS: u64 = 50;

/// Record emitted when a task exceeds [`LONG_TASK_THRESHOLD_MS`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LongTaskRecord {
    /// Identifier of the task (e.g. script URL or timer id).
    pub task_id: String,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
    /// Execution phase when the long task occurred (e.g. "script", "timer").
    pub phase: String,
}

/// Detects script executions that exceed the long-task threshold.
///
/// Call [`start_task`](LongTaskDetector::start_task) before execution and
/// [`end_task`](LongTaskDetector::end_task) after. If the elapsed time
/// exceeds [`LONG_TASK_THRESHOLD_MS`], a [`LongTaskRecord`] is returned.
#[derive(Debug, Default)]
pub struct LongTaskDetector {
    /// Active tasks keyed by task_id -> (start instant, phase).
    active: HashMap<String, (Instant, String)>,
    /// Completed long-task records (kept for snapshot queries).
    records: Vec<LongTaskRecord>,
}

impl LongTaskDetector {
    /// Create an empty detector.
    pub fn new() -> Self {
        Self::default()
    }

    /// Begin timing a task.
    ///
    /// `task_id` should be a stable identifier (script URL, timer handle, etc.).
    /// `phase` describes the execution context (e.g. "script", "timer", "fetch").
    pub fn start_task(&mut self, task_id: &str, phase: &str) {
        self.active
            .insert(task_id.to_owned(), (Instant::now(), phase.to_owned()));
    }

    /// Finish timing a task.
    ///
    /// Returns `Some(LongTaskRecord)` if the task exceeded the threshold,
    /// `None` otherwise. Unknown `task_id`s are silently ignored.
    pub fn end_task(&mut self, task_id: &str) -> Option<LongTaskRecord> {
        let (start, phase) = self.active.remove(task_id)?;
        let elapsed_ms = start.elapsed().as_millis() as u64;
        if elapsed_ms >= LONG_TASK_THRESHOLD_MS {
            let record = LongTaskRecord {
                task_id: task_id.to_owned(),
                duration_ms: elapsed_ms,
                phase,
            };
            self.records.push(record.clone());
            Some(record)
        } else {
            None
        }
    }

    /// All long-task records collected so far.
    pub fn records(&self) -> &[LongTaskRecord] {
        &self.records
    }

    /// Drain all collected records (moves them out).
    pub fn drain_records(&mut self) -> Vec<LongTaskRecord> {
        std::mem::take(&mut self.records)
    }

    /// Reset all state (active tasks and records).
    pub fn reset(&mut self) {
        self.active.clear();
        self.records.clear();
    }
}

// ─── Abort Reasons ───

/// Stable reason codes for every abort, skip, or timeout in the scheduler.
///
/// Each variant is a fixed identifier — callers should match on these rather
/// than parsing human-readable strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AbortReason {
    /// The per-page timer tick budget was exhausted.
    TimerBudgetExhausted,
    /// The per-page fetch budget was exhausted.
    FetchBudgetExhausted,
    /// The global watchdog wall-clock timeout fired.
    WatchdogTimeout,
    /// A single script execution exceeded its timeout.
    ScriptTimeout,
    /// Manual abort requested (e.g. by the caller or an API signal).
    ManualAbort,
    /// Network went idle (no in-flight fetches for the idle window).
    NetworkIdle,
    /// Microtask queue grew unboundedly (possible infinite promise chain).
    MicrotaskStarvation,
}

/// Event emitted when the scheduler aborts or skips work.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbortEvent {
    /// Why the abort happened.
    pub reason: AbortReason,
    /// Execution phase when the abort occurred (e.g. "settle_loop", "timer_fire").
    pub phase: String,
    /// Optional resource that triggered the abort (URL, timer id, etc.).
    pub resource_id: Option<String>,
    /// Milliseconds since the scheduler epoch when the event was created.
    pub timestamp_ms: u64,
}

impl AbortEvent {
    /// Create a new abort event stamped at `timestamp_ms` (monotonic milliseconds).
    pub fn new(
        reason: AbortReason,
        phase: impl Into<String>,
        resource_id: Option<String>,
        timestamp_ms: u64,
    ) -> Self {
        Self {
            reason,
            phase: phase.into(),
            resource_id,
            timestamp_ms,
        }
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

    // ─── R8e: Long Task Detection ───

    #[test]
    fn test_long_task_detected() {
        let mut detector = LongTaskDetector::new();
        detector.start_task("slow_script.js", "script");
        // Sleep >50ms to trigger long-task threshold.
        std::thread::sleep(std::time::Duration::from_millis(60));
        let record = detector.end_task("slow_script.js");
        assert!(record.is_some(), "task >50ms should be flagged");
        let r = record.expect("checked above");
        assert_eq!(r.task_id, "slow_script.js");
        assert_eq!(r.phase, "script");
        assert!(r.duration_ms >= LONG_TASK_THRESHOLD_MS);
        assert_eq!(detector.records().len(), 1);
    }

    #[test]
    fn test_short_task_not_flagged() {
        let mut detector = LongTaskDetector::new();
        detector.start_task("fast.js", "script");
        // No sleep — completes in <1ms.
        let record = detector.end_task("fast.js");
        assert!(record.is_none(), "task <50ms should NOT be flagged");
        assert!(detector.records().is_empty());
    }

    // ─── R8e: Abort Reason Serialization ───

    #[test]
    fn test_abort_reason_serialization() {
        let variants = [
            AbortReason::TimerBudgetExhausted,
            AbortReason::FetchBudgetExhausted,
            AbortReason::WatchdogTimeout,
            AbortReason::ScriptTimeout,
            AbortReason::ManualAbort,
            AbortReason::NetworkIdle,
            AbortReason::MicrotaskStarvation,
        ];
        for v in &variants {
            let json = serde_json::to_string(v).expect("serialize");
            let back: AbortReason = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(*v, back);
        }
    }

    // ─── R8e: Task Snapshot ───

    #[test]
    fn test_task_snapshot() {
        let tracker = TaskTracker::new();
        tracker.add_promise();
        tracker.add_promise();
        tracker.add_timer();
        tracker.add_fetch();
        tracker.add_fetch();
        tracker.add_fetch();

        let long = vec![LongTaskRecord {
            task_id: "heavy.js".into(),
            duration_ms: 120,
            phase: "script".into(),
        }];
        let snap = tracker.snapshot(long, false);

        assert_eq!(snap.promises, 2);
        assert_eq!(snap.timers, 1);
        assert_eq!(snap.fetches, 3);
        assert!(!snap.is_settled);
        assert!(!snap.is_network_idle);
        assert_eq!(snap.long_tasks.len(), 1);
        assert_eq!(snap.long_tasks[0].task_id, "heavy.js");

        // Settled snapshot.
        tracker.reset();
        let snap2 = tracker.snapshot(vec![], true);
        assert!(snap2.is_settled);
        assert!(snap2.is_network_idle);
        assert_eq!(snap2.promises, 0);
    }

    // ─── R8e: Abort Event ───

    #[test]
    fn test_abort_event_creation() {
        let evt = AbortEvent::new(
            AbortReason::WatchdogTimeout,
            "settle_loop",
            Some("https://example.com/slow.js".into()),
            5432,
        );
        assert_eq!(evt.reason, AbortReason::WatchdogTimeout);
        assert_eq!(evt.phase, "settle_loop");
        assert_eq!(
            evt.resource_id.as_deref(),
            Some("https://example.com/slow.js")
        );
        assert_eq!(evt.timestamp_ms, 5432);

        // Without resource_id.
        let evt2 = AbortEvent::new(AbortReason::ManualAbort, "shutdown", None, 0);
        assert!(evt2.resource_id.is_none());

        // Serialization round-trip.
        let json = serde_json::to_string(&evt).expect("serialize");
        let back: AbortEvent = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.reason, AbortReason::WatchdogTimeout);
        assert_eq!(back.phase, "settle_loop");
    }
}
