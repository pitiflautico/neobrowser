//! Deterministic pipeline runner.
//!
//! Executes JSON-defined step sequences: goto, click, type, wait, assert, extract, branch.
//! Each step has retry policy, timeout, and postconditions.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pipeline {
    pub name: String,
    pub steps: Vec<Step>,
    #[serde(default)]
    pub variables: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    pub action: String,   // goto, click, type, wait, assert, extract, eval, screenshot
    #[serde(default)]
    pub target: String,
    #[serde(default)]
    pub value: String,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
    #[serde(default = "default_retries")]
    pub max_retries: u32,
    #[serde(default)]
    pub assert_text: Option<String>,  // postcondition: page must contain this text
    #[serde(default)]
    pub store_as: Option<String>,  // store result in variable
    #[serde(default)]
    pub on_fail: OnFail,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnFail {
    Abort,
    Skip,
    Continue,
}

impl Default for OnFail {
    fn default() -> Self { OnFail::Abort }
}

fn default_timeout() -> u64 { 5000 }
fn default_retries() -> u32 { 2 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    pub step_index: usize,
    pub action: String,
    pub outcome: String,  // "ok", "failed", "skipped"
    pub detail: String,
    pub duration_ms: u64,
    pub retries_used: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineResult {
    pub name: String,
    pub status: String,  // "completed", "aborted", "partial"
    pub steps_completed: usize,
    pub steps_total: usize,
    pub total_ms: u64,
    pub results: Vec<StepResult>,
    pub variables: std::collections::HashMap<String, String>,
}

impl Pipeline {
    pub fn from_json(json: &str) -> Result<Self, String> {
        serde_json::from_str(json).map_err(|e| format!("Invalid pipeline JSON: {e}"))
    }
}
