//! Tools for filling and submitting forms.
//!
//! `form_fill` locates each field at the moment it fills it, because a real form re-renders
//! between fields; `submit` then has to decide whether anything actually happened, since a
//! form that silently fails validation looks identical to one that succeeded.

//! The core loop: navigate, observe, act, verify, extract.
//!
//! Split out of a single 2700-line `tool_impls.rs`: at 67 tools that file had
//! stopped being navigable, and a reviewer could not tell which tools a change
//! touched.

use async_trait::async_trait;
use serde_json::{Map, Value};

use crate::ops;
use crate::tools::{ParamSpec, ParamType, Tool, ToolCtx, ToolError, ToolOutput, ToolSpec};

use super::super::{arg_f64, arg_i64, arg_str, verified};

// --- status --------------------------------------------------------------------

pub struct FillTool;

#[async_trait]
impl Tool for FillTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "fill",
            description: "Fill a field by CSS selector (input/textarea/select/checkbox/radio/contenteditable), firing input+change.",
            params: vec![
                ParamSpec::new("selector", ParamType::String, "CSS selector for the field").required(),
                ParamSpec::new("value", ParamType::String, "Value to fill").required(),
                ParamSpec::new("budget_s", ParamType::Number, "Seconds to wait for the field to reflect the change before reporting 'uncertain' (default 3)"),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let selector = arg_str(args, "selector")
            .ok_or_else(|| ToolError::Argument("fill: selector must be a string".into()))?;
        let value = arg_str(args, "value")
            .ok_or_else(|| ToolError::Argument("fill: value must be a string".into()))?;
        let tab = ctx.browser.tab().await?;
        verified(&tab, "fill", arg_f64(args, "budget_s", 3.0), || {
            ops::fill(&tab, selector, value)
        })
        .await
    }
}

// --- form_fill -----------------------------------------------------------------

// --- form_fill -----------------------------------------------------------------

pub struct FormFillTool;

#[async_trait]
impl Tool for FormFillTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "form_fill",
            description: "Fill multiple fields by fuzzy label/name/placeholder/aria match.",
            params: vec![
                ParamSpec::new(
                    "fields",
                    ParamType::Object,
                    "Dict of {label_or_placeholder: value} pairs",
                )
                .required(),
                ParamSpec::new(
                    "form_index",
                    ParamType::Integer,
                    "Which form to target (default 0)",
                ),
                ParamSpec::new("budget_s", ParamType::Number, "Seconds to wait for the form to reflect the changes before reporting 'uncertain' (default 4)"),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let fields = args
            .get("fields")
            .and_then(|v| v.as_object())
            .ok_or_else(|| ToolError::Argument("form_fill: fields must be an object".into()))?;
        let form_index = arg_i64(args, "form_index", 0);
        let tab = ctx.browser.tab().await?;
        verified(&tab, "form_fill", arg_f64(args, "budget_s", 4.0), || {
            ops::form_fill(&tab, fields, form_index)
        })
        .await
    }
}

// --- submit --------------------------------------------------------------------

// --- submit --------------------------------------------------------------------

pub struct SubmitTool;

#[async_trait]
impl Tool for SubmitTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "submit",
            description: "Submit a form: click the given selector or auto-detect a submit control, then wait for navigation.",
            params: vec![
                ParamSpec::new("selector", ParamType::String, "Optional CSS selector of the submit control"),
                ParamSpec::new("wait_s", ParamType::Number, "Max seconds to wait for navigation (default 5.0)"),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let selector = arg_str(args, "selector");
        let wait_s = arg_f64(args, "wait_s", 5.0);
        if !wait_s.is_finite() || wait_s < 0.0 {
            return Err(ToolError::Argument(
                "submit: wait_s must be a finite number >= 0".into(),
            ));
        }
        let wait_s = wait_s.min(ops::MAX_WAIT.as_secs_f64());
        let tab = ctx.browser.tab().await?;
        // `submit` already waits for navigation internally, so the verify budget only
        // needs to cover a same-page reaction (validation errors, a spinner).
        verified(&tab, "submit", wait_s.min(8.0), || {
            ops::submit(&tab, selector, wait_s)
        })
        .await
    }
}

// --- find_and_click ------------------------------------------------------------

// --- find_and_click ------------------------------------------------------------
