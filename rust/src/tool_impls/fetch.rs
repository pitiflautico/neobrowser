//! Reaching outside the current page: server-side fetch, file upload and
//! download, paginated extraction. All SSRF- and root-guarded.
//!
//! Split out of a single 2700-line `tool_impls.rs`: at 67 tools that file had
//! stopped being navigable, and a reviewer could not tell which tools a change
//! touched.

use async_trait::async_trait;
use serde_json::{json, Map, Value};

use crate::page;
use crate::reach;
use crate::tools::{ParamSpec, ParamType, Tool, ToolCtx, ToolError, ToolOutput, ToolSpec};

use super::{arg_f64, arg_i64, arg_str};

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

pub struct ExtractPaginatedTool;

#[async_trait]
impl Tool for ExtractPaginatedTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "extract_paginated",
            description: "Extract a table or list across pages: extract, click next, repeat until the content stops changing or max_pages is reached. Stops on a repeated page rather than looping, and reports how many pages it actually covered.",
            params: vec![
                ParamSpec::new("selector", ParamType::String, "CSS selector of the table or list container").required(),
                ParamSpec::new("next_selector", ParamType::String, "CSS selector of the 'next page' control").required(),
                ParamSpec::new("max_pages", ParamType::Integer, "Safety cap (default 10, max 100)"),
                ParamSpec::new("budget_s", ParamType::Number, "Total seconds for the whole crawl (default 60)"),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        use crate::action::Budget;

        let selector = arg_str(args, "selector")
            .ok_or_else(|| ToolError::Argument("extract_paginated: selector required".into()))?;
        let next_selector = arg_str(args, "next_selector").ok_or_else(|| {
            ToolError::Argument("extract_paginated: next_selector required".into())
        })?;
        let max_pages = arg_i64(args, "max_pages", 10).clamp(1, 100);
        let budget = Budget::from_secs(arg_f64(args, "budget_s", 60.0));

        let tab = ctx.browser.tab().await?;
        let mut pages: Vec<String> = Vec::new();
        let mut seen: std::collections::HashSet<String> = Default::default();
        let mut stop_reason = "max_pages";

        for page_n in 0..max_pages {
            if budget.expired() {
                stop_reason = "budget_exhausted";
                break;
            }
            let text = page::read_text(&tab, selector).await.unwrap_or_default();
            // A page identical to one already captured means the "next" control is
            // inert or wrapped around. Continuing would loop forever collecting
            // duplicates, so stop and say why.
            if !seen.insert(text.clone()) {
                stop_reason = "repeated_page";
                break;
            }
            pages.push(text);

            let outcome = page::click_selector(&tab, next_selector).await?;
            if !matches!(
                outcome,
                page::ClickOutcome::Clicked | page::ClickOutcome::NoLayoutUsedJs
            ) {
                stop_reason = "no_next_control";
                break;
            }
            // Wait for the content to actually change before reading again, rather
            // than sleeping and hoping.
            let before = crate::action::observe(&tab).await;
            let step_budget = Budget::from_secs(budget.remaining().as_secs_f64().min(5.0));
            let (_, changed) = crate::action::wait_for_change(&tab, &before, &step_budget).await;
            if !changed {
                stop_reason = "page_did_not_change_after_next";
                break;
            }
            let _ = page_n;
        }

        let origin = page::current_url(&tab).await.unwrap_or_default();
        // Fenced: this is page content, and a paginated table is as capable of
        // carrying an injection attempt as any other text.
        let joined = pages.join("\n\n--- page break ---\n\n");
        let wrapped = crate::untrusted::wrap(&origin, &joined);
        Ok(ToolOutput::text(
            json!({
                "ok": !pages.is_empty(),
                "pages_extracted": pages.len(),
                "stop_reason": stop_reason,
                "trust": wrapped["trust"].clone(),
                "content": wrapped["content"].clone(),
                "injection": wrapped.get("injection").cloned(),
            })
            .to_string(),
        ))
    }
}
