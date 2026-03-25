//! Structured trace events for browser engine diagnostics.
//! Events flow: JS → op_console_log → TraceBuffer → drain_trace_events() → consumer

use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "kind")]
pub enum TraceEvent {
    /// Console output (log, warn, error, info, debug)
    Console {
        level: String,
        message: String,
        timestamp_ms: u64,
    },
    /// JavaScript error with optional stack trace
    JsError {
        message: String,
        stack: Option<String>,
        source: String,
        timestamp_ms: u64,
    },
    /// Module lifecycle event
    Module {
        url: String,
        event: ModulePhase,
        detail: Option<String>,
        timestamp_ms: u64,
    },
    /// Network request
    Network {
        url: String,
        method: String,
        status: u16,
        size_bytes: usize,
        duration_ms: u64,
        timestamp_ms: u64,
    },
    /// Pipeline phase (bootstrap, hydration, settle, etc)
    Phase {
        name: String,
        action: PhaseAction,
        detail: Option<String>,
        timestamp_ms: u64,
    },
}

#[derive(Debug, Clone, serde::Serialize)]
pub enum ModulePhase {
    Resolve,
    Load,
    Instantiate,
    Evaluate,
    Success,
    Error,
}

#[derive(Debug, Clone, serde::Serialize)]
pub enum PhaseAction {
    Start,
    End,
    Error,
}

/// Thread-safe buffer that collects trace events during V8 execution.
/// Lives in deno_core OpState so ops can push events.
#[derive(Clone)]
pub struct TraceBuffer {
    events: Arc<Mutex<Vec<TraceEvent>>>,
    start: Arc<Mutex<Instant>>,
}

impl TraceBuffer {
    pub fn new() -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
            start: Arc::new(Mutex::new(Instant::now())),
        }
    }

    /// Push a trace event into the buffer.
    pub fn push(&self, event: TraceEvent) {
        if let Ok(mut events) = self.events.lock() {
            events.push(event);
        }
    }

    /// Drain all events from the buffer, returning them and clearing it.
    pub fn drain(&self) -> Vec<TraceEvent> {
        if let Ok(mut events) = self.events.lock() {
            events.drain(..).collect()
        } else {
            vec![]
        }
    }

    /// Milliseconds elapsed since the buffer was created.
    pub fn elapsed_ms(&self) -> u64 {
        if let Ok(start) = self.start.lock() {
            start.elapsed().as_millis() as u64
        } else {
            0
        }
    }

    /// Push a console event.
    pub fn console(&self, level: &str, message: &str) {
        self.push(TraceEvent::Console {
            level: level.to_string(),
            message: message.to_string(),
            timestamp_ms: self.elapsed_ms(),
        });
    }

    /// Push a JS error event.
    pub fn js_error(&self, message: &str, stack: Option<&str>, source: &str) {
        self.push(TraceEvent::JsError {
            message: message.to_string(),
            stack: stack.map(|s| s.to_string()),
            source: source.to_string(),
            timestamp_ms: self.elapsed_ms(),
        });
    }

    /// Push a module lifecycle event.
    pub fn module_event(&self, url: &str, phase: ModulePhase, detail: Option<&str>) {
        self.push(TraceEvent::Module {
            url: url.to_string(),
            event: phase,
            detail: detail.map(|s| s.to_string()),
            timestamp_ms: self.elapsed_ms(),
        });
    }

    /// Push a network event.
    pub fn network(&self, url: &str, method: &str, status: u16, size: usize, duration_ms: u64) {
        self.push(TraceEvent::Network {
            url: url.to_string(),
            method: method.to_string(),
            status,
            size_bytes: size,
            duration_ms,
            timestamp_ms: self.elapsed_ms(),
        });
    }

    /// Push a pipeline phase event.
    pub fn phase(&self, name: &str, action: PhaseAction, detail: Option<&str>) {
        self.push(TraceEvent::Phase {
            name: name.to_string(),
            action,
            detail: detail.map(|s| s.to_string()),
            timestamp_ms: self.elapsed_ms(),
        });
    }

    /// Number of events currently in the buffer.
    pub fn len(&self) -> usize {
        if let Ok(events) = self.events.lock() {
            events.len()
        } else {
            0
        }
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for TraceBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_creates_empty_buffer() {
        let buf = TraceBuffer::new();
        assert!(buf.is_empty());
        assert_eq!(buf.len(), 0);
        assert!(buf.drain().is_empty());
    }

    #[test]
    fn push_adds_events_drain_returns_and_clears() {
        let buf = TraceBuffer::new();
        buf.console("log", "hello");
        buf.console("warn", "world");
        assert_eq!(buf.len(), 2);

        let events = buf.drain();
        assert_eq!(events.len(), 2);
        assert!(buf.is_empty());
    }

    #[test]
    fn drain_is_idempotent() {
        let buf = TraceBuffer::new();
        buf.console("log", "one");
        let first = buf.drain();
        assert_eq!(first.len(), 1);
        let second = buf.drain();
        assert!(second.is_empty());
    }

    #[test]
    fn console_helper_creates_correct_event() {
        let buf = TraceBuffer::new();
        buf.console("error", "something broke");
        let events = buf.drain();
        assert_eq!(events.len(), 1);
        match &events[0] {
            TraceEvent::Console { level, message, .. } => {
                assert_eq!(level, "error");
                assert_eq!(message, "something broke");
            }
            other => panic!("expected Console, got {:?}", other),
        }
    }

    #[test]
    fn js_error_helper_creates_correct_event() {
        let buf = TraceBuffer::new();
        buf.js_error("ReferenceError: x", Some("at line 1"), "inline");
        let events = buf.drain();
        assert_eq!(events.len(), 1);
        match &events[0] {
            TraceEvent::JsError {
                message,
                stack,
                source,
                ..
            } => {
                assert_eq!(message, "ReferenceError: x");
                assert_eq!(stack.as_deref(), Some("at line 1"));
                assert_eq!(source, "inline");
            }
            other => panic!("expected JsError, got {:?}", other),
        }
    }

    #[test]
    fn js_error_without_stack() {
        let buf = TraceBuffer::new();
        buf.js_error("TypeError", None, "eval");
        let events = buf.drain();
        match &events[0] {
            TraceEvent::JsError { stack, .. } => {
                assert!(stack.is_none());
            }
            other => panic!("expected JsError, got {:?}", other),
        }
    }

    #[test]
    fn module_event_helper_creates_correct_event() {
        let buf = TraceBuffer::new();
        buf.module_event("https://cdn.example.com/app.js", ModulePhase::Load, Some("200 OK"));
        let events = buf.drain();
        assert_eq!(events.len(), 1);
        match &events[0] {
            TraceEvent::Module {
                url, event, detail, ..
            } => {
                assert_eq!(url, "https://cdn.example.com/app.js");
                assert!(matches!(event, ModulePhase::Load));
                assert_eq!(detail.as_deref(), Some("200 OK"));
            }
            other => panic!("expected Module, got {:?}", other),
        }
    }

    #[test]
    fn network_helper_creates_correct_event() {
        let buf = TraceBuffer::new();
        buf.network("https://api.example.com/data", "GET", 200, 4096, 150);
        let events = buf.drain();
        assert_eq!(events.len(), 1);
        match &events[0] {
            TraceEvent::Network {
                url,
                method,
                status,
                size_bytes,
                duration_ms,
                ..
            } => {
                assert_eq!(url, "https://api.example.com/data");
                assert_eq!(method, "GET");
                assert_eq!(*status, 200);
                assert_eq!(*size_bytes, 4096);
                assert_eq!(*duration_ms, 150);
            }
            other => panic!("expected Network, got {:?}", other),
        }
    }

    #[test]
    fn phase_helper_creates_correct_event() {
        let buf = TraceBuffer::new();
        buf.phase("hydration", PhaseAction::Start, None);
        buf.phase("hydration", PhaseAction::End, Some("ok"));
        let events = buf.drain();
        assert_eq!(events.len(), 2);
        match &events[0] {
            TraceEvent::Phase {
                name,
                action,
                detail,
                ..
            } => {
                assert_eq!(name, "hydration");
                assert!(matches!(action, PhaseAction::Start));
                assert!(detail.is_none());
            }
            other => panic!("expected Phase, got {:?}", other),
        }
        match &events[1] {
            TraceEvent::Phase { action, detail, .. } => {
                assert!(matches!(action, PhaseAction::End));
                assert_eq!(detail.as_deref(), Some("ok"));
            }
            other => panic!("expected Phase, got {:?}", other),
        }
    }

    #[test]
    fn thread_safety_push_from_multiple_threads() {
        let buf = TraceBuffer::new();
        let mut handles = vec![];
        for i in 0..10 {
            let buf_clone = buf.clone();
            handles.push(std::thread::spawn(move || {
                buf_clone.console("log", &format!("thread {i}"));
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(buf.len(), 10);
        let events = buf.drain();
        assert_eq!(events.len(), 10);
    }

    #[test]
    fn elapsed_ms_increases_over_time() {
        let buf = TraceBuffer::new();
        let t0 = buf.elapsed_ms();
        std::thread::sleep(std::time::Duration::from_millis(15));
        let t1 = buf.elapsed_ms();
        assert!(t1 > t0, "elapsed should increase: t0={t0}, t1={t1}");
    }

    #[test]
    fn len_and_is_empty_work_correctly() {
        let buf = TraceBuffer::new();
        assert!(buf.is_empty());
        assert_eq!(buf.len(), 0);
        buf.console("info", "a");
        assert!(!buf.is_empty());
        assert_eq!(buf.len(), 1);
        buf.console("debug", "b");
        assert_eq!(buf.len(), 2);
        buf.drain();
        assert!(buf.is_empty());
        assert_eq!(buf.len(), 0);
    }
}
