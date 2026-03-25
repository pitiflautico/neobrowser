//! `trace` tool — execution trace and summary for AI decision-making.

use serde_json::Value;

use neo_runtime::trace_events::{ModulePhase, PhaseAction, TraceEvent};

use crate::state::McpState;
use crate::McpError;

use super::ToolDef;

/// Tool definition for `tools/list`.
pub(crate) fn definition() -> ToolDef {
    ToolDef {
        name: "trace",
        description: "Get execution trace, errors, console, modules, network, or timeline",
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "kind": {
                    "type": "string",
                    "enum": ["summary", "full", "last_action", "errors", "console", "modules", "network", "timeline"],
                    "description": "What trace data to return: summary, full action trace, last_action, errors (JS errors only), console (console output), modules (module lifecycle), network (HTTP requests), timeline (all events chronologically)"
                }
            },
            "required": ["kind"]
        }),
    }
}

/// Execute the `trace` tool.
pub fn call(args: Value, state: &mut McpState) -> Result<Value, McpError> {
    let kind = args
        .get("kind")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::InvalidParams("missing 'kind'".into()))?;

    match kind {
        "summary" => {
            let summary = state.engine.summary();
            Ok(serde_json::to_value(summary)?)
        }
        "full" => {
            let entries = state.engine.trace();
            Ok(serde_json::to_value(entries)?)
        }
        "last_action" => {
            let entries = state.engine.trace();
            let last = entries.last().cloned();
            Ok(serde_json::to_value(last)?)
        }
        "errors" => {
            let events = state.engine.drain_trace_events();
            let formatted = format_filtered(&events, |e| matches!(e, TraceEvent::JsError { .. }));
            Ok(text_result(&formatted))
        }
        "console" => {
            let events = state.engine.drain_trace_events();
            let formatted = format_filtered(&events, |e| matches!(e, TraceEvent::Console { .. }));
            Ok(text_result(&formatted))
        }
        "modules" => {
            let events = state.engine.drain_trace_events();
            let formatted = format_filtered(&events, |e| matches!(e, TraceEvent::Module { .. }));
            Ok(text_result(&formatted))
        }
        "network" => {
            let events = state.engine.drain_trace_events();
            let formatted = format_filtered(&events, |e| matches!(e, TraceEvent::Network { .. }));
            Ok(text_result(&formatted))
        }
        "timeline" => {
            let mut events = state.engine.drain_trace_events();
            events.sort_by_key(|e| event_timestamp(e));
            let formatted = format_timeline(&events);
            Ok(text_result(&formatted))
        }
        other => Err(McpError::InvalidParams(format!("unknown kind: {other}"))),
    }
}

/// Wrap a string in the MCP text content format.
fn text_result(text: &str) -> Value {
    serde_json::json!([{
        "type": "text",
        "text": text
    }])
}

/// Format only the events matching a predicate.
fn format_filtered(events: &[TraceEvent], pred: impl Fn(&TraceEvent) -> bool) -> String {
    let filtered: Vec<&TraceEvent> = events.iter().filter(|e| pred(e)).collect();
    if filtered.is_empty() {
        return "(no matching events)".to_string();
    }
    filtered
        .iter()
        .map(|e| format_trace_event(e))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Format all events as a chronological timeline.
pub fn format_timeline(events: &[TraceEvent]) -> String {
    if events.is_empty() {
        return "(no trace events)".to_string();
    }
    let mut lines = Vec::with_capacity(events.len() + 2);
    lines.push(format!(
        "── TRACE ({} events) ──────────────────",
        events.len()
    ));
    for event in events {
        lines.push(format_trace_event(event));
    }
    lines.push("── END TRACE ──────────────────────────".to_string());
    lines.join("\n")
}

/// Extract the timestamp_ms from any TraceEvent variant.
pub fn event_timestamp(event: &TraceEvent) -> u64 {
    match event {
        TraceEvent::Console { timestamp_ms, .. } => *timestamp_ms,
        TraceEvent::JsError { timestamp_ms, .. } => *timestamp_ms,
        TraceEvent::Module { timestamp_ms, .. } => *timestamp_ms,
        TraceEvent::Network { timestamp_ms, .. } => *timestamp_ms,
        TraceEvent::Phase { timestamp_ms, .. } => *timestamp_ms,
    }
}

/// Format a single trace event as a human-readable line.
///
/// Multi-line for events with extra context (stack traces, etc).
pub fn format_trace_event(event: &TraceEvent) -> String {
    match event {
        TraceEvent::Console {
            level,
            message,
            timestamp_ms,
        } => {
            format!("[{:>4}ms] CONSOLE [{}] {}", timestamp_ms, level, message)
        }
        TraceEvent::JsError {
            message,
            stack,
            source,
            timestamp_ms,
        } => {
            let mut line = format!("[{:>4}ms] JS-ERR {}", timestamp_ms, message);
            if let Some(st) = stack {
                line.push_str(&format!("\n        Stack: {}", st));
            }
            if !source.is_empty() {
                line.push_str(&format!("\n        Source: {}", source));
            }
            line
        }
        TraceEvent::Module {
            url,
            event: phase,
            detail,
            timestamp_ms,
        } => {
            let phase_str = format_module_phase(phase);
            let short_url = shorten_url(url);
            let mut line = format!(
                "[{:>4}ms] MODULE {} {}",
                timestamp_ms, short_url, phase_str
            );
            if let Some(d) = detail {
                line.push_str(&format!(" ({})", d));
            }
            line
        }
        TraceEvent::Network {
            url,
            method,
            status,
            size_bytes,
            duration_ms,
            timestamp_ms,
        } => {
            let short_url = shorten_url(url);
            let size_str = format_size(*size_bytes);
            format!(
                "[{:>4}ms] NET {} {} → {} ({}, {}ms)",
                timestamp_ms, method, short_url, status, size_str, duration_ms
            )
        }
        TraceEvent::Phase {
            name,
            action,
            detail,
            timestamp_ms,
        } => {
            let action_str = format_phase_action(action);
            let mut line = format!(
                "[{:>4}ms] PHASE {} {}",
                timestamp_ms, name, action_str
            );
            if let Some(d) = detail {
                line.push_str(&format!(" ({})", d));
            }
            line
        }
    }
}

fn format_module_phase(phase: &ModulePhase) -> &'static str {
    match phase {
        ModulePhase::Resolve => "RESOLVE",
        ModulePhase::Load => "LOAD",
        ModulePhase::Instantiate => "INSTANTIATE",
        ModulePhase::Evaluate => "EVALUATE",
        ModulePhase::Success => "SUCCESS",
        ModulePhase::Error => "ERROR",
    }
}

fn format_phase_action(action: &PhaseAction) -> &'static str {
    match action {
        PhaseAction::Start => "START",
        PhaseAction::End => "END",
        PhaseAction::Error => "ERROR",
    }
}

/// Shorten a URL for display: keep filename or last path segment.
fn shorten_url(url: &str) -> &str {
    if url.len() <= 60 {
        return url;
    }
    // Try to get the last path segment
    url.rsplit('/').next().unwrap_or(url)
}

/// Format byte size for display.
fn format_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{}B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{}KB", bytes / 1024)
    } else {
        format!("{:.1}MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use neo_runtime::trace_events::{ModulePhase, PhaseAction, TraceEvent};

    #[test]
    fn format_console_event() {
        let event = TraceEvent::Console {
            level: "warn".into(),
            message: "deprecated API".into(),
            timestamp_ms: 400,
        };
        let out = format_trace_event(&event);
        assert_eq!(out, "[ 400ms] CONSOLE [warn] deprecated API");
    }

    #[test]
    fn format_js_error_with_stack() {
        let event = TraceEvent::JsError {
            message: "Buffer2.concat is not a function".into(),
            stack: Some("at vendor.js:1:277737".into()),
            source: "vendor.js".into(),
            timestamp_ms: 350,
        };
        let out = format_trace_event(&event);
        assert!(out.contains("[ 350ms] JS-ERR Buffer2.concat is not a function"));
        assert!(out.contains("Stack: at vendor.js:1:277737"));
        assert!(out.contains("Source: vendor.js"));
    }

    #[test]
    fn format_js_error_without_stack() {
        let event = TraceEvent::JsError {
            message: "TypeError: x is null".into(),
            stack: None,
            source: "inline".into(),
            timestamp_ms: 50,
        };
        let out = format_trace_event(&event);
        assert!(out.contains("JS-ERR TypeError: x is null"));
        assert!(!out.contains("Stack:"));
        assert!(out.contains("Source: inline"));
    }

    #[test]
    fn format_module_resolve() {
        let event = TraceEvent::Module {
            url: "https://cdn.example.com/vendor.js".into(),
            event: ModulePhase::Resolve,
            detail: Some("https://cdn.example.com/vendor.js".into()),
            timestamp_ms: 15,
        };
        let out = format_trace_event(&event);
        assert!(out.contains("MODULE"));
        assert!(out.contains("RESOLVE"));
    }

    #[test]
    fn format_module_load_with_detail() {
        let event = TraceEvent::Module {
            url: "app.js".into(),
            event: ModulePhase::Load,
            detail: Some("28041KB".into()),
            timestamp_ms: 20,
        };
        let out = format_trace_event(&event);
        assert!(out.contains("[  20ms] MODULE app.js LOAD (28041KB)"));
    }

    #[test]
    fn format_network_event() {
        let event = TraceEvent::Network {
            url: "https://api.example.com/data".into(),
            method: "GET".into(),
            status: 200,
            size_bytes: 4096,
            duration_ms: 150,
            timestamp_ms: 100,
        };
        let out = format_trace_event(&event);
        assert!(out.contains("NET GET"));
        assert!(out.contains("200"));
        assert!(out.contains("4KB"));
        assert!(out.contains("150ms"));
    }

    #[test]
    fn format_phase_start() {
        let event = TraceEvent::Phase {
            name: "bootstrap".into(),
            action: PhaseAction::Start,
            detail: None,
            timestamp_ms: 0,
        };
        let out = format_trace_event(&event);
        assert_eq!(out, "[   0ms] PHASE bootstrap START");
    }

    #[test]
    fn format_phase_end_with_detail() {
        let event = TraceEvent::Phase {
            name: "hydration".into(),
            action: PhaseAction::End,
            detail: Some("ok".into()),
            timestamp_ms: 1200,
        };
        let out = format_trace_event(&event);
        assert_eq!(out, "[1200ms] PHASE hydration END (ok)");
    }

    #[test]
    fn format_size_bytes() {
        assert_eq!(format_size(512), "512B");
        assert_eq!(format_size(1024), "1KB");
        assert_eq!(format_size(4096), "4KB");
        assert_eq!(format_size(1048576), "1.0MB");
        assert_eq!(format_size(1572864), "1.5MB");
    }

    #[test]
    fn shorten_long_url() {
        let long = "https://cdn.example.com/assets/js/chunks/vendor-abc123.js";
        assert!(long.len() < 60); // still short enough
        assert_eq!(shorten_url(long), long);

        let very_long = "https://cdn.example.com/assets/js/chunks/some/deeply/nested/path/vendor-abc123456789.js";
        assert_eq!(shorten_url(very_long), "vendor-abc123456789.js");
    }

    #[test]
    fn timeline_sorts_by_timestamp() {
        let events = vec![
            TraceEvent::Console {
                level: "log".into(),
                message: "second".into(),
                timestamp_ms: 200,
            },
            TraceEvent::Phase {
                name: "bootstrap".into(),
                action: PhaseAction::Start,
                detail: None,
                timestamp_ms: 0,
            },
            TraceEvent::JsError {
                message: "err".into(),
                stack: None,
                source: "".into(),
                timestamp_ms: 100,
            },
        ];

        // timeline format should sort by timestamp
        let mut sorted = events.clone();
        sorted.sort_by_key(|e| event_timestamp(e));
        let out = format_timeline(&sorted);

        let lines: Vec<&str> = out.lines().collect();
        // First line is header, then events in order
        assert!(lines[0].contains("TRACE (3 events)"));
        assert!(lines[1].contains("[   0ms] PHASE bootstrap START"));
        assert!(lines[2].contains("[ 100ms] JS-ERR err"));
        assert!(lines[3].contains("[ 200ms] CONSOLE [log] second"));
        assert!(lines[4].contains("END TRACE"));
    }

    #[test]
    fn timeline_empty() {
        let out = format_timeline(&[]);
        assert_eq!(out, "(no trace events)");
    }

    #[test]
    fn filter_errors_only() {
        let events = vec![
            TraceEvent::Console {
                level: "log".into(),
                message: "hello".into(),
                timestamp_ms: 10,
            },
            TraceEvent::JsError {
                message: "boom".into(),
                stack: None,
                source: "eval".into(),
                timestamp_ms: 20,
            },
            TraceEvent::Phase {
                name: "test".into(),
                action: PhaseAction::Start,
                detail: None,
                timestamp_ms: 30,
            },
        ];
        let out = format_filtered(&events, |e| matches!(e, TraceEvent::JsError { .. }));
        assert!(out.contains("JS-ERR boom"));
        assert!(!out.contains("CONSOLE"));
        assert!(!out.contains("PHASE"));
    }

    #[test]
    fn filter_no_matches() {
        let events = vec![TraceEvent::Console {
            level: "log".into(),
            message: "hello".into(),
            timestamp_ms: 10,
        }];
        let out = format_filtered(&events, |e| matches!(e, TraceEvent::JsError { .. }));
        assert_eq!(out, "(no matching events)");
    }

    #[test]
    fn event_timestamp_extraction() {
        assert_eq!(
            event_timestamp(&TraceEvent::Console {
                level: "log".into(),
                message: "".into(),
                timestamp_ms: 42,
            }),
            42
        );
        assert_eq!(
            event_timestamp(&TraceEvent::JsError {
                message: "".into(),
                stack: None,
                source: "".into(),
                timestamp_ms: 99,
            }),
            99
        );
        assert_eq!(
            event_timestamp(&TraceEvent::Network {
                url: "".into(),
                method: "GET".into(),
                status: 200,
                size_bytes: 0,
                duration_ms: 0,
                timestamp_ms: 77,
            }),
            77
        );
    }
}
