//! Console capture op.

use crate::ops::ConsoleBuffer;
use crate::trace_events::TraceBuffer;
use deno_core::op2;
use deno_core::OpState;
use std::cell::RefCell;
use std::rc::Rc;

/// Capture console.log output from JavaScript.
///
/// Pushes messages to both ConsoleBuffer (legacy) and TraceBuffer (structured).
/// Classification logic determines the TraceEvent type based on message patterns.
#[op2(fast)]
pub fn op_console_log(state: Rc<RefCell<OpState>>, #[string] msg: String) {
    // Print errors/warnings to stderr for debugging
    if msg.starts_with("[error]") || msg.starts_with("[warn]") || msg.starts_with("[script-error]") {
        eprintln!("[js] {}", &msg[..msg.len().min(300)]);
    }
    let s = state.borrow();

    // Push to structured TraceBuffer
    if let Some(trace_buf) = s.try_borrow::<TraceBuffer>() {
        let event = classify_console_message(&msg, trace_buf.elapsed_ms());
        trace_buf.push(event);
    }

    // Push to legacy ConsoleBuffer
    if let Some(buf) = s.try_borrow::<ConsoleBuffer>() {
        let mut messages = buf.messages.lock().expect("console buffer lock poisoned");
        messages.push(msg);
    }
}

/// Classify a console message into a structured TraceEvent.
///
/// Pattern matching:
/// - `[error]`, `[MODULE-ERROR]`, `[script-error]` → JsError
/// - `[uncaught]`, `[unhandled-rejection]` → JsError
/// - `[warn]` → Console { level: "warn" }
/// - everything else → Console { level: "log" }
fn classify_console_message(msg: &str, timestamp_ms: u64) -> crate::trace_events::TraceEvent {
    use crate::trace_events::TraceEvent;

    let lower = msg.to_lowercase();

    if lower.contains("[error]")
        || lower.contains("[module-error]")
        || lower.contains("[script-error]")
    {
        TraceEvent::JsError {
            message: msg.to_string(),
            stack: None,
            source: "console".to_string(),
            timestamp_ms,
        }
    } else if lower.contains("[uncaught]") || lower.contains("[unhandled-rejection]") {
        TraceEvent::JsError {
            message: msg.to_string(),
            stack: None,
            source: "console".to_string(),
            timestamp_ms,
        }
    } else if lower.contains("[warn]") {
        TraceEvent::Console {
            level: "warn".to_string(),
            message: msg.to_string(),
            timestamp_ms,
        }
    } else {
        TraceEvent::Console {
            level: "log".to_string(),
            message: msg.to_string(),
            timestamp_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace_events::TraceEvent;

    #[test]
    fn classify_plain_log() {
        let event = classify_console_message("Hello world", 0);
        match event {
            TraceEvent::Console { level, message, .. } => {
                assert_eq!(level, "log");
                assert_eq!(message, "Hello world");
            }
            other => panic!("expected Console, got {:?}", other),
        }
    }

    #[test]
    fn classify_error_prefix() {
        let event = classify_console_message("[error] something broke", 10);
        match event {
            TraceEvent::JsError { message, source, .. } => {
                assert_eq!(message, "[error] something broke");
                assert_eq!(source, "console");
            }
            other => panic!("expected JsError, got {:?}", other),
        }
    }

    #[test]
    fn classify_module_error() {
        let event = classify_console_message("[MODULE-ERROR] Failed to load ./app.js", 0);
        match event {
            TraceEvent::JsError { message, .. } => {
                assert!(message.contains("MODULE-ERROR"));
            }
            other => panic!("expected JsError, got {:?}", other),
        }
    }

    #[test]
    fn classify_script_error() {
        let event = classify_console_message("[script-error] ReferenceError: x is not defined", 0);
        match event {
            TraceEvent::JsError { message, .. } => {
                assert!(message.contains("script-error"));
            }
            other => panic!("expected JsError, got {:?}", other),
        }
    }

    #[test]
    fn classify_warn() {
        let event = classify_console_message("[warn] deprecated API usage", 5);
        match event {
            TraceEvent::Console { level, message, .. } => {
                assert_eq!(level, "warn");
                assert_eq!(message, "[warn] deprecated API usage");
            }
            other => panic!("expected Console warn, got {:?}", other),
        }
    }

    #[test]
    fn classify_uncaught() {
        let event = classify_console_message("[uncaught] TypeError: null is not an object", 0);
        match event {
            TraceEvent::JsError { message, .. } => {
                assert!(message.contains("[uncaught]"));
            }
            other => panic!("expected JsError, got {:?}", other),
        }
    }

    #[test]
    fn classify_unhandled_rejection() {
        let event = classify_console_message(
            "[unhandled-rejection] Promise rejected with: network error",
            0,
        );
        match event {
            TraceEvent::JsError { message, .. } => {
                assert!(message.contains("[unhandled-rejection]"));
            }
            other => panic!("expected JsError, got {:?}", other),
        }
    }

    #[test]
    fn classify_case_insensitive() {
        let event = classify_console_message("[ERROR] uppercase works too", 0);
        match event {
            TraceEvent::JsError { .. } => {}
            other => panic!("expected JsError, got {:?}", other),
        }
    }

    #[test]
    fn classify_error_mid_message() {
        let event = classify_console_message("React: [error] hydration mismatch", 0);
        match event {
            TraceEvent::JsError { message, .. } => {
                assert_eq!(message, "React: [error] hydration mismatch");
            }
            other => panic!("expected JsError, got {:?}", other),
        }
    }

    #[test]
    fn classify_preserves_timestamp() {
        let event = classify_console_message("hello", 42);
        match event {
            TraceEvent::Console { timestamp_ms, .. } => {
                assert_eq!(timestamp_ms, 42);
            }
            other => panic!("expected Console, got {:?}", other),
        }
    }
}
