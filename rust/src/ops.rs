//! Higher-level page operations built on `page::js` — the JS-blob-backed tools
//! (page_info, analyze, fill, form_fill, submit, find_and_click, dismiss_overlay,
//! extract, extract_table, scroll, wait, paginate). JS blobs are ported verbatim
//! from the Python server; arguments are interpolated with `serde_json::to_string`
//! for the same safe escaping the Python got from `json.dumps`.
//!
//! The verbs are grouped by what they are for: [`introspect`] asks what the page is,
//! [`forms`] puts data in, [`target`] gets to an element and clears what covers it, and
//! [`harvest`] gets data out. The shared limits and quoting helpers stay here, since every
//! group needs them and they are plumbing rather than a domain of their own.

use std::time::Duration;

use serde_json::Value;

/// Cap on client-controlled waits: the server is sequential, so an unbounded
/// `wait`/`submit` would wedge every other tool call.
pub const MAX_WAIT: Duration = Duration::from_secs(60);

pub mod forms;
pub mod harvest;
pub mod introspect;
pub mod target;

pub use forms::{fill, form_fill, submit};
pub use harvest::{extract, extract_table, paginate};
pub use introspect::{analyze, debug, eval_js, page_info};
pub use target::{dismiss_overlay, find_and_click, scroll, wait};

/// Clamp a client-supplied seconds value into `[0, MAX_WAIT]`. Non-finite
/// (NaN/inf) becomes 0 — `Duration::from_secs_f64` panics on those, so nothing
/// unvalidated may ever reach it.
fn bounded_secs_f64(v: f64) -> f64 {
    if v.is_finite() {
        v.clamp(0.0, MAX_WAIT.as_secs_f64())
    } else {
        0.0
    }
}

/// Clamp a client-supplied milliseconds value into `[0, MAX_WAIT]`.
fn bounded_ms_i64(v: i64) -> u64 {
    v.clamp(0, MAX_WAIT.as_millis() as i64) as u64
}

/// Safely encode a Rust string as a JS literal (quotes + escaping).
fn js_lit(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// Return a JSON value's string, or a fallback if it wasn't a non-empty string.
fn str_or(v: Value, fallback: &str) -> String {
    match v {
        Value::String(s) if !s.is_empty() => s,
        Value::Null => fallback.to_string(),
        Value::String(_) => fallback.to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::target::dismiss_overlay_js;
    use super::*;

    #[test]
    fn js_lit_escapes_quotes_and_specials() {
        assert_eq!(js_lit("a\"b"), "\"a\\\"b\"");
        assert_eq!(js_lit("x\ny"), "\"x\\ny\"");
    }

    #[test]
    fn str_or_falls_back_on_null_and_empty() {
        assert_eq!(str_or(Value::Null, "{}"), "{}");
        assert_eq!(str_or(Value::String(String::new()), "[]"), "[]");
        assert_eq!(str_or(Value::String("ok".into()), "x"), "ok");
        assert_eq!(str_or(json!({"a":1}), "x"), "{\"a\":1}");
    }

    #[test]
    fn dismiss_overlay_force_is_substituted() {
        assert!(dismiss_overlay_js().contains("(FORCE)"));
        let t = dismiss_overlay_js().replace("FORCE", "true");
        assert!(t.contains("(true)"));
        assert!(!t.contains("FORCE"));
    }

    #[test]
    fn bounded_secs_f64_never_panics_input() {
        // NaN/inf/negative would panic Duration::from_secs_f64 if passed raw.
        assert_eq!(bounded_secs_f64(f64::NAN), 0.0);
        assert_eq!(bounded_secs_f64(f64::INFINITY), 0.0);
        assert_eq!(bounded_secs_f64(f64::NEG_INFINITY), 0.0);
        assert_eq!(bounded_secs_f64(-1.0), 0.0);
        assert_eq!(bounded_secs_f64(5.0), 5.0);
        assert_eq!(bounded_secs_f64(86_400.0), MAX_WAIT.as_secs_f64());
        // Every result is a legal from_secs_f64 argument.
        for v in [-1.0, f64::NAN, f64::INFINITY, 1e300, 0.5] {
            let _ = Duration::from_secs_f64(bounded_secs_f64(v));
        }
    }

    #[test]
    fn bounded_ms_i64_clamps_to_range() {
        assert_eq!(bounded_ms_i64(-5), 0);
        assert_eq!(bounded_ms_i64(250), 250);
        assert_eq!(bounded_ms_i64(i64::MAX), MAX_WAIT.as_millis() as u64);
    }
}
