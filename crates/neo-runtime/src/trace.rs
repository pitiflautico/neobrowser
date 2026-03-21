//! Trace flag — enables detailed diagnostic output when `NEORENDER_TRACE=1`.
//!
//! All trace macros are no-ops unless the env var is set at startup.

use std::sync::atomic::{AtomicBool, Ordering};

/// Global trace flag, checked once at startup.
static TRACE_ENABLED: AtomicBool = AtomicBool::new(false);
/// Whether we've initialized the flag.
static TRACE_INIT: AtomicBool = AtomicBool::new(false);

/// Check (and lazily initialize) the trace flag.
pub fn is_trace_enabled() -> bool {
    if !TRACE_INIT.load(Ordering::Relaxed) {
        let val = std::env::var("NEORENDER_TRACE")
            .map(|v| v == "1")
            .unwrap_or(false);
        TRACE_ENABLED.store(val, Ordering::Relaxed);
        TRACE_INIT.store(true, Ordering::Release);
    }
    TRACE_ENABLED.load(Ordering::Relaxed)
}

/// Emit a trace line to stderr if `NEORENDER_TRACE=1`.
#[macro_export]
macro_rules! neo_trace {
    ($($arg:tt)*) => {
        if $crate::trace::is_trace_enabled() {
            eprintln!($($arg)*);
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trace_flag_default_off() {
        // Without NEORENDER_TRACE=1, should be off (unless set in env).
        // We can't control env in unit tests reliably, so just check it doesn't panic.
        let _ = is_trace_enabled();
    }
}
