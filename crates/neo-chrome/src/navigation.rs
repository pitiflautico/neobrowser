//! CDP navigation tools — page lifecycle, tabs, and history.
//!
//! Extends `ChromeSession` with methods for navigating pages, opening
//! and closing tabs, listing pages, and switching between targets.

use crate::launcher::reqwest_lite;
use crate::session::ChromeSession;
use crate::{ChromeError, Result};
use serde_json::json;

/// Information about an open browser page/tab.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PageInfo {
    /// CDP target ID.
    pub id: String,
    /// Page title.
    pub title: String,
    /// Current URL.
    pub url: String,
    /// Target type (usually "page").
    pub page_type: String,
}

/// Navigation action type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationType {
    /// Navigate to a URL.
    Url,
    /// Go back in history.
    Back,
    /// Go forward in history.
    Forward,
    /// Reload the current page.
    Reload,
}

impl ChromeSession {
    /// Navigate the current page by URL, history direction, or reload.
    ///
    /// - `Url`: navigates to `url` (required).
    /// - `Back`/`Forward`: uses `window.history` via Runtime.evaluate.
    /// - `Reload`: reloads the page; `ignore_cache` forces bypass cache.
    ///
    /// Waits for load after navigation (up to `timeout_ms`, default 15s).
    pub async fn navigate_page(
        &mut self,
        url: Option<&str>,
        nav_type: NavigationType,
        ignore_cache: bool,
        timeout_ms: Option<u64>,
    ) -> Result<()> {
        let timeout = timeout_ms.unwrap_or(15_000);

        match nav_type {
            NavigationType::Url => {
                let target_url = url.ok_or_else(|| ChromeError::CommandFailed {
                    method: "navigate_page".into(),
                    error: "URL required for NavigationType::Url".into(),
                })?;

                let nav_result = self
                    .cdp
                    .send_to(
                        &self.page_session_id,
                        "Page.navigate",
                        Some(json!({ "url": target_url })),
                    )
                    .await?;

                if let Some(err) = nav_result.get("errorText").and_then(|v| v.as_str()) {
                    return Err(ChromeError::CommandFailed {
                        method: "Page.navigate".into(),
                        error: err.to_string(),
                    });
                }
            }
            NavigationType::Back => {
                self.cdp
                    .send_to(
                        &self.page_session_id,
                        "Runtime.evaluate",
                        Some(json!({
                            "expression": "window.history.back()",
                            "returnByValue": true,
                        })),
                    )
                    .await?;
            }
            NavigationType::Forward => {
                self.cdp
                    .send_to(
                        &self.page_session_id,
                        "Runtime.evaluate",
                        Some(json!({
                            "expression": "window.history.forward()",
                            "returnByValue": true,
                        })),
                    )
                    .await?;
            }
            NavigationType::Reload => {
                self.cdp
                    .send_to(
                        &self.page_session_id,
                        "Page.reload",
                        Some(json!({ "ignoreCache": ignore_cache })),
                    )
                    .await?;
            }
        }

        self.wait_for_load(timeout).await
    }

    /// Open a new tab.
    ///
    /// If `background` is false, the new tab is activated (brought to front).
    /// Returns the target ID of the new page.
    pub async fn new_page(&mut self, url: &str, background: bool) -> Result<String> {
        let result = self
            .cdp
            .send(
                "Target.createTarget",
                Some(json!({ "url": url })),
            )
            .await?;

        let new_target_id = result
            .get("targetId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ChromeError::CommandFailed {
                    method: "Target.createTarget".into(),
                    error: "No targetId in response".into(),
                }
            })?
            .to_string();

        if !background {
            self.cdp
                .send(
                    "Target.activateTarget",
                    Some(json!({ "targetId": &new_target_id })),
                )
                .await?;
        }

        Ok(new_target_id)
    }

    /// List all open pages using Chrome's `/json/list` HTTP endpoint.
    pub async fn list_pages(&self) -> Result<Vec<PageInfo>> {
        let url = format!("http://127.0.0.1:{}/json/list", self.process.port);
        let body = reqwest_lite(&url)
            .await
            .map_err(|e| ChromeError::ConnectionFailed(format!("/json/list failed: {e}")))?;

        let targets: serde_json::Value =
            serde_json::from_str(&body).map_err(ChromeError::Json)?;

        let arr = targets.as_array().ok_or_else(|| {
            ChromeError::ConnectionFailed("/json/list did not return an array".into())
        })?;

        let pages = arr
            .iter()
            .filter(|t| t.get("type").and_then(|v| v.as_str()) == Some("page"))
            .map(|t| PageInfo {
                id: t.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                title: t.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                url: t.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                page_type: t.get("type").and_then(|v| v.as_str()).unwrap_or("page").to_string(),
            })
            .collect();

        Ok(pages)
    }

    /// Select a page target for future CDP commands.
    ///
    /// Attaches to the target with `flatten: true` and updates the
    /// internal `page_session_id` and `target_id`.
    /// If `bring_to_front` is true, also activates the target.
    pub async fn select_page(&mut self, page_id: &str, bring_to_front: bool) -> Result<()> {
        let attach_result = self
            .cdp
            .send(
                "Target.attachToTarget",
                Some(json!({
                    "targetId": page_id,
                    "flatten": true,
                })),
            )
            .await?;

        let new_session_id = attach_result
            .get("sessionId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ChromeError::ConnectionFailed("No sessionId in attach response".into())
            })?
            .to_string();

        // Enable Page domain on the new session.
        self.cdp
            .send_to(&new_session_id, "Page.enable", None)
            .await?;

        self.page_session_id = new_session_id;
        self.target_id = page_id.to_string();

        if bring_to_front {
            self.cdp
                .send(
                    "Target.activateTarget",
                    Some(json!({ "targetId": page_id })),
                )
                .await?;
        }

        Ok(())
    }

    /// Close a browser tab by its target ID.
    pub async fn close_page(&mut self, page_id: &str) -> Result<()> {
        self.cdp
            .send(
                "Target.closeTarget",
                Some(json!({ "targetId": page_id })),
            )
            .await?;
        Ok(())
    }
}
