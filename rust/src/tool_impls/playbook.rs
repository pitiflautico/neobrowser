//! Recording and replaying tool sequences.
//!
//! Split out of a single 2700-line `tool_impls.rs`: at 67 tools that file had
//! stopped being navigable, and a reviewer could not tell which tools a change
//! touched.

use async_trait::async_trait;
use serde_json::{json, Map, Value};

use crate::tools::{ParamSpec, ParamType, Tool, ToolCtx, ToolError, ToolOutput, ToolSpec};

use super::arg_str;

// --- record_task / stop_recording / replay ------------------------------------

pub struct RecordTaskTool;

#[async_trait]
impl Tool for RecordTaskTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "record_task",
            description: "Start recording interaction steps as a playbook for later replay.",
            params: vec![
                ParamSpec::new(
                    "domain",
                    ParamType::String,
                    "Domain key, e.g. 'linkedin.com'",
                )
                .required(),
                ParamSpec::new(
                    "task_name",
                    ParamType::String,
                    "Task identifier, e.g. 'send_message'",
                )
                .required(),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let domain = arg_str(args, "domain")
            .ok_or_else(|| ToolError::Argument("record_task: domain must be a string".into()))?;
        let task = arg_str(args, "task_name")
            .ok_or_else(|| ToolError::Argument("record_task: task_name must be a string".into()))?;
        ctx.browser.start_recording(domain, task).await;
        Ok(ToolOutput::text(format!(
            "Recording started: {domain}/{task}"
        )))
    }
}

pub struct ReplayTool;

#[async_trait]
impl Tool for ReplayTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "replay",
            description: "Replay a recorded playbook by re-invoking each saved step. Returns ok + the first failed step index (0 = none).",
            params: vec![
                ParamSpec::new("domain", ParamType::String, "Domain key").required(),
                ParamSpec::new("task_name", ParamType::String, "Task name").required(),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let domain = arg_str(args, "domain")
            .ok_or_else(|| ToolError::Argument("replay: domain must be a string".into()))?;
        let task = arg_str(args, "task_name")
            .ok_or_else(|| ToolError::Argument("replay: task_name must be a string".into()))?;
        let steps = crate::playbook::load(domain, task);
        if steps.is_empty() {
            return Ok(ToolOutput::text(json!({ "ok": false, "error": "playbook not found or empty", "first_failed_step": 0 }).to_string()));
        }
        let mut first_failed = 0usize;
        for (i, step) in steps.iter().enumerate() {
            let tool_name = step.get("tool").and_then(|v| v.as_str()).unwrap_or("");
            let step_args = step
                .get("args")
                .and_then(|v| v.as_object())
                .cloned()
                .unwrap_or_default();
            let Some(tool) = ctx.registry.get(tool_name) else {
                first_failed = i + 1;
                break;
            };
            if tool.spec().validate_args(&step_args).is_err()
                || tool.call(ctx, &step_args).await.is_err()
            {
                first_failed = i + 1;
                break;
            }
        }
        Ok(ToolOutput::text(
            json!({ "ok": first_failed == 0, "steps": steps.len(), "first_failed_step": first_failed }).to_string(),
        ))
    }
}

// --- multi-tab management ------------------------------------------------------

pub struct StopRecordingTool;

#[async_trait]
impl Tool for StopRecordingTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "stop_recording",
            description:
                "Stop recording and save the playbook. Returns the number of steps captured.",
            params: vec![],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        _args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let n = ctx.browser.stop_recording().await;
        Ok(ToolOutput::text(
            json!({ "steps": n, "saved": n > 0 }).to_string(),
        ))
    }
}
