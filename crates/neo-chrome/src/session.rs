//! High-level Chrome session — launch + navigate + eval + close.
//!
//! Owns the Chrome process and CDP client. Provides the minimal API
//! that neo-engine needs to use Chrome as a fallback renderer.

use crate::cdp::CdpClient;
use crate::launcher::ChromeProcess;
use crate::{ChromeError, ChromeSessionTrait, Result};
use neo_types::{PageResult, PageState};
use serde_json::json;

/// A live Chrome session: process + CDP connection + active page target.
pub struct ChromeSession {
    pub(crate) process: ChromeProcess,
    pub(crate) cdp: CdpClient,
    /// CDP target ID for the active page.
    pub(crate) target_id: String,
    /// CDP flat session ID attached to the page target.
    pub(crate) page_session_id: String,
}

impl ChromeSession {
    /// Launch Chrome, connect CDP, attach to the first page target.
    pub async fn launch(profile: Option<&str>, headless: bool) -> Result<Self> {
        let process = ChromeProcess::launch(profile, headless).await?;
        // Use sync methods to avoid blocking tokio runtime
        eprintln!("[session] getting ws_url...");
        let ws_url = process.ws_url_sync()?;
        eprintln!("[session] ws_url: {ws_url}");
        eprintln!("[session] connecting CDP WebSocket...");
        let cdp = CdpClient::connect(&ws_url).await?;
        eprintln!("[session] CDP connected");
        let target_id = process.first_target_id_sync()?;

        // Attach to the page target to get a session ID.
        let attach_result = cdp
            .send(
                "Target.attachToTarget",
                Some(json!({
                    "targetId": target_id,
                    "flatten": true,
                })),
            )
            .await?;

        let page_session_id = attach_result
            .get("sessionId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ChromeError::ConnectionFailed("No sessionId in attach response".into()))?
            .to_string();

        // Enable Page domain for navigation events.
        cdp.send_to(&page_session_id, "Page.enable", None).await?;

        Ok(Self {
            process,
            cdp,
            target_id,
            page_session_id,
        })
    }

    /// Navigate to a URL, wait for load, and return page analysis.
    pub async fn navigate(&mut self, url: &str) -> Result<PageResult> {
        let nav_result = self
            .cdp
            .send_to(
                &self.page_session_id,
                "Page.navigate",
                Some(json!({ "url": url })),
            )
            .await?;

        // Check for navigation errors.
        if let Some(err) = nav_result.get("errorText").and_then(|v| v.as_str()) {
            return Err(ChromeError::CommandFailed {
                method: "Page.navigate".into(),
                error: err.to_string(),
            });
        }

        // Wait for load event (best-effort, 15s timeout).
        self.wait_for_load(15_000).await?;

        // Gather page info via JS evaluation.
        self.build_page_result(url).await
    }

    /// Evaluate JavaScript in the page and return the string result.
    pub async fn eval(&self, js: &str) -> Result<String> {
        let result = self
            .cdp
            .send_to(
                &self.page_session_id,
                "Runtime.evaluate",
                Some(json!({
                    "expression": js,
                    "returnByValue": true,
                })),
            )
            .await?;

        // Check for exceptions.
        if let Some(exc) = result.get("exceptionDetails") {
            let text = exc
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("JS exception");
            return Err(ChromeError::CommandFailed {
                method: "Runtime.evaluate".into(),
                error: text.to_string(),
            });
        }

        let value = result
            .get("result")
            .and_then(|r| r.get("value"))
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        match value {
            serde_json::Value::String(s) => Ok(s),
            other => Ok(other.to_string()),
        }
    }

    /// The CDP target ID for this session's page.
    pub fn target_id(&self) -> &str {
        &self.target_id
    }

    /// Close the session: close page target, kill Chrome.
    pub async fn close(mut self) {
        let _ = self
            .cdp
            .send(
                "Target.closeTarget",
                Some(json!({ "targetId": self.target_id })),
            )
            .await;
        self.process.kill().await;
    }

    // ─── Internal helpers ───

    /// Wait for the page to finish loading via a polling loop.
    pub(crate) async fn wait_for_load(&self, timeout_ms: u64) -> Result<()> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            if std::time::Instant::now() > deadline {
                return Err(ChromeError::Timeout("Page load timed out".into()));
            }
            let state = self.eval("document.readyState").await?;
            if state == "complete" || state == "\"complete\"" {
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }

    /// Build a PageResult by evaluating JS to count page elements.
    async fn build_page_result(&self, url: &str) -> Result<PageResult> {
        let js = r#"JSON.stringify({
            title: document.title || '',
            links: document.querySelectorAll('a').length,
            forms: document.querySelectorAll('form').length,
            inputs: document.querySelectorAll('input,textarea,select').length,
            buttons: document.querySelectorAll('button,[type=submit]').length,
            scripts: document.querySelectorAll('script').length
        })"#;

        let json_str = self.eval(js).await?;
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap_or_else(|_| {
            json!({
                "title": "", "links": 0, "forms": 0,
                "inputs": 0, "buttons": 0, "scripts": 0
            })
        });

        Ok(PageResult {
            url: url.to_string(),
            title: parsed["title"].as_str().unwrap_or("").to_string(),
            state: PageState::Complete,
            render_ms: 0,
            links: parsed["links"].as_u64().unwrap_or(0) as usize,
            forms: parsed["forms"].as_u64().unwrap_or(0) as usize,
            inputs: parsed["inputs"].as_u64().unwrap_or(0) as usize,
            buttons: parsed["buttons"].as_u64().unwrap_or(0) as usize,
            scripts: parsed["scripts"].as_u64().unwrap_or(0) as usize,
            errors: vec![],
            redirect_chain: vec![],
            page_id: 0,
        })
    }
}

impl ChromeSessionTrait for ChromeSession {
    fn navigate(
        &mut self,
        url: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<PageResult>> + Send + '_>> {
        let url = url.to_string();
        Box::pin(async move { self.navigate(&url).await })
    }

    fn eval(
        &self,
        js: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send + '_>> {
        let js = js.to_string();
        Box::pin(async move { self.eval(&js).await })
    }
}
