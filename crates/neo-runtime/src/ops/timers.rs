//! Timer ops — setTimeout/setInterval bridge.

use crate::ops::OpsSchedulerConfig;
use crate::scheduler::{TaskTracker, TimerBudget, TimerState};
use deno_core::op2;
use deno_core::OpState;
use std::cell::RefCell;
use std::rc::Rc;

/// Timer — sync with nested clamping per the HTML spec and abort support.
///
/// Applies nested clamping (depth >= 5 → min 4 ms), then caps at 10 ms.
/// Checks the abort flag before sleeping; returns `false` if aborted.
#[op2(fast)]
pub fn op_timer(state: Rc<RefCell<OpState>>, #[smi] ms: u32) -> bool {
    let s = state.borrow();

    // Check abort flag first — bail if watchdog cancelled timers.
    if let Some(ts) = s.try_borrow::<TimerState>() {
        if ts.is_aborted() {
            return false;
        }
        let depth = ts.nesting_depth();
        let effective = ts.effective_delay(ms, depth);
        let delay = if effective == 0 { 0 } else { effective.clamp(1, 10) };
        // Release borrow before sleeping.
        drop(s);
        if delay > 0 {
            std::thread::sleep(std::time::Duration::from_millis(delay as u64));
        }
    } else {
        // Fallback when no TimerState is installed (backward compat).
        let delay = if ms == 0 { 0 } else { ms.clamp(1, 10) };
        drop(s);
        if delay > 0 {
            std::thread::sleep(std::time::Duration::from_millis(delay as u64));
        }
    }
    true
}

/// Register a new timer in the task tracker.
///
/// Called by JS setTimeout/setInterval to signal pending async work.
/// Returns false if the timer budget is exhausted.
#[op2(fast)]
pub fn op_timer_register(state: Rc<RefCell<OpState>>) -> bool {
    let s = state.borrow();
    // Check budget first
    if let Some(budget) = s.try_borrow::<TimerBudget>() {
        if budget.is_exhausted() {
            return false;
        }
    }
    if let Some(tracker) = s.try_borrow::<TaskTracker>() {
        tracker.add_timer();
    }
    true
}

/// Signal that a timer callback has fired.
///
/// Decrements the timer count and consumes one tick from the budget.
/// Returns false if the budget is now exhausted (interval should stop).
#[op2(fast)]
pub fn op_timer_fire(state: Rc<RefCell<OpState>>) -> bool {
    let s = state.borrow();
    if let Some(tracker) = s.try_borrow::<TaskTracker>() {
        tracker.resolve_timer();
    }
    // Consume budget tick
    if let Some(budget) = s.try_borrow::<TimerBudget>() {
        return budget.tick();
    }
    true
}

/// Get the configured interval max ticks.
#[op2(fast)]
pub fn op_scheduler_config(state: Rc<RefCell<OpState>>) -> u32 {
    let s = state.borrow();
    if let Some(cfg) = s.try_borrow::<OpsSchedulerConfig>() {
        cfg.interval_max_ticks
    } else {
        20
    }
}
