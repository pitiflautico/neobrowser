//! Multi-source search that routes around walled providers.
//!
//! Split out of a single 2700-line `tool_impls.rs`: at 67 tools that file had
//! stopped being navigable, and a reviewer could not tell which tools a change
//! touched.

use async_trait::async_trait;
use serde_json::{Map, Value};

use crate::search;
use crate::tools::{ParamSpec, ParamType, Tool, ToolCtx, ToolError, ToolOutput, ToolSpec};

use super::{arg_i64, arg_str};

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
