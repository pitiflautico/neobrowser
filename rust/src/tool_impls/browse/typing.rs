//! The type tool: send keystrokes to the focused element.
//!
//! Keystrokes rather than a value assignment, because they are what a real user produces and
//! therefore what a framework's `keydown`/`input` handlers see. `human=true` spaces them with
//! a human cadence, which matters to behavioural anti-bot systems in a way an instant paste
//! does not.

use super::super::{arg_bool, arg_f64, arg_str, verified};
use crate::page;
use crate::tools::{ParamSpec, ParamType, Tool, ToolCtx, ToolError, ToolOutput, ToolSpec};
use async_trait::async_trait;
use serde_json::{Map, Value};

pub struct TypeTool;

#[async_trait]
impl Tool for TypeTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "type",
            description: "Type text into the focused element. Default: instant insert (React/Vue-safe). Set human=true for per-key events with human cadence.",
            params: vec![
                ParamSpec::new("text", ParamType::String, "Text to type").required(),
                ParamSpec::new("human", ParamType::Boolean, "Type key-by-key with human-like timing (default false)"),
                ParamSpec::new("budget_s", ParamType::Number, "Seconds to wait for the field to reflect the change before reporting 'uncertain' (default 3)"),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let text = arg_str(args, "text")
            .ok_or_else(|| ToolError::Argument("type: text must be a string".into()))?;
        let human = arg_bool(args, "human", false);
        let tab = ctx.browser.tab().await?;
        let n = text.chars().count();
        // Typing changes a control's value, which the state digest records as a
        // length — so "typed 12 chars" is now backed by the field having changed.
        // Bind a reference so the async block does not move `tab`, which `verified`
        // still needs for its before/after observations.
        let tab_ref = &tab;
        verified(
            &tab,
            "type",
            arg_f64(args, "budget_s", 3.0),
            || async move {
                page::type_text(tab_ref, text, human).await?;
                Ok(format!("typed {n} chars into the focused element"))
            },
        )
        .await
    }
}
