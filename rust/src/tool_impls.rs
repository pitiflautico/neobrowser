//! Concrete tool implementations and the registry builder.
//!
//! Phase 2 shipped `status`. Phase 3 adds the core browser verbs:
//! navigate, read, screenshot, find, click, type. Phases 5–6 add the rest.

use async_trait::async_trait;
use serde_json::{json, Map, Value};
use std::sync::Arc;

use crate::ops;
use crate::page;
use crate::reach;
use crate::search;
use crate::sessions;
use crate::tools::{
    ParamSpec, ParamType, Registry, Tool, ToolCtx, ToolError, ToolOutput, ToolSpec,
};

// --- small typed arg accessors -------------------------------------------------

fn arg_str<'a>(args: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| v.as_str())
}
fn arg_f64(args: &Map<String, Value>, key: &str, default: f64) -> f64 {
    args.get(key).and_then(|v| v.as_f64()).unwrap_or(default)
}
fn arg_i64(args: &Map<String, Value>, key: &str, default: i64) -> i64 {
    args.get(key).and_then(|v| v.as_i64()).unwrap_or(default)
}
fn arg_bool(args: &Map<String, Value>, key: &str, default: bool) -> bool {
    args.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
}

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

pub struct NavigateTool;

#[async_trait]
impl Tool for NavigateTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "navigate",
            description: "Open URL in Chrome (tab reuse, self-healing). Required for SPAs, JS-heavy sites, and login-required pages.",
            params: vec![
                ParamSpec::new("url", ParamType::String, "HTTP/HTTPS URL to open").required(),
                ParamSpec::new("wait_s", ParamType::Number, "Seconds to wait for page render (default 3.0)"),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let url = arg_str(args, "url")
            .ok_or_else(|| ToolError::Argument("navigate: url must be a string".into()))?;
        let wait_s = arg_f64(args, "wait_s", 3.0);
        let tab = ctx.browser.tab().await?;
        page::navigate(&tab, url, wait_s).await?;
        let landed = page::current_url(&tab)
            .await
            .unwrap_or_else(|_| url.to_string());
        let mut msg = format!("Navigated to {landed}");
        // Surface anti-bot friction generically so the model can react (retry with a
        // real profile, dismiss a consent gate, pick another source).
        if let Some(wall) = crate::walls::detect(&tab).await {
            msg.push_str(&format!("\n⚠️ {}: {}", wall.as_str(), wall.hint()));
        }
        Ok(ToolOutput::text(msg))
    }
}

// --- read ----------------------------------------------------------------------

pub struct ReadTool;

#[async_trait]
impl Tool for ReadTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "read",
            description: "Extract visible text from the current page via JavaScript.",
            params: vec![ParamSpec::new(
                "selector",
                ParamType::String,
                "Optional CSS selector to read a specific element (default: body)",
            )],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let selector = arg_str(args, "selector").unwrap_or("body");
        let tab = ctx.browser.tab().await?;
        let text = page::read_text(&tab, selector).await?;
        Ok(ToolOutput::text(if text.is_empty() {
            "(empty)".into()
        } else {
            text
        }))
    }
}

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

// --- click ---------------------------------------------------------------------

pub struct ClickTool;

#[async_trait]
impl Tool for ClickTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "click",
            description: "Click an element by backendNodeId (from find) or CSS selector. Uses real (isTrusted) mouse events. Scrolls the target into view first, and refuses to click when another element (modal, cookie banner, sticky header) covers it — reporting which one, so you can dismiss it and retry rather than assuming the click landed.",
            params: vec![
                ParamSpec::new("backend_node_id", ParamType::Integer, "backendNodeId from a find result"),
                ParamSpec::new("selector", ParamType::String, "CSS selector fallback"),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let tab = ctx.browser.tab().await?;
        let outcome = if let Some(id) = args.get("backend_node_id").and_then(|v| v.as_i64()) {
            page::click_backend_node(&tab, id).await?
        } else if let Some(sel) = arg_str(args, "selector") {
            page::click_selector(&tab, sel).await?
        } else {
            return Err(ToolError::Argument(
                "click: provide either backend_node_id or selector".into(),
            ));
        };
        // Say what actually happened. "Clicked" for a click that never landed
        // is worse than an error: the agent builds on it and fails much later,
        // far from the cause.
        Ok(ToolOutput::text(match outcome {
            page::ClickOutcome::Clicked => "Clicked".to_string(),
            page::ClickOutcome::NoLayoutUsedJs => {
                "Clicked via JS fallback (element had no box model)".to_string()
            }
            page::ClickOutcome::NotFound => "Click target not found".to_string(),
            page::ClickOutcome::Obscured { by } => format!(
                "Not clicked: target is covered by {by}. \
                 Dismiss the overlay (dismiss_overlay) or scroll it out of the way, then retry."
            ),
        }))
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
        page::type_text(&tab, text, human).await?;
        Ok(ToolOutput::text(format!(
            "Typed {} chars",
            text.chars().count()
        )))
    }
}

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
        Ok(ToolOutput::text(ops::fill(&tab, selector, value).await?))
    }
}

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
        Ok(ToolOutput::text(
            ops::form_fill(&tab, fields, form_index).await?,
        ))
    }
}

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
        Ok(ToolOutput::text(ops::submit(&tab, selector, wait_s).await?))
    }
}

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
        Ok(ToolOutput::text(
            ops::find_and_click(&tab, text, role, nth).await?,
        ))
    }
}

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

pub struct ConsoleLogsTool;

#[async_trait]
impl Tool for ConsoleLogsTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "console_logs",
            description: "Get captured browser console entries (log/info/warning/error/exception).",
            params: vec![
                ParamSpec::new(
                    "level",
                    ParamType::String,
                    "Filter by level: log, info, warning, error (default: all)",
                ),
                ParamSpec::new(
                    "limit",
                    ParamType::Integer,
                    "Max entries to return (default 50)",
                ),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let level = arg_str(args, "level");
        let limit = arg_i64(args, "limit", 50).max(0) as usize;
        Ok(ToolOutput::text(
            ctx.browser.console_logs(level, limit).await?,
        ))
    }
}

// --- network_log ---------------------------------------------------------------

pub struct NetworkLogTool;

#[async_trait]
impl Tool for NetworkLogTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "network_log",
            description: "Get captured network requests with status, duration, and size.",
            params: vec![
                ParamSpec::new(
                    "url_pattern",
                    ParamType::String,
                    "Filter by URL substring (default: all)",
                ),
                ParamSpec::new("limit", ParamType::Integer, "Max entries (default 50)"),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let pattern = arg_str(args, "url_pattern");
        let limit = arg_i64(args, "limit", 50).max(0) as usize;
        Ok(ToolOutput::text(
            ctx.browser.network_log(pattern, limit).await?,
        ))
    }
}

// --- metrics -------------------------------------------------------------------

pub struct MetricsTool;

#[async_trait]
impl Tool for MetricsTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "metrics",
            description: "Get Chrome performance metrics: JSHeapUsedSize, Nodes, Documents, etc.",
            params: vec![ParamSpec::new(
                "key",
                ParamType::String,
                "Return only this metric (default: all)",
            )],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let key = arg_str(args, "key");
        Ok(ToolOutput::text(ctx.browser.metrics(key).await?))
    }
}

// --- debug ---------------------------------------------------------------------

pub struct DebugTool;

#[async_trait]
impl Tool for DebugTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "debug",
            description: "Control an in-page console interceptor: start (install), flush (default, drain captured logs), stop (remove).",
            params: vec![ParamSpec::new("action", ParamType::String, "start, flush (default), or stop").with_enum(&["start", "flush", "stop"])],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let action = arg_str(args, "action").unwrap_or("flush");
        let tab = ctx.browser.tab().await?;
        Ok(ToolOutput::text(ops::debug(&tab, action).await?))
    }
}

// --- save_cookies / restore_cookies -------------------------------------------

pub struct SaveCookiesTool;

#[async_trait]
impl Tool for SaveCookiesTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec { name: "save_cookies", description: "Save the current session's cookies to ~/.neobrowser/cookies/{profile}.json (0600 perms).", params: vec![] }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        _args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let tab = ctx.browser.tab().await?;
        let n = sessions::save_cookies(&tab).await?;
        Ok(ToolOutput::text(format!("Saved {n} cookies")))
    }
}

pub struct RestoreCookiesTool;

#[async_trait]
impl Tool for RestoreCookiesTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "restore_cookies",
            description:
                "Inject saved cookies from disk into the current tab. Returns count restored.",
            params: vec![],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        _args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let tab = ctx.browser.tab().await?;
        let n = sessions::restore_cookies(&tab).await?;
        Ok(ToolOutput::text(format!("Restored {n} cookies")))
    }
}

// --- save_session / session_info ----------------------------------------------

pub struct SaveSessionTool;

#[async_trait]
impl Tool for SaveSessionTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec { name: "save_session", description: "Full session save: cookies + localStorage → ~/.neobrowser/sessions/. Persists authenticated state across restarts.", params: vec![] }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        _args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let tab = ctx.browser.tab().await?;
        Ok(ToolOutput::text(sessions::save_session(&tab).await?))
    }
}

pub struct SessionInfoTool;

#[async_trait]
impl Tool for SessionInfoTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "session_info",
            description:
                "Show session persistence state: last save time, cookie count, domains, file paths.",
            params: vec![],
        }
    }
    async fn call(
        &self,
        _ctx: &ToolCtx,
        _args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput::text(sessions::session_info()))
    }
}

// --- login ---------------------------------------------------------------------

pub struct LoginTool;

#[async_trait]
impl Tool for LoginTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "login",
            description: "Navigate an https login page, fill email + password, submit, and report honest success (a lingering password field means it failed).",
            params: vec![
                ParamSpec::new("url", ParamType::String, "Login page URL (must be https)").required(),
                ParamSpec::new("email", ParamType::String, "Email or username").required(),
                ParamSpec::new("password", ParamType::String, "Password").required(),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let url = arg_str(args, "url")
            .ok_or_else(|| ToolError::Argument("login: url must be a string".into()))?;
        let email = arg_str(args, "email")
            .ok_or_else(|| ToolError::Argument("login: email must be a string".into()))?;
        let password = arg_str(args, "password")
            .ok_or_else(|| ToolError::Argument("login: password must be a string".into()))?;
        let tab = ctx.browser.tab().await?;
        Ok(ToolOutput::text(
            sessions::login(&tab, url, email, password).await?,
        ))
    }
}

// --- browse --------------------------------------------------------------------

pub struct BrowseTool;

#[async_trait]
impl Tool for BrowseTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "browse",
            description: "Server-side fetch of a public URL (SSRF-guarded). JSON passes through; HTML is reduced to text. Does not use the browser tab.",
            params: vec![
                ParamSpec::new("url", ParamType::String, "Public http/https URL to fetch").required(),
                ParamSpec::new("headers", ParamType::Object, "Optional extra request headers"),
            ],
        }
    }
    async fn call(
        &self,
        _ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let url = arg_str(args, "url")
            .ok_or_else(|| ToolError::Argument("browse: url must be a string".into()))?;
        let empty = Map::new();
        let headers = args
            .get("headers")
            .and_then(|v| v.as_object())
            .unwrap_or(&empty);
        Ok(ToolOutput::text(reach::browse(url, headers).await))
    }
}

// --- upload --------------------------------------------------------------------

pub struct UploadTool;

#[async_trait]
impl Tool for UploadTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "upload",
            description:
                "Attach local file(s) to a file input by CSS selector. Files must live under an allowed directory (~/Downloads, ~/Desktop, ~/Documents, ~/.neobrowser/downloads, or NEOBROWSER_UPLOAD_DIR) and never a sensitive path (ssh/aws keys, .env, keychains, credentials).",
            params: vec![
                ParamSpec::new(
                    "selector",
                    ParamType::String,
                    "CSS selector of the file input",
                )
                .required(),
                ParamSpec::new("files", ParamType::Array, "Absolute file path(s) to attach")
                    .required(),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let selector = arg_str(args, "selector")
            .ok_or_else(|| ToolError::Argument("upload: selector must be a string".into()))?;
        let files: Vec<String> = match args.get("files") {
            Some(Value::String(s)) => vec![s.clone()],
            Some(Value::Array(a)) => a
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect(),
            _ => {
                return Err(ToolError::Argument(
                    "upload: files must be a string or array of strings".into(),
                ))
            }
        };
        let tab = ctx.browser.tab().await?;
        Ok(ToolOutput::text(
            reach::upload(&tab, selector, files).await?,
        ))
    }
}

// --- download ------------------------------------------------------------------

pub struct DownloadTool;

#[async_trait]
impl Tool for DownloadTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "download",
            description: "Download a public URL to ~/.neobrowser/downloads/, reusing the tab's cookies for auth-gated files (SSRF-guarded, 200MB cap).",
            params: vec![
                ParamSpec::new("url", ParamType::String, "Direct file URL to download").required(),
                ParamSpec::new("filename", ParamType::String, "Optional output filename"),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let url = arg_str(args, "url")
            .ok_or_else(|| ToolError::Argument("download: url must be a string".into()))?;
        let filename = arg_str(args, "filename");
        let tab = ctx.browser.tab().await?;
        Ok(ToolOutput::text(
            reach::download(&tab, url, filename).await?,
        ))
    }
}

// --- search --------------------------------------------------------------------

pub struct SearchTool;

#[async_trait]
impl Tool for SearchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "search",
            description: "Web text search through the real browser: Google first (uses your logged-in profile if set), DuckDuckGo fallback.",
            params: vec![
                ParamSpec::new("query", ParamType::String, "Search query").required(),
                ParamSpec::new("limit", ParamType::Integer, "Max results (default 10)"),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let query = arg_str(args, "query")
            .ok_or_else(|| ToolError::Argument("search: query must be a string".into()))?;
        let limit = arg_i64(args, "limit", 10).max(0) as usize;
        let tab = ctx.browser.tab().await?;
        Ok(ToolOutput::text(search::search(&tab, query, limit).await))
    }
}

// --- search_images -------------------------------------------------------------

pub struct SearchImagesTool;

#[async_trait]
impl Tool for SearchImagesTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "search_images",
            description: "Google Images search: returns image_url, source, title, and a curl download command.",
            params: vec![
                ParamSpec::new("query", ParamType::String, "Image search query").required(),
                ParamSpec::new("count", ParamType::Integer, "Max results (default 10, max 30)"),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let query = arg_str(args, "query")
            .ok_or_else(|| ToolError::Argument("search_images: query must be a string".into()))?;
        let count = arg_i64(args, "count", 10).max(0) as usize;
        let tab = ctx.browser.tab().await?;
        Ok(ToolOutput::text(
            search::search_images(&tab, query, count).await,
        ))
    }
}

// --- search_videos -------------------------------------------------------------

pub struct SearchVideosTool;

#[async_trait]
impl Tool for SearchVideosTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "search_videos",
            description: "Google Videos search: returns url, channel, duration, platform, and a yt-dlp download command.",
            params: vec![
                ParamSpec::new("query", ParamType::String, "Video search query").required(),
                ParamSpec::new("count", ParamType::Integer, "Max results (default 10, max 30)"),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let query = arg_str(args, "query")
            .ok_or_else(|| ToolError::Argument("search_videos: query must be a string".into()))?;
        let count = arg_i64(args, "count", 10).max(0) as usize;
        let tab = ctx.browser.tab().await?;
        Ok(ToolOutput::text(
            search::search_videos(&tab, query, count).await,
        ))
    }
}

// --- search_twitter_videos -----------------------------------------------------

pub struct SearchTwitterVideosTool;

#[async_trait]
impl Tool for SearchTwitterVideosTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "search_twitter_videos",
            description:
                "Search for videos on X/Twitter (video search scoped to x.com/twitter.com).",
            params: vec![
                ParamSpec::new("query", ParamType::String, "Twitter video search query").required(),
                ParamSpec::new(
                    "count",
                    ParamType::Integer,
                    "Max results (default 10, max 30)",
                ),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let query = arg_str(args, "query").ok_or_else(|| {
            ToolError::Argument("search_twitter_videos: query must be a string".into())
        })?;
        let count = arg_i64(args, "count", 10).max(0) as usize;
        let tab = ctx.browser.tab().await?;
        Ok(ToolOutput::text(
            search::search_twitter_videos(&tab, query, count).await,
        ))
    }
}

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

pub struct NewTabTool;

#[async_trait]
impl Tool for NewTabTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec { name: "new_tab", description: "Open a new browser tab on the shared Chrome and make it active. Returns its index/id.", params: vec![] }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        _args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput::text(ctx.browser.new_tab().await?))
    }
}

pub struct ListTabsTool;

#[async_trait]
impl Tool for ListTabsTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "list_tabs",
            description: "List open tabs with index, url, title, and which is active.",
            params: vec![],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        _args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput::text(ctx.browser.list_tabs().await?))
    }
}

pub struct SwitchTabTool;

#[async_trait]
impl Tool for SwitchTabTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "switch_tab",
            description: "Make the tab at the given index active (subsequent tools act on it).",
            params: vec![
                ParamSpec::new("index", ParamType::Integer, "Tab index from list_tabs").required(),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let index = args
            .get("index")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| ToolError::Argument("switch_tab: index must be an integer".into()))?;
        if index < 0 {
            return Err(ToolError::Argument("switch_tab: index must be >= 0".into()));
        }
        Ok(ToolOutput::text(
            ctx.browser.switch_tab(index as usize).await?,
        ))
    }
}

pub struct CloseTabTool;

#[async_trait]
impl Tool for CloseTabTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "close_tab",
            description: "Close the tab at the given index (cannot close the last remaining tab).",
            params: vec![
                ParamSpec::new("index", ParamType::Integer, "Tab index from list_tabs").required(),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let index = args
            .get("index")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| ToolError::Argument("close_tab: index must be an integer".into()))?;
        if index < 0 {
            return Err(ToolError::Argument("close_tab: index must be >= 0".into()));
        }
        Ok(ToolOutput::text(
            ctx.browser.close_tab(index as usize).await?,
        ))
    }
}

/// Build the tool registry for the current phase.
pub fn build_registry() -> Registry {
    let mut r = Registry::new();
    for t in tool_list() {
        r.register(t);
    }
    r
}

/// All registered tools. Kept as a list so tests can assert coverage/parity.
pub fn tool_list() -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(StatusTool),
        Arc::new(NavigateTool),
        Arc::new(ReadTool),
        Arc::new(ScreenshotTool),
        Arc::new(FindTool),
        Arc::new(ClickTool),
        Arc::new(TypeTool),
        Arc::new(JsTool),
        Arc::new(PageInfoTool),
        Arc::new(AnalyzeTool),
        Arc::new(FillTool),
        Arc::new(FormFillTool),
        Arc::new(SubmitTool),
        Arc::new(FindAndClickTool),
        Arc::new(DismissOverlayTool),
        Arc::new(ExtractTool),
        Arc::new(ExtractTableTool),
        Arc::new(ScrollTool),
        Arc::new(WaitTool),
        Arc::new(PaginateTool),
        Arc::new(ConsoleLogsTool),
        Arc::new(NetworkLogTool),
        Arc::new(MetricsTool),
        Arc::new(DebugTool),
        Arc::new(SaveCookiesTool),
        Arc::new(RestoreCookiesTool),
        Arc::new(SaveSessionTool),
        Arc::new(SessionInfoTool),
        Arc::new(LoginTool),
        Arc::new(BrowseTool),
        Arc::new(UploadTool),
        Arc::new(DownloadTool),
        Arc::new(SearchTool),
        Arc::new(SearchImagesTool),
        Arc::new(SearchVideosTool),
        Arc::new(SearchTwitterVideosTool),
        Arc::new(RecordTaskTool),
        Arc::new(StopRecordingTool),
        Arc::new(ReplayTool),
        Arc::new(NewTabTool),
        Arc::new(ListTabsTool),
        Arc::new(SwitchTabTool),
        Arc::new(CloseTabTool),
    ]
}

/// Regression guard: the registry must expose the full Python-parity tool set.
#[cfg(test)]
mod tests {
    use super::*;

    const EXPECTED: &[&str] = &[
        // 39 Python-parity tools
        "status",
        "navigate",
        "read",
        "screenshot",
        "find",
        "click",
        "type",
        "js",
        "page_info",
        "analyze",
        "fill",
        "form_fill",
        "submit",
        "find_and_click",
        "dismiss_overlay",
        "extract",
        "extract_table",
        "scroll",
        "wait",
        "paginate",
        "console_logs",
        "network_log",
        "metrics",
        "debug",
        "save_cookies",
        "restore_cookies",
        "save_session",
        "session_info",
        "login",
        "browse",
        "upload",
        "download",
        "search",
        "search_images",
        "search_videos",
        "search_twitter_videos",
        "record_task",
        "stop_recording",
        "replay",
        // Rust additions: real multi-tab support
        "new_tab",
        "list_tabs",
        "switch_tab",
        "close_tab",
    ];

    #[test]
    fn registry_has_full_tool_parity() {
        let reg = build_registry();
        let names: std::collections::HashSet<&str> =
            tool_list().iter().map(|t| t.spec().name).collect();
        for name in EXPECTED {
            assert!(names.contains(name), "missing tool: {name}");
        }
        assert_eq!(names.len(), EXPECTED.len(), "tool count mismatch");
        assert_eq!(reg.len(), EXPECTED.len());
    }

    #[test]
    fn no_duplicate_tool_names() {
        let list = tool_list();
        let unique: std::collections::HashSet<&str> = list.iter().map(|t| t.spec().name).collect();
        assert_eq!(unique.len(), list.len(), "duplicate tool name registered");
    }
}
