//! Chrome CDP fetch proxy — execute HTTP requests through a real Chrome instance.
//!
//! Two modes:
//! 1. **Connect to existing Chrome** — connects to neobrowser's Chrome (or any Chrome
//!    with remote debugging). No new process. Uses the REAL session with cookies.
//! 2. **Launch new Chrome** (fallback) — starts headless Chrome if nothing found.

use crate::cdp::CdpClient;
use crate::session::ChromeSession;
use crate::{ChromeError, Result};
use serde_json::json;
use std::collections::{HashMap, HashSet};

/// Result of a fetch through Chrome.
#[derive(Debug, serde::Deserialize, serde::Serialize, Clone)]
pub struct ChromeFetchResult {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: String,
}

/// Proxy that routes HTTP requests through a real Chrome instance via CDP.
pub struct ChromeFetchProxy {
    cdp: CdpClient,
    page_session_id: String,
    cookies_injected: HashSet<String>,
}

impl ChromeFetchProxy {
    /// Best-effort: connect to existing Chrome, fallback to launching new one.
    pub async fn new() -> Result<Self> {
        if let Some(port) = Self::find_existing_chrome().await {
            eprintln!("[chrome-proxy] Connecting to existing Chrome on port {port}");
            match Self::connect_existing(port).await {
                Ok(proxy) => return Ok(proxy),
                Err(e) => eprintln!("[chrome-proxy] Connect failed: {e}, launching new"),
            }
        }
        eprintln!("[chrome-proxy] Launching new headless Chrome");
        Self::launch_new().await
    }

    /// Connect to existing Chrome and find a tab matching the target domain.
    /// If no matching tab found, creates a new one and navigates to the domain.
    async fn connect_existing(port: u16) -> Result<Self> {
        let ws_url = Self::get_ws_url(port).await?;
        let cdp = CdpClient::connect(&ws_url).await?;

        Ok(Self {
            cdp,
            page_session_id: String::new(), // Will be set per-fetch
            cookies_injected: HashSet::new(),
        })
    }

    /// Launch a new headless Chrome.
    async fn launch_new() -> Result<Self> {
        let session = ChromeSession::launch(None, true).await?;
        let cdp = session.cdp;
        let sid = session.page_session_id;
        cdp.send_to(&sid, "Page.navigate", Some(json!({ "url": "about:blank" })))
            .await?;
        Ok(Self {
            cdp,
            page_session_id: sid,
            cookies_injected: HashSet::new(),
        })
    }

    /// Find an existing Chrome with CDP enabled.
    async fn find_existing_chrome() -> Option<u16> {
        // 1. Env var (fastest — set CHROME_CDP_PORT=57725)
        if let Ok(p) = std::env::var("CHROME_CDP_PORT") {
            if let Ok(port) = p.parse::<u16>() {
                if Self::get_ws_url(port).await.is_ok() {
                    return Some(port);
                }
            }
        }

        // 2. Quick scan: common Chrome temp dirs (no slow `find` command)
        let tmpdir = std::env::var("TMPDIR")
            .unwrap_or_else(|_| "/tmp".to_string());
        if let Ok(entries) = std::fs::read_dir(&tmpdir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("agent-browser-chrome-")
                    || name.starts_with("neo-chrome-")
                    || name.starts_with("chromiumoxide-")
                {
                    let port_file = entry.path().join("DevToolsActivePort");
                    if let Ok(content) = std::fs::read_to_string(&port_file) {
                        if let Some(port) = content.lines().next()
                            .and_then(|s| s.trim().parse::<u16>().ok())
                        {
                            if Self::get_ws_url(port).await.is_ok() {
                                eprintln!("[chrome-proxy] Found Chrome at {name} → port {port}");
                                return Some(port);
                            }
                        }
                    }
                }
            }
        }

        // 3. Brute-force common ports
        for port in [9222u16, 9229, 57725] {
            if Self::get_ws_url(port).await.is_ok() {
                eprintln!("[chrome-proxy] Found Chrome on port {port}");
                return Some(port);
            }
        }

        None
    }

    /// Navigate the proxy tab to a URL (for cookie/origin context).
    pub async fn navigate_for_context(&self, url: &str) -> Result<()> {
        self.cdp
            .send_to(
                &self.page_session_id,
                "Page.navigate",
                Some(json!({ "url": url })),
            )
            .await?;
        // Wait for page load
        tokio::time::sleep(std::time::Duration::from_millis(3000)).await;
        Ok(())
    }

    /// Inject cookies via CDP Network.setCookie.
    pub async fn inject_cookies(&mut self, cookies: &[(String, String, String)]) -> Result<()> {
        for (name, value, domain) in cookies {
            if self.cookies_injected.contains(domain) {
                continue;
            }
            self.cdp
                .send_to(
                    &self.page_session_id,
                    "Network.setCookie",
                    Some(json!({
                        "name": name,
                        "value": value,
                        "domain": domain,
                        "path": "/",
                        "secure": true,
                        "httpOnly": true,
                    })),
                )
                .await?;
        }
        for (_, _, domain) in cookies {
            self.cookies_injected.insert(domain.clone());
        }
        Ok(())
    }

    pub fn mark_domain_injected(&mut self, domain: &str) {
        self.cookies_injected.insert(domain.to_string());
    }

    pub fn is_domain_injected(&self, domain: &str) -> bool {
        self.cookies_injected.contains(domain)
    }

    /// Find or create a tab for the target domain and attach to it.
    async fn ensure_tab_for_domain(&mut self, url: &str) -> Result<()> {
        let domain = url::Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(|h| h.to_string()))
            .unwrap_or_default();

        // List all targets, find one matching the domain
        let list_body = crate::launcher::reqwest_lite(
            &format!("http://127.0.0.1:{}/json/list",
                self.cdp.send("Browser.getVersion", None).await
                    .ok().and_then(|v| v.get("webSocketDebuggerUrl")
                        .and_then(|u| u.as_str())
                        .and_then(|u| u.split(':').last())
                        .and_then(|p| p.split('/').next())
                        .and_then(|p| p.parse::<u16>().ok()))
                    .unwrap_or(0)
            )
        ).await.unwrap_or_default();

        // Simpler approach: just list targets via CDP
        let targets = self.cdp.send("Target.getTargets", None).await.unwrap_or_default();
        let target_list = targets.get("targetInfos").and_then(|v| v.as_array());

        if let Some(targets) = target_list {
            for t in targets {
                let t_url = t.get("url").and_then(|v| v.as_str()).unwrap_or("");
                let t_type = t.get("type").and_then(|v| v.as_str()).unwrap_or("");
                let t_id = t.get("targetId").and_then(|v| v.as_str()).unwrap_or("");
                if t_type == "page" && t_url.contains(&domain) && !t_id.is_empty() {
                    // Found a tab on the right domain — attach to it
                    if let Ok(attach) = self.cdp.send(
                        "Target.attachToTarget",
                        Some(json!({ "targetId": t_id, "flatten": true })),
                    ).await {
                        if let Some(sid) = attach.get("sessionId").and_then(|v| v.as_str()) {
                            self.page_session_id = sid.to_string();
                            eprintln!("[chrome-proxy] Attached to existing tab on {domain}");
                            return Ok(());
                        }
                    }
                }
            }
        }

        // No matching tab — create one and navigate
        let result = self.cdp
            .send("Target.createTarget", Some(json!({ "url": format!("https://{domain}") })))
            .await?;
        let target_id = result.get("targetId").and_then(|v| v.as_str())
            .ok_or_else(|| ChromeError::ConnectionFailed("no targetId".into()))?;
        let attach = self.cdp.send(
            "Target.attachToTarget",
            Some(json!({ "targetId": target_id, "flatten": true })),
        ).await?;
        self.page_session_id = attach.get("sessionId").and_then(|v| v.as_str())
            .ok_or_else(|| ChromeError::ConnectionFailed("no sessionId".into()))?.to_string();
        // Wait for page to load
        tokio::time::sleep(std::time::Duration::from_millis(3000)).await;
        eprintln!("[chrome-proxy] Created new tab for {domain}");
        Ok(())
    }

    /// Execute fetch() through Chrome. Uses `credentials: 'include'` for cookies.
    pub async fn fetch(
        &mut self,
        url: &str,
        method: &str,
        body: Option<&str>,
        headers: &HashMap<String, String>,
    ) -> Result<ChromeFetchResult> {
        // Ensure we have a tab on the right domain
        self.ensure_tab_for_domain(url).await?;
        let headers_json = serde_json::to_string(headers).map_err(ChromeError::Json)?;
        let body_js = body
            .map(|b| serde_json::to_string(b).unwrap_or_else(|_| "null".into()))
            .unwrap_or_else(|| "null".into());

        let js = format!(
            r#"(async () => {{
    try {{
        const resp = await fetch({url}, {{
            method: {method},
            headers: {headers},
            body: {body},
            credentials: 'include',
        }});
        const text = await resp.text();
        const h = {{}};
        resp.headers.forEach((v, k) => {{ h[k] = v; }});
        return JSON.stringify({{ status: resp.status, headers: h, body: text }});
    }} catch(e) {{
        return JSON.stringify({{ status: 0, headers: {{}}, body: 'fetch error: ' + e.message }});
    }}
}})()"#,
            url = serde_json::to_string(url).unwrap_or_else(|_| format!("\"{}\"", url)),
            method = serde_json::to_string(method).unwrap_or_else(|_| "\"GET\"".into()),
            headers = headers_json,
            body = body_js,
        );

        let result = self
            .cdp
            .send_to(
                &self.page_session_id,
                "Runtime.evaluate",
                Some(json!({
                    "expression": js,
                    "returnByValue": true,
                    "awaitPromise": true,
                    "timeout": 30000,
                })),
            )
            .await?;

        let value = result
            .get("result")
            .and_then(|r| r.get("value"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                let exc = result
                    .get("exceptionDetails")
                    .and_then(|e| e.get("text"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("unknown");
                ChromeError::CommandFailed {
                    method: "fetch".into(),
                    error: exc.to_string(),
                }
            })?;

        serde_json::from_str(value).map_err(ChromeError::Json)
    }

    async fn get_ws_url(port: u16) -> Result<String> {
        let url = format!("http://127.0.0.1:{port}/json/version");
        let body = crate::launcher::reqwest_lite(&url)
            .await
            .map_err(|e| ChromeError::ConnectionFailed(format!("CDP: {e}")))?;
        let json: serde_json::Value =
            serde_json::from_str(&body).map_err(ChromeError::Json)?;
        json.get("webSocketDebuggerUrl")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| ChromeError::ConnectionFailed("no wsUrl".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chrome_fetch_result_deserialize() {
        let json = r#"{"status":200,"headers":{"content-type":"text/plain"},"body":"ok"}"#;
        let r: ChromeFetchResult = serde_json::from_str(json).unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(r.body, "ok");
    }
}
