//! The two tools that change the page: click and type.
//!
//! Both report what actually happened rather than that a message was dispatched. A click
//! that cannot be verified comes back as unverified, never as success — this is the whole
//! point of the verified-action contract, and these are the tools where it is visible.

use async_trait::async_trait;
use serde_json::{Map, Value};

use crate::page;
use crate::tools::{ParamSpec, ParamType, Tool, ToolCtx, ToolError, ToolOutput, ToolSpec};

use super::super::{arg_bool, arg_f64, arg_str, verified};

pub struct ClickTool;

#[async_trait]
impl Tool for ClickTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "click",
            description: "Click an element by backendNodeId (from find) or CSS selector. Uses real (isTrusted) mouse events. Scrolls the target into view first, and refuses to click when another element (modal, cookie banner, sticky header) covers it — reporting which one, so you can dismiss it and retry rather than assuming the click landed.",
            params: vec![
                ParamSpec::new("ref", ParamType::String, "Stable reference from `observe` (e.g. `button:Continue#0`). PREFERRED: it is re-resolved against the live tree on every use, so it still works after a re-render"),
                ParamSpec::new("backend_node_id", ParamType::Integer, "backendNodeId from a find result. Valid only until the node is recreated"),
                ParamSpec::new("selector", ParamType::String, "CSS selector fallback"),
                ParamSpec::new("budget_s", ParamType::Number, "Seconds to wait for the page to react before reporting 'uncertain' (default 5)"),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        use crate::action::{ActionStatus, Budget, TargetDesc};

        let tab = ctx.browser.tab().await?;
        let stable_ref = arg_str(args, "ref");
        let target_desc = if let Some(r) = stable_ref {
            let (role, name, _) = crate::observe::StableRef::decode(r)
                .unwrap_or_else(|| (String::new(), String::new(), 0));
            TargetDesc::new(r, role, name)
        } else if let Some(id) = args.get("backend_node_id").and_then(|v| v.as_i64()) {
            TargetDesc::new(id.to_string(), "", "")
        } else if let Some(sel) = arg_str(args, "selector") {
            TargetDesc::new(sel, "", "")
        } else {
            return Err(ToolError::Argument(
                "click: provide ref (preferred), backend_node_id, or selector".into(),
            ));
        };
        let budget = Budget::from_secs(arg_f64(args, "budget_s", 5.0));

        // A stable reference is resolved HERE, against the current tree, rather than
        // trusting an id captured earlier. That re-resolution is the whole point:
        // between `observe` and this call the page may have rebuilt the node.
        let mut resolve_warning = None;
        let resolved_id = match stable_ref {
            Some(r) => match crate::observe::resolve(&tab, r).await? {
                Some(id) => Some(id),
                None => {
                    let mut res = crate::action::ActionResult::new("click", ActionStatus::Failed)
                        .with_detail(format!(
                            "no element currently matches the reference `{r}`. Re-run \
                             `observe` to get current references"
                        ))
                        .with_target(target_desc)
                        .retryable(false);
                    res.before = crate::action::observe(&tab).await;
                    res.after = res.before.clone();
                    return Ok(ToolOutput::text(res.to_string_pretty()));
                }
            },
            None => None,
        };
        if let (Some(r), Some(_)) = (stable_ref, resolved_id) {
            resolve_warning = Some(format!("resolved `{r}` against the live tree"));
        }

        // Two things have to be true before this click can be judged, and both were missing.
        //
        // First the baseline must be a state the page has settled into. Taken too early, the
        // page's own finishing — images arriving, a route transition completing — lands in the
        // "after" observation and gets credited to the click. On a real site an `add to cart`
        // that never landed reported `succeeded` for exactly that reason: the page was still
        // settling from the login navigation before it.
        //
        // Second, a page that never settles still has to be actionable, so whatever keeps
        // moving on its own is measured and excluded from the evidence rather than counted.
        let settle_budget = crate::action::Budget::from_secs(2.0);
        let (before, quiet) = crate::action::quiesce(&tab, &settle_budget).await;
        let noise = if quiet {
            Vec::new()
        } else {
            crate::action::ambient_noise(&tab, std::time::Duration::from_millis(350)).await
        };
        let outcome = if let Some(id) = resolved_id {
            page::click_backend_node(&tab, id).await?
        } else if let Some(id) = args.get("backend_node_id").and_then(|v| v.as_i64()) {
            page::click_backend_node(&tab, id).await?
        } else {
            page::click_selector(&tab, arg_str(args, "selector").unwrap_or_default()).await?
        };

        // A click that never left the gate is decided by the outcome alone — there is
        // no point waiting for a page reaction to an event we did not dispatch.
        let dispatched = match &outcome {
            page::ClickOutcome::Clicked | page::ClickOutcome::NoLayoutUsedJs => true,
            page::ClickOutcome::NotFound
            | page::ClickOutcome::Obscured { .. }
            | page::ClickOutcome::Disabled { .. } => false,
        };
        let detail = match &outcome {
            page::ClickOutcome::Clicked => "click dispatched as real mouse events".to_string(),
            page::ClickOutcome::NoLayoutUsedJs => {
                "click dispatched via JS fallback (element had no box model)".to_string()
            }
            page::ClickOutcome::NotFound => "click target not found".to_string(),
            page::ClickOutcome::Obscured { by } => format!(
                "not clicked: target is covered by {by}. Dismiss the overlay \
                 (dismiss_overlay) or scroll it out of the way, then retry"
            ),
            page::ClickOutcome::Disabled { reason } => format!(
                "not clicked: {reason}. Change what keeps it disabled — a required field, a \
                 pending validation — rather than retrying the click"
            ),
        };
        // An obstruction is not a failure, and the difference is the caller's next move.
        //
        // `failed` means "this did not happen and will not on retry"; `blocked` means "clear
        // the thing in the way and try again". The distinction was already encoded here — in
        // `retryable`, and in the detail text — but the *status* lumped an overlay in with a
        // target that does not exist. A caller switching on status therefore read a removable
        // cookie banner as a dead end. Conformance scenario C2 is what caught it.
        let status = match &outcome {
            page::ClickOutcome::Obscured { .. } | page::ClickOutcome::Disabled { .. } => {
                ActionStatus::Blocked
            }
            _ => ActionStatus::Failed,
        };

        if !dispatched {
            let after = crate::action::observe(&tab).await;
            let mut r = crate::action::ActionResult::new("click", status)
                .with_detail(detail)
                .with_target(target_desc)
                // An overlay is removable, so retrying after dismissing it is reasonable; a
                // target that does not exist will not appear on retry. A disabled control is
                // blocked but NOT retryable — the same click will keep being refused until
                // something else changes, and saying otherwise invites an infinite loop.
                .retryable(matches!(outcome, page::ClickOutcome::Obscured { .. }));
            r.before = before;
            r.after = after;
            return Ok(ToolOutput::text(r.to_string_pretty()));
        }

        // Dispatched: now find out whether the page agreed.
        let (after, changed) =
            crate::action::wait_for_change_discounting(&tab, &before, &budget, &noise).await;
        let status = if changed {
            ActionStatus::Succeeded
        } else {
            ActionStatus::Uncertain
        };
        let mut r = crate::action::ActionResult::new("click", status)
            .with_detail(detail)
            .with_target(target_desc);
        r.before = before;
        r.after = after;
        r.changes = crate::action::detect_changes_discounting(&r.before, &r.after, &noise);
        if !quiet {
            r = r.warn(format!(
                "ambient_change: this page never stopped changing on its own, so {} could not \
                 serve as evidence and was excluded. A `succeeded` here rests on the remaining \
                 components only",
                if noise.is_empty() {
                    "nothing".to_string()
                } else {
                    noise.join(", ")
                }
            ));
        }
        if let Some(note) = resolve_warning {
            r.warnings.push(note);
        }
        if !changed {
            // Before blaming the element, check whether the tab is still receiving input at
            // all. Chrome 151 can reach a state where every `Input.*` command is accepted and
            // silently dropped — see `page::input_is_alive`. From the outside that is
            // indistinguishable from a click on an inert button, and the two need opposite
            // responses: retry the one, replace the tab for the other. Telling a caller to
            // "try a different target" when no target can ever work is the kind of advice
            // that turns one failure into a loop.
            //
            // The probe costs three round trips, so it runs only here, where the answer
            // changes what the caller should do.
            if page::input_is_alive(&tab).await {
                r = r.retryable(true).warn(
                    "no_observable_change: the mouse events were delivered but nothing on the \
                     page changed. The element may be inert, or its handler may not be bound \
                     yet. Do NOT assume this click took effect",
                );
            } else {
                match ctx.browser.replace_active_tab().await {
                    Ok((_, url)) => {
                        r = r.retryable(true).warn(format!(
                            "input_pipeline_stalled: this tab had stopped delivering mouse and \
                             keyboard events — a Chrome-level fault, not a property of the \
                             element. It has been replaced with a fresh tab reloaded at {url}. \
                             Cookies and storage survived, but anything the old document held \
                             only in memory (unsaved input, scroll position) did not. \
                             Re-observe, then retry"
                        ));
                    }
                    Err(e) => {
                        r = r.retryable(false).warn(format!(
                            "input_pipeline_stalled: this tab had stopped delivering mouse and \
                             keyboard events, and replacing it failed ({e}). No click can \
                             succeed here until the browser is restarted"
                        ));
                    }
                }
            }
        }
        Ok(ToolOutput::text(r.to_string_pretty()))
    }
}

// --- type ----------------------------------------------------------------------

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

// --- js ------------------------------------------------------------------------
