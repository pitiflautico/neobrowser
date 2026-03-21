//! neo-trace — observability layer for NeoRender AI browser.
//!
//! Structured execution records that an AI reads to decide what to do next.
//! Every action, network request, state change, and DOM mutation is traced.

pub mod file_tracer;
pub mod mock;
pub mod noop;
pub mod redaction;
pub mod summary;
pub mod tracer;

use neo_types::{PageState, TraceEntry};
use serde::{Deserialize, Serialize};

/// Navigation lifecycle event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NavEvent {
    Started,
    Committed,
    Finished,
    Failed,
}

/// Compact execution summary for AI decision-making.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionSummary {
    pub total_actions: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub total_requests: usize,
    pub blocked_requests: usize,
    pub dom_changes: usize,
    pub js_errors: usize,
    pub duration_ms: u64,
    pub warnings: Vec<String>,
    pub state: PageState,
}

/// A network event with causal link to the triggering action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkEvent<'a> {
    pub request_id: &'a str,
    pub url: &'a str,
    pub method: &'a str,
    pub status: u16,
    pub duration_ms: u64,
    pub action_id: Option<&'a str>,
    pub frame_id: Option<&'a str>,
    pub kind: &'a str,
}

/// Tracer errors.
#[derive(Debug, thiserror::Error)]
pub enum TraceError {
    #[error("failed to serialize trace: {0}")]
    Serialize(#[from] serde_json::Error),

    #[error("failed to write trace file: {0}")]
    Io(#[from] std::io::Error),
}

/// Observability trait — every action, request, and state change is recorded.
///
/// This is NOT logging for humans. It's a structured execution record
/// that another AI reads to decide what to do next.
pub trait Tracer: Send + Sync {
    /// AI declares what it intends to do (before action).
    fn intent(&self, action_id: &str, intent: &str, target: &str, confidence: f32);

    /// Action completed — what actually happened.
    fn action_result(&self, action_id: &str, success: bool, effect: &str, error: Option<&str>);

    /// Network request with causal link to action.
    fn network(&self, event: &NetworkEvent<'_>);

    /// Navigation lifecycle event.
    fn navigation(&self, event: NavEvent, url: &str, nav_id: &str, status: Option<u16>);

    /// Page state changed.
    fn state_change(&self, from: PageState, to: PageState, reason: &str);

    /// DOM diff summary (not raw mutations — summarized for AI).
    fn dom_diff(&self, added: usize, removed: usize, changed: usize, summary: &str);

    /// JS console message.
    fn console(&self, level: &str, message: &str);

    /// JS exception.
    fn js_exception(&self, error: &str, stack: Option<&str>);

    /// Resource blocked (telemetry, ads, etc.).
    fn resource_blocked(&self, url: &str, reason: &str);

    /// Export full trace as entries.
    fn export(&self) -> Vec<TraceEntry>;

    /// Execution summary — compact overview for AI decision-making.
    fn summary(&self) -> ExecutionSummary;
}
