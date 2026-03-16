//! Action tracing and observability.
//!
//! Records every browser action with timing, outcome, and optional state snapshots.
//! Enables debugging, replay analysis, and success rate tracking.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionTrace {
    pub id: u64,
    pub action: String,
    pub target: String,
    pub outcome: String,       // "succeeded", "not_found", "error", "timeout"
    pub effect: String,
    pub duration_ms: u64,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionStats {
    pub total: u64,
    pub succeeded: u64,
    pub failed: u64,
    pub avg_duration_ms: u64,
    pub by_action: std::collections::HashMap<String, ActionTypeStats>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionTypeStats {
    pub count: u64,
    pub success_rate: f64,
    pub avg_ms: u64,
}

pub struct TraceLog {
    traces: Vec<ActionTrace>,
    next_id: u64,
    enabled: bool,
}

impl TraceLog {
    pub fn new() -> Self {
        Self {
            traces: Vec::new(),
            next_id: 1,
            enabled: false,
        }
    }

    pub fn enable(&mut self) {
        self.enabled = true;
    }

    pub fn disable(&mut self) {
        self.enabled = false;
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn record(
        &mut self,
        action: &str,
        target: &str,
        outcome: &str,
        effect: &str,
        duration_ms: u64,
        url: &str,
        error: Option<String>,
    ) -> u64 {
        if !self.enabled {
            return 0;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.traces.push(ActionTrace {
            id,
            action: action.into(),
            target: target.into(),
            outcome: outcome.into(),
            effect: effect.into(),
            duration_ms,
            url: url.into(),
            error,
            timestamp: chrono::Utc::now().to_rfc3339(),
        });
        id
    }

    pub fn read(&self, last_n: Option<usize>) -> Vec<&ActionTrace> {
        let n = last_n.unwrap_or(self.traces.len());
        self.traces.iter().rev().take(n).collect()
    }

    pub fn clear(&mut self) {
        self.traces.clear();
    }

    pub fn stats(&self) -> ActionStats {
        let total = self.traces.len() as u64;
        let succeeded = self.traces.iter().filter(|t| t.outcome == "succeeded").count() as u64;
        let failed = total - succeeded;
        let avg_duration_ms = if total > 0 {
            self.traces.iter().map(|t| t.duration_ms).sum::<u64>() / total
        } else {
            0
        };

        let mut by_action: std::collections::HashMap<String, (u64, u64, u64)> =
            std::collections::HashMap::new();
        for t in &self.traces {
            let entry = by_action.entry(t.action.clone()).or_insert((0, 0, 0));
            entry.0 += 1;
            if t.outcome == "succeeded" || t.outcome == "ok" {
                entry.1 += 1;
            }
            entry.2 += t.duration_ms;
        }

        let by_action = by_action
            .into_iter()
            .map(|(k, (count, succ, total_ms))| {
                (
                    k,
                    ActionTypeStats {
                        count,
                        success_rate: if count > 0 { succ as f64 / count as f64 } else { 0.0 },
                        avg_ms: if count > 0 { total_ms / count } else { 0 },
                    },
                )
            })
            .collect();

        ActionStats {
            total,
            succeeded,
            failed,
            avg_duration_ms,
            by_action,
        }
    }

    pub fn export_json(&self) -> Value {
        serde_json::to_value(&self.traces).unwrap_or(Value::Array(vec![]))
    }
}
