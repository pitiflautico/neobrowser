//! EventLoopRunner — decomposed event loop management with panic recovery.
//!
//! Extracts the settle/quiescence/pump logic from `v8_runtime_impl.rs` into
//! small, independently testable pieces. The critical improvement is wrapping
//! `runtime.run_event_loop()` in `catch_unwind` to prevent `web_timeout.rs:189`
//! panics from crashing the entire process.

use std::future::Future;
use std::task::Poll;
use std::time::{Duration, Instant};

use deno_core::PollEventLoopOptions;

use crate::neo_trace;
use crate::v8_runtime_impl::first_line;

// ─── PumpResult ───

/// Outcome of a single event loop pump.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PumpResult {
    /// Event loop went idle (no pending work).
    Idle,
    /// Event loop timed out (work was in progress).
    Timeout,
    /// JS error during event loop execution.
    Error(String),
    /// V8 panicked (e.g. web_timeout.rs:189). Isolate may be corrupted.
    Panic,
}

// ─── SettleReason ───

/// Why the settle loop exited.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettleReason {
    /// N consecutive quiet rounds reached.
    Quiet,
    /// max_timeout_ms exceeded.
    Timeout,
    /// No async activity ever seen — fast path exit.
    NoAsyncWork,
    /// pushState/replaceState detected — caller should re-enter.
    RouteChange,
    /// Event loop error (non-fatal).
    Error(String),
    /// V8 panicked — isolate may be corrupted.
    Panic,
}

// ─── SettleConfig ───

/// Configuration for how to determine when a page is "settled".
#[derive(Debug, Clone)]
pub struct SettleConfig {
    /// Minimum consecutive quiet rounds before declaring settled.
    pub quiet_rounds_required: u32,
    /// Interval between quiescence checks (ms).
    pub check_interval_ms: u64,
    /// Maximum total time before timeout (ms).
    pub max_timeout_ms: u64,
    /// Minimum time before settling when async activity was seen (ms).
    pub min_settle_ms: u64,
    /// JS idle_ms threshold for a round to count as "quiet".
    pub quiet_window_ms: u64,
}

impl Default for SettleConfig {
    fn default() -> Self {
        Self {
            quiet_rounds_required: 3,
            check_interval_ms: 50,
            max_timeout_ms: 15000,
            min_settle_ms: 1500,
            quiet_window_ms: 400,
        }
    }
}

impl SettleConfig {
    /// Config for bootstrap settle (original defaults).
    pub fn bootstrap(timeout_ms: u64) -> Self {
        Self {
            quiet_rounds_required: 3,
            check_interval_ms: 50,
            max_timeout_ms: timeout_ms,
            min_settle_ms: 1500_u64.min(timeout_ms * 2 / 3),
            quiet_window_ms: 400,
        }
    }

    /// Config for post-interaction settle (much more aggressive).
    pub fn interaction(timeout_ms: u64) -> Self {
        Self {
            quiet_rounds_required: 1, // epoch tracking handles the "extra cycle" requirement
            check_interval_ms: 50,
            max_timeout_ms: timeout_ms,
            min_settle_ms: 75_u64.min(timeout_ms * 2 / 3),
            quiet_window_ms: 400,
        }
    }
}

// ─── RunStats ───

/// Statistics from a settle loop run.
#[derive(Debug, Clone)]
pub struct RunStats {
    /// Wall-clock milliseconds elapsed.
    pub elapsed_ms: u64,
    /// How many consecutive quiet rounds were achieved.
    pub quiet_rounds: u32,
    /// Whether any async activity (fetches, timers, mutations) was seen.
    pub saw_async_activity: bool,
    /// Whether a route change (pushState) was detected.
    pub route_changed: bool,
    /// Why the settle loop exited.
    pub reason: SettleReason,
}

// ─── QuiescenceState ───

/// Quiescence signals read from the JS `__neo_quiescence()` function.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct QuiescenceState {
    /// Milliseconds since last JS activity.
    #[serde(default)]
    pub idle_ms: u64,
    /// Active setTimeout/setInterval handles.
    #[serde(default)]
    pub pending_timers: usize,
    /// In-flight fetch() calls.
    #[serde(default)]
    pub pending_fetches: usize,
    /// Dynamic import() in flight.
    #[serde(default)]
    pub pending_modules: usize,
    /// Recent DOM mutations since last reset.
    #[serde(default)]
    pub dom_mutations: usize,
}

impl QuiescenceState {
    /// Whether all async categories are at zero.
    pub fn is_fully_idle(&self) -> bool {
        self.pending_timers == 0
            && self.pending_fetches == 0
            && self.pending_modules == 0
            && self.dom_mutations == 0
    }

    /// Query the JS runtime for current quiescence state.
    ///
    /// Returns `None` if the JS function is not available or eval fails.
    pub fn check(runtime: &mut deno_core::JsRuntime) -> Option<Self> {
        let code = "typeof __neo_quiescence==='function'?__neo_quiescence():'{}'";
        let val = runtime.execute_script("<quiescence>", code.to_string()).ok()?;
        let json = {
            crate::v8_runtime_impl::neo_handle_scope!(scope, runtime);
            let local = deno_core::v8::Local::new(scope, val);
            local
                .to_string(scope)
                .map(|s| s.to_rust_string_lossy(scope))?
        };
        serde_json::from_str(&json).ok()
    }

    /// Reset the JS-side mutation counter after reading.
    pub fn reset_mutations(runtime: &mut deno_core::JsRuntime) {
        let _ = runtime.execute_script(
            "<reset-mutations>",
            "typeof __neo_resetMutationCount==='function'&&__neo_resetMutationCount()".to_string(),
        );
    }

    /// Check if a route change (pushState/replaceState) happened.
    pub fn check_route_changed(runtime: &mut deno_core::JsRuntime) -> bool {
        let code = "typeof __neo_routeChanged!=='undefined'&&__neo_routeChanged";
        match runtime.execute_script("<route-check>", code.to_string()) {
            Ok(val) => {
                crate::v8_runtime_impl::neo_handle_scope!(scope, runtime);
                let local = deno_core::v8::Local::new(scope, val);
                local.is_true()
            }
            Err(_) => false,
        }
    }

    /// Clear the route change flag in JS.
    pub fn clear_route_changed(runtime: &mut deno_core::JsRuntime) {
        let _ = runtime.execute_script(
            "<route-reset>",
            "__neo_routeChanged=false".to_string(),
        );
    }
}

// ─── EventLoopRunner ───

/// Manages V8 event loop execution with timeout, quiescence detection,
/// and panic recovery.
pub struct EventLoopRunner {
    /// Statistics from the last run.
    pub last_run: Option<RunStats>,
}

impl EventLoopRunner {
    pub fn new() -> Self {
        Self { last_run: None }
    }

    /// Pump the event loop using manual polling — avoids nested block_on.
    ///
    /// Instead of `tokio_rt.block_on(runtime.run_event_loop())` which causes
    /// a data race when deno_core's internal WebTimers try to use the same
    /// tokio runtime, we:
    /// 1. Enter the tokio context (so timers can register)
    /// 2. Manually poll the event loop future
    /// 3. Use std::thread::sleep between polls (not tokio::time::sleep)
    ///
    /// This eliminates the nested block_on that causes web_timeout.rs:189 panics.
    pub fn pump_once(
        runtime: &mut deno_core::JsRuntime,
        tokio_rt: &tokio::runtime::Runtime,
        timeout_ms: u64,
    ) -> PumpResult {
        Self::pump_once_with_options(
            runtime,
            tokio_rt,
            timeout_ms,
            PollEventLoopOptions::default(),
        )
    }

    /// Pump with explicit PollEventLoopOptions using manual polling.
    ///
    /// Uses `Future::poll` with tokio runtime context entered (not block_on)
    /// to avoid nested block_on conflicts with deno_core's WebTimer system.
    pub fn pump_once_with_options(
        runtime: &mut deno_core::JsRuntime,
        tokio_rt: &tokio::runtime::Runtime,
        timeout_ms: u64,
        options: PollEventLoopOptions,
    ) -> PumpResult {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);

        // Use block_on with timeout. The web_timeout.rs:189 data race is fixed
        // in our patched deno_core (catch_unwind in MutableSleep::change).
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            tokio_rt.block_on(async {
                tokio::time::timeout(
                    Duration::from_millis(timeout_ms),
                    runtime.run_event_loop(options),
                )
                .await
            })
        })) {
            Ok(Ok(Ok(()))) => PumpResult::Idle,
            Ok(Ok(Err(e))) => {
                let full = format!("{e:?}");
                // Use Debug format for full error with JS stack trace
                if full.len() > 200 {
                    PumpResult::Error(full[..500.min(full.len())].to_string())
                } else {
                    PumpResult::Error(full)
                }
            }
            Ok(Err(_)) => PumpResult::Timeout,
            Err(panic_info) => {
                let msg = if let Some(s) = panic_info.downcast_ref::<String>() {
                    s.clone()
                } else if let Some(s) = panic_info.downcast_ref::<&str>() {
                    s.to_string()
                } else {
                    "unknown panic".to_string()
                };
                eprintln!("[neo-runtime] EVENT LOOP PANIC (caught): {msg}");
                PumpResult::Panic
            }
        }
    }

    /// Run the event loop until the page is settled (bootstrap mode).
    ///
    /// Uses consecutive quiet rounds + min_settle_ms + quiescence checks.
    /// This is the decomposed version of `run_until_settled` from v8_runtime_impl.
    pub fn run_until_settled(
        &mut self,
        runtime: &mut deno_core::JsRuntime,
        tokio_rt: &tokio::runtime::Runtime,
        config: &SettleConfig,
        rust_pending_modules_fn: impl Fn() -> usize,
    ) -> RunStats {
        let start = Instant::now();
        let deadline = start + Duration::from_millis(config.max_timeout_ms);
        let mut quiet_rounds: u32 = 0;
        let mut saw_async = false;

        loop {
            // Hard deadline check
            if Instant::now() >= deadline {
                let stats = RunStats {
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    quiet_rounds,
                    saw_async_activity: saw_async,
                    route_changed: false,
                    reason: SettleReason::Timeout,
                };
                self.last_run = Some(stats.clone());
                return stats;
            }

            let remaining_ms = deadline
                .saturating_duration_since(Instant::now())
                .as_millis() as u64;
            let loop_timeout = config.check_interval_ms.min(remaining_ms);
            if loop_timeout == 0 {
                let stats = RunStats {
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    quiet_rounds,
                    saw_async_activity: saw_async,
                    route_changed: false,
                    reason: SettleReason::Timeout,
                };
                self.last_run = Some(stats.clone());
                return stats;
            }

            // Pump event loop with panic recovery
            let pump = Self::pump_once(runtime, tokio_rt, loop_timeout);

            match &pump {
                PumpResult::Panic => {
                    let stats = RunStats {
                        elapsed_ms: start.elapsed().as_millis() as u64,
                        quiet_rounds,
                        saw_async_activity: saw_async,
                        route_changed: false,
                        reason: SettleReason::Panic,
                    };
                    self.last_run = Some(stats.clone());
                    return stats;
                }
                PumpResult::Error(msg) => {
                    eprintln!(
                        "[neo-runtime] event loop error (non-fatal): {}",
                        msg
                    );
                    let stats = RunStats {
                        elapsed_ms: start.elapsed().as_millis() as u64,
                        quiet_rounds,
                        saw_async_activity: saw_async,
                        route_changed: false,
                        reason: SettleReason::Error(msg.clone()),
                    };
                    self.last_run = Some(stats.clone());
                    return stats;
                }
                PumpResult::Timeout => {
                    // Event loop had work — reset quiet counter
                    quiet_rounds = 0;
                    continue;
                }
                PumpResult::Idle => {
                    // Fall through to quiescence check
                }
            }

            // Query quiescence state from JS
            let q = QuiescenceState::check(runtime).unwrap_or_default();
            let elapsed = start.elapsed().as_millis() as u64;

            // Reset mutation counter after reading
            QuiescenceState::reset_mutations(runtime);

            // Check Rust-side module tracker
            let rust_pending = rust_pending_modules_fn();

            // Route change detection
            if QuiescenceState::check_route_changed(runtime) {
                QuiescenceState::clear_route_changed(runtime);
                quiet_rounds = 0;
                neo_trace!("[SETTLE] route changed (pushState/replaceState) — resetting quiet counter");
                continue;
            }

            // Track async activity
            let no_pending = q.pending_timers == 0
                && q.pending_fetches == 0
                && q.pending_modules == 0
                && rust_pending == 0;
            let no_mutations = q.dom_mutations == 0;

            if !no_pending || !no_mutations {
                saw_async = true;
            }

            // Settle criteria
            let min_elapsed = elapsed >= config.min_settle_ms;
            let quiet = q.idle_ms >= config.quiet_window_ms;
            let time_gate = if saw_async { min_elapsed } else { true };
            let round_quiet = time_gate && quiet && no_pending && no_mutations;

            if round_quiet {
                quiet_rounds += 1;
            } else {
                quiet_rounds = 0;
            }

            neo_trace!(
                "[SETTLE] elapsed={}ms idle={}ms timers={} fetches={} js_modules={} rust_modules={} mutations={} quiet_rounds={}/{} -> {}",
                elapsed, q.idle_ms, q.pending_timers, q.pending_fetches,
                q.pending_modules, rust_pending, q.dom_mutations,
                quiet_rounds, config.quiet_rounds_required,
                if quiet_rounds >= config.quiet_rounds_required { "SETTLED" } else { "waiting" }
            );

            if quiet_rounds >= config.quiet_rounds_required {
                let stats = RunStats {
                    elapsed_ms: elapsed,
                    quiet_rounds,
                    saw_async_activity: saw_async,
                    route_changed: false,
                    reason: SettleReason::Quiet,
                };
                self.last_run = Some(stats.clone());
                return stats;
            }

            // Brief sleep before next check — std::thread::sleep to avoid
            // nested block_on on the same tokio runtime.
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Run the event loop until stable after an interaction.
    ///
    /// Uses epoch tracking: after DOM mutation or fetch resolve, requires at
    /// least one additional quiet cycle before declaring settled.
    pub fn run_until_interaction_stable(
        &mut self,
        runtime: &mut deno_core::JsRuntime,
        tokio_rt: &tokio::runtime::Runtime,
        config: &SettleConfig,
        rust_pending_modules_fn: impl Fn() -> usize,
    ) -> RunStats {
        let start = Instant::now();
        let deadline = start + Duration::from_millis(config.max_timeout_ms);
        let mut epoch_dirty = false;
        let mut saw_quiet_after_epoch = false;

        loop {
            // Hard deadline
            if Instant::now() >= deadline {
                let elapsed = start.elapsed().as_millis() as u64;
                let reason_str = if epoch_dirty {
                    "timeout_after_mutation"
                } else {
                    "timeout_with_pending_fetch"
                };
                eprintln!(
                    "[neo-runtime] interaction settle: {} after {}ms",
                    reason_str, elapsed
                );
                let stats = RunStats {
                    elapsed_ms: elapsed,
                    quiet_rounds: 0,
                    saw_async_activity: epoch_dirty,
                    route_changed: false,
                    reason: SettleReason::Timeout,
                };
                self.last_run = Some(stats.clone());
                return stats;
            }

            let remaining_ms = deadline
                .saturating_duration_since(Instant::now())
                .as_millis() as u64;
            let loop_timeout = config.check_interval_ms.min(remaining_ms);
            if loop_timeout == 0 {
                let stats = RunStats {
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    quiet_rounds: 0,
                    saw_async_activity: epoch_dirty,
                    route_changed: false,
                    reason: SettleReason::Timeout,
                };
                self.last_run = Some(stats.clone());
                return stats;
            }

            // Pump with panic recovery
            let pump = Self::pump_once(runtime, tokio_rt, loop_timeout);

            match &pump {
                PumpResult::Panic => {
                    let stats = RunStats {
                        elapsed_ms: start.elapsed().as_millis() as u64,
                        quiet_rounds: 0,
                        saw_async_activity: epoch_dirty,
                        route_changed: false,
                        reason: SettleReason::Panic,
                    };
                    self.last_run = Some(stats.clone());
                    return stats;
                }
                PumpResult::Error(msg) => {
                    eprintln!(
                        "[neo-runtime] interaction event loop error (non-fatal): {}",
                        msg
                    );
                    let stats = RunStats {
                        elapsed_ms: start.elapsed().as_millis() as u64,
                        quiet_rounds: 0,
                        saw_async_activity: epoch_dirty,
                        route_changed: false,
                        reason: SettleReason::Error(msg.clone()),
                    };
                    self.last_run = Some(stats.clone());
                    return stats;
                }
                PumpResult::Timeout => {
                    // Event loop had work — mark epoch dirty
                    epoch_dirty = true;
                    saw_quiet_after_epoch = false;
                    continue;
                }
                PumpResult::Idle => {
                    // Fall through to quiescence check
                }
            }

            // Query quiescence
            let q = QuiescenceState::check(runtime).unwrap_or_default();
            let elapsed = start.elapsed().as_millis() as u64;

            // Reset mutation counter
            QuiescenceState::reset_mutations(runtime);

            let rust_pending = rust_pending_modules_fn();
            let has_activity = q.dom_mutations > 0 || q.pending_fetches > 0;
            let no_pending = q.pending_timers == 0
                && q.pending_fetches == 0
                && q.pending_modules == 0
                && rust_pending == 0;
            let no_mutations = q.dom_mutations == 0;
            let quiet = q.idle_ms >= config.quiet_window_ms;
            let min_elapsed = elapsed >= config.min_settle_ms;

            // Epoch tracking
            if has_activity {
                epoch_dirty = true;
                saw_quiet_after_epoch = false;
            } else if epoch_dirty && no_pending && no_mutations && quiet {
                saw_quiet_after_epoch = true;
            }

            let epoch_ok = !epoch_dirty || saw_quiet_after_epoch;

            let settled = min_elapsed && quiet && no_pending && no_mutations && epoch_ok;

            neo_trace!(
                "[INTERACTION-SETTLE] elapsed={}ms idle={}ms timers={} fetches={} mutations={} epoch_dirty={} saw_quiet={} -> {}",
                elapsed, q.idle_ms, q.pending_timers, q.pending_fetches,
                q.dom_mutations, epoch_dirty, saw_quiet_after_epoch,
                if settled { "SETTLED" } else { "waiting" }
            );

            if settled {
                eprintln!(
                    "[neo-runtime] interaction settle: quiet_no_pending after {}ms",
                    elapsed
                );
                let stats = RunStats {
                    elapsed_ms: elapsed,
                    quiet_rounds: 1,
                    saw_async_activity: epoch_dirty,
                    route_changed: false,
                    reason: SettleReason::Quiet,
                };
                self.last_run = Some(stats.clone());
                return stats;
            }

            // Brief sleep before next check — std::thread::sleep to avoid
            // nested block_on on the same tokio runtime.
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

impl Default for EventLoopRunner {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_settle_config_defaults() {
        let c = SettleConfig::default();
        assert_eq!(c.quiet_rounds_required, 3);
        assert_eq!(c.check_interval_ms, 50);
        assert_eq!(c.max_timeout_ms, 15000);
        assert_eq!(c.min_settle_ms, 1500);
        assert_eq!(c.quiet_window_ms, 400);
    }

    #[test]
    fn test_settle_config_bootstrap() {
        let c = SettleConfig::bootstrap(5000);
        assert_eq!(c.quiet_rounds_required, 3);
        assert_eq!(c.max_timeout_ms, 5000);
        // min_settle = min(1500, 5000*2/3=3333) = 1500
        assert_eq!(c.min_settle_ms, 1500);
    }

    #[test]
    fn test_settle_config_bootstrap_short_timeout() {
        let c = SettleConfig::bootstrap(500);
        // min_settle = min(1500, 500*2/3=333) = 333
        assert_eq!(c.min_settle_ms, 333);
    }

    #[test]
    fn test_settle_config_interaction() {
        let c = SettleConfig::interaction(3000);
        assert_eq!(c.quiet_rounds_required, 1);
        assert_eq!(c.max_timeout_ms, 3000);
        assert_eq!(c.min_settle_ms, 75);
    }

    #[test]
    fn test_settle_config_interaction_short_timeout() {
        let c = SettleConfig::interaction(50);
        // min_settle = min(75, 50*2/3=33) = 33
        assert_eq!(c.min_settle_ms, 33);
    }

    #[test]
    fn test_quiescence_fully_idle() {
        let q = QuiescenceState {
            idle_ms: 500,
            pending_timers: 0,
            pending_fetches: 0,
            pending_modules: 0,
            dom_mutations: 0,
        };
        assert!(q.is_fully_idle());
    }

    #[test]
    fn test_quiescence_with_pending_fetch() {
        let q = QuiescenceState {
            idle_ms: 500,
            pending_timers: 0,
            pending_fetches: 1,
            pending_modules: 0,
            dom_mutations: 0,
        };
        assert!(!q.is_fully_idle());
    }

    #[test]
    fn test_quiescence_with_pending_timer() {
        let q = QuiescenceState {
            idle_ms: 500,
            pending_timers: 2,
            pending_fetches: 0,
            pending_modules: 0,
            dom_mutations: 0,
        };
        assert!(!q.is_fully_idle());
    }

    #[test]
    fn test_quiescence_with_pending_module() {
        let q = QuiescenceState {
            idle_ms: 100,
            pending_timers: 0,
            pending_fetches: 0,
            pending_modules: 1,
            dom_mutations: 0,
        };
        assert!(!q.is_fully_idle());
    }

    #[test]
    fn test_quiescence_with_dom_mutations() {
        let q = QuiescenceState {
            idle_ms: 100,
            pending_timers: 0,
            pending_fetches: 0,
            pending_modules: 0,
            dom_mutations: 5,
        };
        assert!(!q.is_fully_idle());
    }

    #[test]
    fn test_quiescence_default_is_idle() {
        let q = QuiescenceState::default();
        assert!(q.is_fully_idle());
        assert_eq!(q.idle_ms, 0);
    }

    #[test]
    fn test_quiescence_deserialize() {
        let json = r#"{"idle_ms":250,"pending_timers":1,"pending_fetches":0,"pending_modules":0,"dom_mutations":3}"#;
        let q: QuiescenceState = serde_json::from_str(json).unwrap();
        assert_eq!(q.idle_ms, 250);
        assert_eq!(q.pending_timers, 1);
        assert_eq!(q.dom_mutations, 3);
        assert!(!q.is_fully_idle());
    }

    #[test]
    fn test_quiescence_deserialize_partial() {
        // Missing fields should default to 0
        let json = r#"{"idle_ms":100}"#;
        let q: QuiescenceState = serde_json::from_str(json).unwrap();
        assert_eq!(q.idle_ms, 100);
        assert_eq!(q.pending_timers, 0);
        assert_eq!(q.pending_fetches, 0);
        assert!(q.is_fully_idle());
    }

    #[test]
    fn test_quiescence_deserialize_empty() {
        let json = "{}";
        let q: QuiescenceState = serde_json::from_str(json).unwrap();
        assert!(q.is_fully_idle());
    }

    #[test]
    fn test_pump_result_variants() {
        assert_eq!(PumpResult::Idle, PumpResult::Idle);
        assert_eq!(PumpResult::Timeout, PumpResult::Timeout);
        assert_eq!(PumpResult::Panic, PumpResult::Panic);
        assert_eq!(
            PumpResult::Error("test".into()),
            PumpResult::Error("test".into())
        );
        assert_ne!(PumpResult::Idle, PumpResult::Timeout);
    }

    #[test]
    fn test_settle_reason_variants() {
        // Ensure all variants exist and can be matched
        let reasons = vec![
            SettleReason::Quiet,
            SettleReason::Timeout,
            SettleReason::NoAsyncWork,
            SettleReason::RouteChange,
            SettleReason::Error("test".into()),
            SettleReason::Panic,
        ];
        for r in &reasons {
            match r {
                SettleReason::Quiet => {}
                SettleReason::Timeout => {}
                SettleReason::NoAsyncWork => {}
                SettleReason::RouteChange => {}
                SettleReason::Error(_) => {}
                SettleReason::Panic => {}
            }
        }
        assert_eq!(reasons.len(), 6);
    }

    #[test]
    fn test_run_stats_fields() {
        let s = RunStats {
            elapsed_ms: 1234,
            quiet_rounds: 3,
            saw_async_activity: true,
            route_changed: false,
            reason: SettleReason::Quiet,
        };
        assert_eq!(s.elapsed_ms, 1234);
        assert_eq!(s.quiet_rounds, 3);
        assert!(s.saw_async_activity);
        assert!(!s.route_changed);
        assert_eq!(s.reason, SettleReason::Quiet);
    }

    #[test]
    fn test_run_stats_timeout_reason() {
        let s = RunStats {
            elapsed_ms: 15000,
            quiet_rounds: 0,
            saw_async_activity: true,
            route_changed: false,
            reason: SettleReason::Timeout,
        };
        assert_eq!(s.reason, SettleReason::Timeout);
    }

    #[test]
    fn test_run_stats_panic_reason() {
        let s = RunStats {
            elapsed_ms: 500,
            quiet_rounds: 1,
            saw_async_activity: false,
            route_changed: false,
            reason: SettleReason::Panic,
        };
        assert_eq!(s.reason, SettleReason::Panic);
    }

    #[test]
    fn test_event_loop_runner_new() {
        let runner = EventLoopRunner::new();
        assert!(runner.last_run.is_none());
    }

    #[test]
    fn test_event_loop_runner_default() {
        let runner = EventLoopRunner::default();
        assert!(runner.last_run.is_none());
    }
}
