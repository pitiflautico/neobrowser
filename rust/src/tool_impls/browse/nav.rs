//! Tools for getting somewhere and seeing what is there.
//!
//! `navigate` is the one that carries the most weight: it spends its budget on making the
//! page *usable* rather than merely loaded, because a tool that returns at the load event
//! hands back an empty page as though it were the truth.

use async_trait::async_trait;
use serde_json::{Map, Value};

use crate::page;
use crate::tools::{ParamSpec, ParamType, Tool, ToolCtx, ToolError, ToolOutput, ToolSpec};

use super::super::{arg_bool, arg_f64, arg_i64, arg_str};

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
        crate::tools::check_domain_allowlist(url).map_err(ToolError::Argument)?;
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

        // The HTTP status of the document we landed on.
        //
        // Navigating to a 404 *is* a successful navigation — the browser went there and
        // rendered what it was given — so the status stays `succeeded` and the spec agrees
        // (§3: `failed` means the action did not take place). But an agent handed
        // `succeeded` plus a blank page has no way to tell a 404 from a page that genuinely
        // has no content, and the answer was sitting in the capture layer the whole time.
        // Withholding evidence we already hold is the same defect the conformance suite
        // found in `click` on a disabled control: a diagnosis available for free, not made.
        let http_status = ctx
            .browser
            .network_entries(None, 200)
            .await
            .into_iter()
            .find(|e| {
                e.url == landed || e.url.trim_end_matches('/') == landed.trim_end_matches('/')
            })
            .and_then(|e| e.status);

        let mut result =
            crate::action::ActionResult::new("navigate", status).with_detail(match http_status {
                Some(code) => format!("Navigated to {landed} (HTTP {code})"),
                None => format!("Navigated to {landed}"),
            });
        if let Some(code) = http_status.filter(|c| *c >= 400) {
            result = result.warn(format!(
                "http_{code}: the server returned {code} for this URL. The navigation \
                 happened, but the page is an error page — check the URL rather than \
                 re-reading the content"
            ));
        }
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
                ParamSpec::new(
                    "include_links",
                    ParamType::Boolean,
                    "Append page links as `[text](href)` after the visible text (default false)",
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
        let include_links = arg_bool(args, "include_links", false);
        let tab = ctx.browser.tab().await?;
        let text = page::read_text_with_options(&tab, selector, include_links).await?;
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

// --- record_video ---------------------------------------------------------------

pub struct RecordVideoTool;

#[async_trait]
impl Tool for RecordVideoTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "record_video",
            description: "Record the current tab as an MP4 video for N seconds (default 10). Returns base64 video data.",
            params: vec![ParamSpec::new(
                "seconds",
                ParamType::Integer,
                "Duration in seconds (default 10, max 60)",
            )],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let seconds = arg_i64(args, "seconds", 10).clamp(1, 60) as u64;
        let tab = ctx.browser.tab().await?;
        let data = page::video::record_video_base64(&tab, seconds)
            .await
            .map_err(ToolError::Failed)?;
        Ok(ToolOutput::Image {
            data,
            mime: "video/mp4".into(),
        })
    }
}

// --- network interception ---------------------------------------------------------

pub struct BlockUrlsTool;

#[async_trait]
impl Tool for BlockUrlsTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "block_urls",
            description: "Block network requests by URL pattern (e.g. '*tracker*', '*.doubleclick.net'). Stays active until unblock_urls.",
            params: vec![ParamSpec::new(
                "patterns",
                ParamType::String,
                "Comma-separated URL patterns to block (supports * wildcards)",
            ).required()],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let patterns_str = arg_str(args, "patterns")
            .ok_or_else(|| ToolError::Argument("patterns is required".into()))?;
        let patterns: Vec<&str> = patterns_str
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        let tab = ctx.browser.tab().await?;
        let msg = page::intercept::block_urls(&tab, &patterns)
            .await
            .map_err(ToolError::Failed)?;
        Ok(ToolOutput::text(msg))
    }
}

pub struct UnblockUrlsTool;

#[async_trait]
impl Tool for UnblockUrlsTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "unblock_urls",
            description: "Remove all URL blocks set by block_urls.",
            params: vec![],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        _args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let tab = ctx.browser.tab().await?;
        let msg = page::intercept::unblock_urls(&tab)
            .await
            .map_err(ToolError::Failed)?;
        Ok(ToolOutput::text(msg))
    }
}

pub struct BlockTrackersTool;

#[async_trait]
impl Tool for BlockTrackersTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "block_trackers",
            description:
                "Block common trackers and ads (Google Analytics, Facebook Pixel, Hotjar, etc.).",
            params: vec![],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        _args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let tab = ctx.browser.tab().await?;
        let msg = page::intercept::block_trackers(&tab)
            .await
            .map_err(ToolError::Failed)?;
        Ok(ToolOutput::text(msg))
    }
}
