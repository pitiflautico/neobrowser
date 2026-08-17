//! The core loop: navigate, observe, act, verify, extract.
//!
//! Split out of a single 2700-line `tool_impls.rs`: at 67 tools that file had
//! stopped being navigable, and a reviewer could not tell which tools a change
//! touched.

use async_trait::async_trait;
use serde_json::{json, Map, Value};

use crate::ops;
use crate::page;
use crate::tools::{ParamSpec, ParamType, Tool, ToolCtx, ToolError, ToolOutput, ToolSpec};

use super::{arg_bool, arg_f64, arg_i64, arg_str, verified};

// --- status --------------------------------------------------------------------

pub struct StatusTool;

#[async_trait]
impl Tool for StatusTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "status",
            description:
                "Report browser status: Chrome binary discovery and whether a live session exists.",
            params: vec![],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        _args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let status = ctx.browser.status().await;
        Ok(ToolOutput::text(
            serde_json::to_string(&status).unwrap_or_else(|_| "{}".into()),
        ))
    }
}

// --- navigate ------------------------------------------------------------------

// --- navigate ------------------------------------------------------------------

pub struct NavigateTool;

#[async_trait]
impl Tool for NavigateTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "navigate",
            description: "Open URL in Chrome (tab reuse, self-healing). Required for SPAs, JS-heavy sites, and login-required pages.",
            params: vec![
                ParamSpec::new("url", ParamType::String, "HTTP/HTTPS URL to open").required(),
                ParamSpec::new("budget_s", ParamType::Number, "Total seconds allowed for the navigation (default 15). Returns status 'uncertain' if the page is not ready in time rather than waiting longer"),
                ParamSpec::new("wait_s", ParamType::Number, "Deprecated alias kept for compatibility: raises budget_s if larger"),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        use crate::action::{ActionStatus, Budget};

        let url = arg_str(args, "url")
            .ok_or_else(|| ToolError::Argument("navigate: url must be a string".into()))?;
        // `wait_s` used to mean "render buffer" while a separate hardcoded 15s
        // governed the load, so a caller had no way to bound the total. Now one
        // budget covers both; the old name is honoured as a floor so an existing
        // call asking for a long wait does not suddenly time out sooner.
        let budget_s = arg_f64(args, "budget_s", 15.0).max(arg_f64(args, "wait_s", 0.0));
        let budget = Budget::from_secs(budget_s);

        let tab = ctx.browser.tab().await?;
        let before = crate::action::observe(&tab).await;
        let complete = page::navigate_budgeted(&tab, url, &budget).await?;
        let after = crate::action::observe(&tab).await;
        let landed = if after.url.is_empty() {
            page::current_url(&tab)
                .await
                .unwrap_or_else(|_| url.to_string())
        } else {
            after.url.clone()
        };

        // A wall decides the status: reaching a captcha is not a successful
        // navigation to the requested page, and reporting it as one is exactly the
        // false success this contract exists to prevent.
        let wall = crate::walls::detect(&tab).await;
        let status = match (&wall, complete) {
            (Some(w), _) => w.action_status(),
            (None, true) => ActionStatus::Succeeded,
            (None, false) => ActionStatus::Uncertain,
        };

        let mut result = crate::action::ActionResult::new("navigate", status)
            .with_detail(format!("Navigated to {landed}"));
        result.before = before;
        result.after = after;
        result.changes = crate::action::detect_changes(&result.before, &result.after);
        if let Some(w) = wall {
            result = result
                .warn(format!("{}: {}", w.as_str(), w.hint()))
                .retryable(matches!(
                    w,
                    crate::walls::Wall::RateLimited | crate::walls::Wall::Error
                ));
        } else if !complete {
            result = result
                .warn(format!(
                    "budget_exhausted: the page was still loading after {budget_s}s; \
                     content may be incomplete. Re-read, or retry with a larger budget_s"
                ))
                .retryable(true);
        }
        Ok(ToolOutput::text(result.to_string_pretty()))
    }
}

// --- read ----------------------------------------------------------------------

// --- read ----------------------------------------------------------------------

pub struct ReadTool;

#[async_trait]
impl Tool for ReadTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "read",
            description: "Extract visible text from the current page via JavaScript.",
            params: vec![
                ParamSpec::new(
                    "selector",
                    ParamType::String,
                    "Optional CSS selector to read a specific element (default: body)",
                ),
                ParamSpec::new(
                    "raw",
                    ParamType::Boolean,
                    "Return the bare text without the untrusted-content fence (default false). Only for scripted extraction — a model should read the fenced form",
                ),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let selector = arg_str(args, "selector").unwrap_or("body");
        let raw = arg_bool(args, "raw", false);
        let tab = ctx.browser.tab().await?;
        let text = page::read_text(&tab, selector).await?;
        if text.is_empty() {
            return Ok(ToolOutput::text("(empty)"));
        }
        if raw {
            // Escape hatch for scripted extraction, where the caller is code rather
            // than a model and the fence is just noise to strip.
            return Ok(ToolOutput::text(text));
        }
        // Everything a page says is data written by whoever controls the page. Fence
        // and label it so it cannot be mistaken for an instruction from the user, and
        // say so when the page is actively trying.
        let origin = page::current_url(&tab).await.unwrap_or_default();
        Ok(ToolOutput::text(
            crate::untrusted::wrap(&origin, &text).to_string(),
        ))
    }
}

// --- screenshot ----------------------------------------------------------------

// --- screenshot ----------------------------------------------------------------

pub struct ScreenshotTool;

#[async_trait]
impl Tool for ScreenshotTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "screenshot",
            description: "Capture current page viewport as a base64 image (PNG default, or JPEG).",
            params: vec![
                ParamSpec::new(
                    "format",
                    ParamType::String,
                    "Image format: png (default) or jpeg",
                )
                .with_enum(&["png", "jpeg"]),
                ParamSpec::new(
                    "quality",
                    ParamType::Integer,
                    "JPEG quality 0-100 (default 80, ignored for PNG)",
                ),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let format = arg_str(args, "format").unwrap_or("png").to_string();
        let quality = arg_i64(args, "quality", 80);
        let tab = ctx.browser.tab().await?;
        let data = page::screenshot_base64(&tab, &format, quality).await?;
        let mime = if format == "jpeg" {
            "image/jpeg"
        } else {
            "image/png"
        };
        Ok(ToolOutput::Image {
            data,
            mime: mime.into(),
        })
    }
}

// --- find ----------------------------------------------------------------------

// --- find ----------------------------------------------------------------------

pub struct FindTool;

#[async_trait]
impl Tool for FindTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "find",
            description: "Find a UI element by natural-language intent (accessibility tree + heuristics). Returns a backendNodeId for use with click.",
            params: vec![ParamSpec::new(
                "intent",
                ParamType::String,
                "What to find, e.g. 'send button', 'message input box'",
            )
            .required()],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let intent = arg_str(args, "intent")
            .ok_or_else(|| ToolError::Argument("find: intent must be a string".into()))?;
        let tab = ctx.browser.tab().await?;
        match page::find(&tab, intent).await? {
            Some(n) => Ok(ToolOutput::text(
                json!({
                    "found": true,
                    "backend_node_id": n.backend_node_id,
                    "role": n.role,
                    "name": n.name,
                })
                .to_string(),
            )),
            None => Ok(ToolOutput::text(
                json!({ "found": false, "backend_node_id": null }).to_string(),
            )),
        }
    }
}

// --- revoke_session ------------------------------------------------------------

// --- observe -------------------------------------------------------------------

pub struct ObserveTool;

#[async_trait]
impl Tool for ObserveTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "observe",
            description: "Accessibility snapshot with STABLE element references you can pass to click/type. Prefer this over `find` for multi-step work: a reference like `button:Continue#0` survives a re-render, while a backendNodeId does not. Pass diff=true to get only what changed since the last observe, which is usually a few lines instead of the whole tree.",
            params: vec![
                ParamSpec::new("mode", ParamType::String, "interactive (default, actionable elements only) | visible (adds static text) | full (everything, for debugging)"),
                ParamSpec::new("diff", ParamType::Boolean, "Return only added/removed/changed elements since the previous observe (default false)"),
                ParamSpec::new("budget_chars", ParamType::Integer, "Maximum characters of listing to return (default 4000). Truncation is reported, never silent"),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        use crate::observe::{self, SnapshotMode};

        let mode = arg_str(args, "mode")
            .and_then(SnapshotMode::parse)
            .unwrap_or(SnapshotMode::Interactive);
        let want_diff = arg_bool(args, "diff", false);
        let budget_chars = arg_i64(args, "budget_chars", 4000).clamp(200, 200_000) as usize;

        let tab = ctx.browser.tab().await?;
        // Force a frame first: in headless the compositor is idle, so deferred
        // content would be missing from the tree we are about to read.
        page::nudge_frame(&tab).await;
        let snap = observe::snapshot(&tab, mode).await?;

        let previous = ctx.browser.take_snapshot().await;
        let mut out = snap.to_json(budget_chars);
        if want_diff {
            match &previous {
                Some(prev) => {
                    let d = observe::diff(prev, &snap);
                    out = json!({
                        "mode": mode.as_str(),
                        "url": snap.url,
                        "elements": snap.nodes.len(),
                        "diff": d.to_json(),
                    });
                }
                None => {
                    // Nothing to diff against yet. Returning the full snapshot with a
                    // note is more useful than an empty diff that reads as "no
                    // changes" when the truth is "no baseline".
                    out["warnings"] = json!([
                        "no_previous_snapshot: returned a full snapshot because there was \
                         nothing to diff against; the next observe(diff=true) will diff"
                    ]);
                }
            }
        }
        ctx.browser.store_snapshot(snap).await;
        Ok(ToolOutput::text(out.to_string()))
    }
}

// --- click ---------------------------------------------------------------------

// --- click ---------------------------------------------------------------------

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

        // Observe first: the whole point is to compare against this.
        let before = crate::action::observe(&tab).await;
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
            page::ClickOutcome::NotFound | page::ClickOutcome::Obscured { .. } => false,
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
        };

        if !dispatched {
            let after = crate::action::observe(&tab).await;
            let mut r = crate::action::ActionResult::new("click", ActionStatus::Failed)
                .with_detail(detail)
                .with_target(target_desc)
                // An overlay is removable, so retrying after dismissing it is
                // reasonable; a target that does not exist will not appear on retry.
                .retryable(matches!(outcome, page::ClickOutcome::Obscured { .. }));
            r.before = before;
            r.after = after;
            return Ok(ToolOutput::text(r.to_string_pretty()));
        }

        // Dispatched: now find out whether the page agreed.
        let (after, changed) = crate::action::wait_for_change(&tab, &before, &budget).await;
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
        r.changes = crate::action::detect_changes(&r.before, &r.after);
        if let Some(note) = resolve_warning {
            r.warnings.push(note);
        }
        if !changed {
            r = r.retryable(true).warn(
                "no_observable_change: the mouse events were delivered but nothing on the \
                 page changed. The element may be inert, or its handler may not be bound \
                 yet. Do NOT assume this click took effect",
            );
        }
        Ok(ToolOutput::text(r.to_string_pretty()))
    }
}

// --- type ----------------------------------------------------------------------

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

// --- js ------------------------------------------------------------------------

pub struct JsTool;

#[async_trait]
impl Tool for JsTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "js",
            description: "Execute JavaScript in the page. Use a return statement to get a value; `await` is supported.",
            params: vec![ParamSpec::new("code", ParamType::String, "JavaScript code to execute. Must use a return statement to return a value.").required()],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let code = arg_str(args, "code")
            .ok_or_else(|| ToolError::Argument("js: code must be a string".into()))?;
        let tab = ctx.browser.tab().await?;
        Ok(ToolOutput::text(ops::eval_js(&tab, code).await?))
    }
}

// --- page_info -----------------------------------------------------------------

// --- page_info -----------------------------------------------------------------

pub struct PageInfoTool;

#[async_trait]
impl Tool for PageInfoTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec { name: "page_info", description: "Summarize the page: url, title, interactive-element count, forms, and overlay presence.", params: vec![] }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        _args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let tab = ctx.browser.tab().await?;
        Ok(ToolOutput::text(ops::page_info(&tab).await?))
    }
}

// --- analyze -------------------------------------------------------------------

// --- analyze -------------------------------------------------------------------

pub struct AnalyzeTool;

#[async_trait]
impl Tool for AnalyzeTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec { name: "analyze", description: "Structured page analysis: forms with fields+labels, buttons, overlays, and the active element.", params: vec![] }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        _args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let tab = ctx.browser.tab().await?;
        Ok(ToolOutput::text(ops::analyze(&tab).await?))
    }
}

// --- fill ----------------------------------------------------------------------

// --- fill ----------------------------------------------------------------------

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

pub struct FindAndClickTool;

#[async_trait]
impl Tool for FindAndClickTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "find_and_click",
            description: "Click the nth VISIBLE clickable element whose text or aria-label contains the given text. Hidden and collapsed matches (closed accordion steps, header panels duplicating a body form) are skipped and counted in matched_total vs matched_visible, so a multi-step form can't silently submit the wrong step.",
            params: vec![
                ParamSpec::new("text", ParamType::String, "Visible text or label to search for").required(),
                ParamSpec::new("role", ParamType::String, "Optional ARIA role to narrow the search"),
                ParamSpec::new("nth", ParamType::Integer, "Which match to click (0-based, default 0)"),
                ParamSpec::new("budget_s", ParamType::Number, "Seconds to wait for the page to react before reporting 'uncertain' (default 5)"),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let text = arg_str(args, "text")
            .ok_or_else(|| ToolError::Argument("find_and_click: text must be a string".into()))?;
        let role = arg_str(args, "role").unwrap_or("");
        let nth = arg_i64(args, "nth", 0);
        let tab = ctx.browser.tab().await?;
        verified(
            &tab,
            "find_and_click",
            arg_f64(args, "budget_s", 5.0),
            || ops::find_and_click(&tab, text, role, nth),
        )
        .await
    }
}

// --- dismiss_overlay -----------------------------------------------------------

// --- dismiss_overlay -----------------------------------------------------------

pub struct DismissOverlayTool;

#[async_trait]
impl Tool for DismissOverlayTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "dismiss_overlay",
            description: "Detect and dismiss cookie/GDPR/newsletter overlays by clicking accept/close inside them. force=true also tries Escape + backdrop.",
            params: vec![ParamSpec::new("force", ParamType::Boolean, "Also try Escape + backdrop click (default false)")],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let force = arg_bool(args, "force", false);
        let tab = ctx.browser.tab().await?;
        Ok(ToolOutput::text(ops::dismiss_overlay(&tab, force).await?))
    }
}

// --- extract -------------------------------------------------------------------

// --- wait ----------------------------------------------------------------------

pub struct WaitTool;

#[async_trait]
impl Tool for WaitTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "wait",
            description:
                "Wait for a fixed time, or (if selector given) poll until it appears, up to ms.",
            params: vec![
                ParamSpec::new(
                    "ms",
                    ParamType::Integer,
                    "Milliseconds to wait / poll timeout (default 1000)",
                ),
                ParamSpec::new(
                    "selector",
                    ParamType::String,
                    "Optional CSS selector to wait for",
                ),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let ms = arg_i64(args, "ms", 1000);
        if ms < 0 {
            return Err(ToolError::Argument("wait: ms must be >= 0".into()));
        }
        let ms = ms.min(ops::MAX_WAIT.as_millis() as i64);
        let selector = arg_str(args, "selector");
        let tab = ctx.browser.tab().await?;
        Ok(ToolOutput::text(ops::wait(&tab, ms, selector).await?))
    }
}

// --- paginate ------------------------------------------------------------------

// --- scroll --------------------------------------------------------------------

pub struct ScrollTool;

#[async_trait]
impl Tool for ScrollTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "scroll",
            description: "Scroll the viewport (up/down/top/bottom), then force a frame so load-on-scroll content paints.",
            params: vec![
                ParamSpec::new("direction", ParamType::String, "up, down (default), top, or bottom").with_enum(&["up", "down", "top", "bottom"]),
                ParamSpec::new("amount", ParamType::Integer, "Pixels to scroll for up/down (default 500)"),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let direction = arg_str(args, "direction").unwrap_or("down");
        let amount = arg_i64(args, "amount", 500);
        let tab = ctx.browser.tab().await?;
        Ok(ToolOutput::text(
            ops::scroll(&tab, direction, amount).await?,
        ))
    }
}

// --- wait ----------------------------------------------------------------------

// --- paginate ------------------------------------------------------------------

pub struct PaginateTool;

#[async_trait]
impl Tool for PaginateTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "paginate",
            description: "Advance to the next page: click a given selector or auto-detect a next/more control, then force a frame.",
            params: vec![ParamSpec::new("selector", ParamType::String, "Optional CSS selector of the next control")],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let selector = arg_str(args, "selector");
        let tab = ctx.browser.tab().await?;
        Ok(ToolOutput::text(ops::paginate(&tab, selector).await?))
    }
}

// --- console_logs --------------------------------------------------------------

// --- extract -------------------------------------------------------------------

pub struct ExtractTool;

#[async_trait]
impl Tool for ExtractTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "extract",
            description: "Extract structured data: 'links' (default) returns anchors as text+href; 'tables' returns table outerHTML.",
            params: vec![ParamSpec::new("what", ParamType::String, "What to extract: links (default) or tables").with_enum(&["links", "tables"])],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let what = arg_str(args, "what").unwrap_or("links");
        let tab = ctx.browser.tab().await?;
        Ok(ToolOutput::text(ops::extract(&tab, what).await?))
    }
}

// --- extract_table -------------------------------------------------------------

// --- extract_table -------------------------------------------------------------

pub struct ExtractTableTool;

#[async_trait]
impl Tool for ExtractTableTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "extract_table",
            description: "Parse a table into an array of {header: cell} row objects.",
            params: vec![
                ParamSpec::new(
                    "selector",
                    ParamType::String,
                    "CSS selector for the table(s) (default: table)",
                ),
                ParamSpec::new(
                    "index",
                    ParamType::Integer,
                    "Which matched table to parse (default 0)",
                ),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let selector = arg_str(args, "selector").unwrap_or("table");
        let index = arg_i64(args, "index", 0);
        let tab = ctx.browser.tab().await?;
        Ok(ToolOutput::text(
            ops::extract_table(&tab, selector, index).await?,
        ))
    }
}

// --- scroll --------------------------------------------------------------------
