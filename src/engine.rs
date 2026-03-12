//! Browser engine — raw CDP, no chromiumoxide.
//!
//! Launches Chrome, connects via WebSocket, drives everything
//! through direct CDP commands. No lifecycle waits, no abstractions
//! that block. Just send command → get result.

use crate::cdp::CdpSession;
use crate::semantic;
use crate::vision;
use crate::wom;
use html5ever::parse_document;
use html5ever::tendril::TendrilSink;
use markup5ever_rcdom::RcDom;
use rusqlite::params;
use serde_json::{json, Value};
use std::time::Instant;

// ─── Chrome binary discovery ───

fn find_chrome() -> Result<&'static str, &'static str> {
    let paths = [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
        "/usr/bin/google-chrome",
    ];
    for p in paths {
        if std::path::Path::new(p).exists() {
            return Ok(p);
        }
    }
    Err("Chrome/Chromium not found")
}

/// Find Chrome's DevToolsActivePort file to get the debug port.
fn find_debug_port() -> Option<(u16, String)> {
    let home = dirs::home_dir()?;
    // Standard macOS Chrome location
    let paths = [
        home.join("Library/Application Support/Google/Chrome/DevToolsActivePort"),
        home.join(".config/chromium/DevToolsActivePort"),
    ];
    for path in &paths {
        if let Ok(content) = std::fs::read_to_string(path) {
            let lines: Vec<&str> = content.lines().collect();
            if let Some(port_str) = lines.first() {
                if let Ok(port) = port_str.parse::<u16>() {
                    let ws_path = lines.get(1).unwrap_or(&"/devtools/browser");
                    return Some((port, ws_path.to_string()));
                }
            }
        }
    }
    None
}

// ─── Session ───

pub struct Session {
    cdp: CdpSession,
    /// CDP target ID for the active page
    target_id: String,
    /// Page-level CDP session (attached to a target)
    page_session_id: String,
    pub last_url: String,
    chrome_process: Option<tokio::process::Child>,
    connected_mode: bool,
}

/// Default persistent profile directory for the AI browser.
pub fn default_profile_dir() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".neobrowser")
        .join("profile")
}

/// Write cookies directly to the Chrome profile's SQLite database.
/// Must be called BEFORE Chrome launches (Chrome locks the file).
pub fn persist_cookies_to_profile(
    profile_dir: &std::path::Path,
    cookie_file: &str,
) -> Result<usize, Box<dyn std::error::Error>> {
    let content = std::fs::read_to_string(cookie_file)?;
    let data: Value = serde_json::from_str(&content)?;
    let cookies = if let Some(arr) = data.as_array() {
        arr.clone()
    } else if let Some(arr) = data.get("cookies").and_then(|c| c.as_array()) {
        arr.clone()
    } else {
        return Err("Invalid cookie format".into());
    };

    let db_path = profile_dir.join("Default").join("Cookies");
    std::fs::create_dir_all(profile_dir.join("Default"))?;

    let conn = rusqlite::Connection::open(&db_path)?;

    // Create table if first run
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS cookies (
            creation_utc INTEGER NOT NULL,
            host_key TEXT NOT NULL DEFAULT '',
            top_frame_site_key TEXT NOT NULL DEFAULT '',
            name TEXT NOT NULL DEFAULT '',
            value TEXT NOT NULL DEFAULT '',
            encrypted_value BLOB NOT NULL DEFAULT X'',
            path TEXT NOT NULL DEFAULT '/',
            expires_utc INTEGER NOT NULL DEFAULT 0,
            is_secure INTEGER NOT NULL DEFAULT 0,
            is_httponly INTEGER NOT NULL DEFAULT 0,
            last_access_utc INTEGER NOT NULL DEFAULT 0,
            has_expires INTEGER NOT NULL DEFAULT 1,
            is_persistent INTEGER NOT NULL DEFAULT 1,
            priority INTEGER NOT NULL DEFAULT 1,
            samesite INTEGER NOT NULL DEFAULT -1,
            source_scheme INTEGER NOT NULL DEFAULT 0,
            source_port INTEGER NOT NULL DEFAULT -1,
            last_update_utc INTEGER NOT NULL DEFAULT 0,
            source_type INTEGER NOT NULL DEFAULT 0,
            has_cross_site_ancestor INTEGER NOT NULL DEFAULT 0,
            UNIQUE (host_key, top_frame_site_key, name, path, source_scheme, source_port, has_cross_site_ancestor)
        );"
    )?;

    // Chrome epoch: microseconds since 1601-01-01
    // Unix epoch offset in microseconds: 11644473600 * 1_000_000
    let chrome_epoch_offset: i64 = 11_644_473_600 * 1_000_000;
    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_micros() as i64;
    let now_chrome = now_unix + chrome_epoch_offset;
    let expires_30d = now_chrome + 30 * 86400 * 1_000_000;

    let mut count = 0;
    for c in &cookies {
        let name = match c.get("name").and_then(|v| v.as_str()) {
            Some(n) => n,
            None => continue,
        };
        let value = c.get("value").and_then(|v| v.as_str()).unwrap_or("");
        let domain = c.get("domain").and_then(|v| v.as_str()).unwrap_or("");
        let path = c.get("path").and_then(|v| v.as_str()).unwrap_or("/");
        let secure = c.get("secure").and_then(|v| v.as_bool()).unwrap_or(false) as i32;
        let http_only = c.get("httpOnly").and_then(|v| v.as_bool()).unwrap_or(false) as i32;

        // Convert expires from unix seconds to chrome microseconds
        let expires_unix = c.get("expires").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let expires_chrome = if expires_unix > 1.0 {
            (expires_unix as i64) * 1_000_000 + chrome_epoch_offset
        } else {
            expires_30d
        };

        let source_scheme = if secure == 1 { 2 } else { 1 };

        // REPLACE — update if exists, insert if not
        conn.execute(
            "INSERT OR REPLACE INTO cookies (
                creation_utc, host_key, name, value, encrypted_value,
                path, expires_utc, is_secure, is_httponly,
                last_access_utc, has_expires, is_persistent,
                priority, samesite, source_scheme, source_port,
                last_update_utc, top_frame_site_key, source_type, has_cross_site_ancestor
            ) VALUES (?1,?2,?3,?4,X'',?5,?6,?7,?8,?9,1,1,1,-1,?10,-1,?9,'',0,0)",
            params![
                now_chrome, domain, name, value,
                path, expires_chrome, secure, http_only,
                now_chrome, source_scheme,
            ],
        )?;
        count += 1;
    }

    eprintln!("[ENGINE] Persisted {count} cookies to profile SQLite");
    Ok(count)
}

impl Session {
    /// Launch a new Chrome with a random debug port, connect via CDP.
    /// Always uses a persistent profile directory so sessions survive restarts.
    /// - `user_data_dir = Some(path)` → use that specific profile
    /// - `user_data_dir = None` → use ~/.neobrowser/profile/ (default)
    pub async fn launch(
        user_data_dir: Option<&str>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let chrome = find_chrome()?;

        // Persistent profile — always
        let profile_dir = match user_data_dir {
            Some(dir) => std::path::PathBuf::from(dir),
            None => default_profile_dir(),
        };
        std::fs::create_dir_all(&profile_dir)?;
        let profile_str = profile_dir.to_string_lossy().to_string();

        // Find a free port
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        drop(listener);

        let mut args = vec![
            format!("--remote-debugging-port={port}"),
            format!("--user-data-dir={profile_str}"),
            "--headless=new".to_string(),
            "--disable-blink-features=AutomationControlled".to_string(),
            "--disable-gpu".to_string(),
            "--disable-dev-shm-usage".to_string(),
            "--disable-default-apps".to_string(),
            "--disable-sync".to_string(),
            "--no-first-run".to_string(),
            "--no-default-browser-check".to_string(),
            "--window-size=1440,900".to_string(),
        ];

        let child = tokio::process::Command::new(chrome)
            .args(&args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        eprintln!("[ENGINE] Chrome launching on port {port}...");

        // Wait for Chrome to be ready (poll /json/version)
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()?;

        let mut ws_url = String::new();
        for _ in 0..30 {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let url = format!("http://127.0.0.1:{port}/json/version");
            if let Ok(resp) = client.get(&url).send().await {
                if let Ok(data) = resp.json::<Value>().await {
                    if let Some(ws) = data["webSocketDebuggerUrl"].as_str() {
                        ws_url = ws.to_string();
                        break;
                    }
                }
            }
        }
        if ws_url.is_empty() {
            return Err("Chrome didn't start in time".into());
        }

        let cdp = CdpSession::connect(&ws_url).await?;

        // Create a new page (target)
        let result = cdp
            .send("Target.createTarget", Some(json!({"url": "about:blank"})))
            .await?;
        let target_id = result["targetId"]
            .as_str()
            .ok_or("No targetId")?
            .to_string();

        // Attach to the target to get a session
        let result = cdp
            .send(
                "Target.attachToTarget",
                Some(json!({"targetId": target_id, "flatten": true})),
            )
            .await?;
        let session_id = result["sessionId"]
            .as_str()
            .ok_or("No sessionId")?
            .to_string();

        // Enable Page and Runtime domains
        cdp.send_to(&session_id, "Page.enable", None).await?;
        cdp.send_to(&session_id, "Runtime.enable", None).await?;

        // Stealth: remove webdriver flag
        cdp.send_to(
            &session_id,
            "Page.addScriptToEvaluateOnNewDocument",
            Some(json!({
                "source": "Object.defineProperty(navigator, 'webdriver', {get: () => undefined})"
            })),
        )
        .await?;

        eprintln!("[ENGINE] Ready — target={}, session={}", &target_id[..8], &session_id[..8]);

        Ok(Self {
            cdp,
            target_id,
            page_session_id: session_id,
            last_url: String::new(),
            chrome_process: Some(child),
            connected_mode: false,
        })
    }

    /// Connect to an already-running Chrome via its debug port.
    pub async fn connect_port(port: u16) -> Result<Self, Box<dyn std::error::Error>> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()?;

        let url = format!("http://127.0.0.1:{port}/json/version");
        let resp = client.get(&url).send().await?;
        let data: Value = resp.json().await?;
        let ws_url = data["webSocketDebuggerUrl"]
            .as_str()
            .ok_or("No webSocketDebuggerUrl")?;

        Self::connect(ws_url).await
    }

    /// Connect to Chrome's WebSocket directly.
    pub async fn connect(ws_url: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let cdp = CdpSession::connect(ws_url).await?;

        // Get existing pages
        let result = cdp.send("Target.getTargets", None).await?;
        let targets = result["targetInfos"].as_array().ok_or("No targets")?;

        // Find the first page target
        let page_target = targets
            .iter()
            .find(|t| t["type"].as_str() == Some("page"))
            .ok_or("No page target found")?;

        let target_id = page_target["targetId"]
            .as_str()
            .ok_or("No targetId")?
            .to_string();

        // Attach
        let result = cdp
            .send(
                "Target.attachToTarget",
                Some(json!({"targetId": target_id, "flatten": true})),
            )
            .await?;
        let session_id = result["sessionId"]
            .as_str()
            .ok_or("No sessionId")?
            .to_string();

        cdp.send_to(&session_id, "Page.enable", None).await?;
        cdp.send_to(&session_id, "Runtime.enable", None).await?;

        let current_url = page_target["url"]
            .as_str()
            .unwrap_or("")
            .to_string();

        eprintln!("[ENGINE] Connected to existing Chrome — {current_url}");

        Ok(Self {
            cdp,
            target_id,
            page_session_id: session_id,
            last_url: current_url,
            chrome_process: None,
            connected_mode: true,
        })
    }

    /// Connect to user's running Chrome (reads DevToolsActivePort).
    pub async fn connect_running() -> Result<Self, Box<dyn std::error::Error>> {
        let (port, _ws_path) =
            find_debug_port().ok_or("Chrome not running with --remote-debugging-port")?;
        Self::connect_port(port).await
    }

    // ─── Frame-aware helpers ───

    /// JS snippet that resolves the "active document" — the frame with real content.
    /// In a frameset, the top-level document is just <frameset> with no interactive elements.
    /// This finds the largest same-origin frame and returns its document.
    /// Outside framesets, returns the normal document.
    const ACTIVE_DOC_JS: &'static str = r#"
        (function() {
            // Quick check: if top document has interactive content, use it
            var topCount = document.querySelectorAll('a,button,input,select,textarea').length;
            if (topCount > 5 || !document.querySelector('frameset, iframe')) return document;

            // Frameset detected with sparse top content — find the best frame
            var best = null, bestScore = 0;
            var frames = document.querySelectorAll('frame, iframe');
            for (var i = 0; i < frames.length; i++) {
                try {
                    var doc = frames[i].contentDocument;
                    if (!doc || !doc.body) continue;
                    var score = doc.querySelectorAll('a,button,input,select,textarea').length;
                    // Also consider text length for content-heavy frames
                    score += Math.min(doc.body.innerText.length / 100, 50);
                    if (score > bestScore) { bestScore = score; best = doc; }
                } catch(e) { /* cross-origin frame, skip */ }
            }
            return best || document;
        })()
    "#;

    /// Wraps a JS expression to run against the active document (frame-aware).
    /// The expression receives `_doc` as the resolved document.
    #[allow(dead_code)]
    fn frame_aware_js(js_using_doc: &str) -> String {
        format!(
            r#"(() => {{ const _doc = {active_doc}; {body} }})()"#,
            active_doc = Self::ACTIVE_DOC_JS,
            body = js_using_doc,
        )
    }

    // ─── CDP helpers ───

    /// Evaluate JS in the page, return the result as string.
    pub async fn eval_string(&self, expression: &str) -> Result<String, Box<dyn std::error::Error>> {
        let result = self
            .cdp
            .send_to(
                &self.page_session_id,
                "Runtime.evaluate",
                Some(json!({
                    "expression": expression,
                    "returnByValue": true,
                    "awaitPromise": true,
                })),
            )
            .await?;

        if let Some(exception) = result.get("exceptionDetails") {
            let text = exception
                .get("text")
                .and_then(|t| t.as_str())
                .unwrap_or("JS exception");
            return Err(text.to_string().into());
        }

        let value = &result["result"]["value"];
        match value {
            Value::String(s) => Ok(s.clone()),
            Value::Null => Ok(String::new()),
            other => Ok(other.to_string()),
        }
    }

    /// Get the current page URL.
    async fn current_url(&self) -> Result<String, Box<dyn std::error::Error>> {
        self.eval_string("window.location.href").await
    }

    /// Get the current page title.
    async fn current_title(&self) -> Result<String, Box<dyn std::error::Error>> {
        self.eval_string("document.title || ''").await
    }

    // ─── Cookies ───

    pub async fn load_cookies(&self, path: &str) -> Result<usize, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        let data: Value = serde_json::from_str(&content)?;

        let cookies = if let Some(arr) = data.as_array() {
            arr.clone()
        } else if let Some(arr) = data.get("cookies").and_then(|c| c.as_array()) {
            arr.clone()
        } else {
            return Err("Invalid cookie format".into());
        };

        let count = cookies.len();

        // 30 days from now — ensures Chrome persists cookies to disk
        let expires_default = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64()
            + 30.0 * 86400.0;

        // Build CDP cookies array
        let cdp_cookies: Vec<Value> = cookies
            .iter()
            .filter_map(|c| {
                let name = c.get("name")?.as_str()?;
                let value = c.get("value")?.as_str()?;
                let domain = c.get("domain").and_then(|v| v.as_str()).unwrap_or("");
                let path = c.get("path").and_then(|v| v.as_str()).unwrap_or("/");
                let secure = c.get("secure").and_then(|v| v.as_bool()).unwrap_or(false);
                let http_only = c.get("httpOnly").and_then(|v| v.as_bool()).unwrap_or(false);

                // Use provided expiry, or force 30 days so Chrome persists them
                let expires = c.get("expires")
                    .and_then(|v| v.as_f64())
                    .filter(|&e| e > 1.0)
                    .unwrap_or(expires_default);

                let scheme = if secure { "https" } else { "http" };
                let clean = domain.trim_start_matches('.');
                let url = format!("{scheme}://{clean}/");

                Some(json!({
                    "name": name,
                    "value": value,
                    "domain": domain,
                    "path": path,
                    "secure": secure,
                    "httpOnly": http_only,
                    "url": url,
                    "expires": expires,
                }))
            })
            .collect();

        // Network.setCookies for the current session
        self.cdp
            .send_to(
                &self.page_session_id,
                "Network.setCookies",
                Some(json!({"cookies": cdp_cookies})),
            )
            .await?;

        eprintln!("[ENGINE] Injected {count} cookies");
        Ok(count)
    }

    // ─── Navigation ───

    pub async fn goto(&mut self, url: &str) -> Result<(), Box<dyn std::error::Error>> {
        let t0 = Instant::now();

        // Page.navigate — just sends the command, does NOT wait for lifecycle.
        // This is the key difference from chromiumoxide: no hanging.
        let result = self
            .cdp
            .send_to(
                &self.page_session_id,
                "Page.navigate",
                Some(json!({"url": url})),
            )
            .await?;

        if let Some(error) = result.get("errorText").and_then(|e| e.as_str()) {
            if !error.is_empty() {
                return Err(format!("Navigation error: {error}").into());
            }
        }

        // Wait for content to be usable — poll until DOM has interactive elements.
        // This replaces both the lifecycle wait AND the SPA wait.
        self.wait_for_content(5000).await;

        self.last_url = url.to_string();
        eprintln!("[ENGINE] goto {url} ({}ms)", t0.elapsed().as_millis());
        Ok(())
    }

    /// Wait until the page has enough interactive content to be useful.
    async fn wait_for_content(&self, max_ms: u64) {
        let js = r#"
            (() => {
                const count = document.querySelectorAll('a,button,input,select,textarea,[role]').length;
                const textLen = (document.body ? document.body.innerText.length : 0);
                return JSON.stringify({count, textLen});
            })()
        "#;

        let mut prev_count = -1i64;
        let mut stable = 0;
        let mut elapsed = 0u64;
        let interval = 300u64;

        // Initial wait for frameworks to bootstrap
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        elapsed += 500;

        while elapsed < max_ms {
            if let Ok(result) = self.eval_string(js).await {
                if let Ok(data) = serde_json::from_str::<Value>(&result) {
                    let count = data["count"].as_i64().unwrap_or(0);
                    let text_len = data["textLen"].as_i64().unwrap_or(0);

                    if count > 3 && count == prev_count && text_len > 30 {
                        return; // Content stabilized
                    }
                    if stable >= 2 {
                        return; // Stable enough
                    }
                    if count == prev_count {
                        stable += 1;
                    } else {
                        stable = 0;
                    }
                    prev_count = count;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(interval)).await;
            elapsed += interval;
        }
    }

    pub async fn back(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.eval_string("window.history.back()").await?;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        eprintln!("[ENGINE] back");
        Ok(())
    }

    pub async fn forward(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.eval_string("window.history.forward()").await?;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        eprintln!("[ENGINE] forward");
        Ok(())
    }

    pub async fn reload(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.cdp
            .send_to(&self.page_session_id, "Page.reload", None)
            .await?;
        self.wait_for_content(5000).await;
        eprintln!("[ENGINE] reload");
        Ok(())
    }

    // ─── Perception ───

    /// Tag interactive DOM elements with data-wom-id so act() can find them
    /// by WOM ID directly. This runs after each observe/WOM build.
    /// Tag DOM elements using the exact WOM node IDs.
    /// Walks the DOM in the same order as the WOM builder, assigning matching IDs.
    async fn tag_dom_elements_from_wom(&self, doc: &wom::WomDocument) -> Result<u64, Box<dyn std::error::Error>> {
        // Strategy: clear old tags, then use the same traversal order as WOM builder.
        // Since WOM builder walks document order (skipping nav/footer for links),
        // we replicate that in JS with the same prefix counters.
        let js = r#"
            (() => {
                // Remove old tags
                document.querySelectorAll('[data-wom-id]').forEach(el => el.removeAttribute('data-wom-id'));

                let hC = 0, lnkC = 0, btnC = 0, fldC = 0, selC = 0, imgC = 0, pC = 0;
                const pad = (n) => String(n).padStart(3, '0');

                function isInNavOrFooter(el) {
                    let p = el.parentElement;
                    while (p) {
                        const tag = p.tagName?.toLowerCase();
                        if (tag === 'nav' || tag === 'footer') return true;
                        p = p.parentElement;
                    }
                    return false;
                }

                // Walk DOM in document order (same as WOM builder)
                const walker = document.createTreeWalker(
                    document.documentElement,
                    NodeFilter.SHOW_ELEMENT,
                    null
                );

                let el;
                while (el = walker.nextNode()) {
                    const tag = el.tagName?.toLowerCase();
                    if (!tag) continue;

                    // Headings
                    if (/^h[1-6]$/.test(tag)) {
                        const t = (el.textContent || '').trim();
                        if (t) el.setAttribute('data-wom-id', 'h_' + pad(++hC));
                        continue;
                    }

                    // Links (skip nav/footer — matches WOM builder)
                    if (tag === 'a') {
                        const t = (el.textContent || '').trim();
                        if (t && !isInNavOrFooter(el)) {
                            el.setAttribute('data-wom-id', 'lnk_' + pad(++lnkC));
                        }
                        continue;
                    }

                    // Buttons, summary, role=button
                    if (tag === 'button' || tag === 'summary' || el.getAttribute('role') === 'button') {
                        const t = (el.textContent || el.value || '').trim();
                        if (t && t.length < 120) el.setAttribute('data-wom-id', 'btn_' + pad(++btnC));
                        continue;
                    }

                    // Contenteditable or role=textbox (BEFORE input check)
                    if (el.getAttribute('contenteditable') === 'true' || el.getAttribute('role') === 'textbox') {
                        el.setAttribute('data-wom-id', 'fld_' + pad(++fldC));
                        continue;
                    }

                    // Inputs and textareas
                    if (tag === 'input' || tag === 'textarea') {
                        const itype = (el.type || 'text').toLowerCase();
                        if (itype === 'hidden') continue;
                        if (itype === 'submit') {
                            el.setAttribute('data-wom-id', 'btn_' + pad(++btnC));
                        } else {
                            el.setAttribute('data-wom-id', 'fld_' + pad(++fldC));
                        }
                        continue;
                    }

                    // Selects
                    if (tag === 'select') {
                        el.setAttribute('data-wom-id', 'sel_' + pad(++selC));
                        continue;
                    }

                    // Forms
                    if (tag === 'form') {
                        // Don't skip children — let walker continue
                    }
                }

                return String(document.querySelectorAll('[data-wom-id]').length);
            })()
        "#;
        let result = self.eval_string(js).await?;
        let count = result.parse::<u64>().unwrap_or(0);
        Ok(count)
    }

    #[allow(dead_code)]
    async fn tag_dom_elements(&self) -> Result<u64, Box<dyn std::error::Error>> {
        let js = format!(
            r#"
            (() => {{
                const _doc = {active_doc};

                // Remove old tags
                _doc.querySelectorAll('[data-wom-id]').forEach(el => el.removeAttribute('data-wom-id'));

                let btnC = 0, lnkC = 0, fldC = 0, hC = 0, selC = 0, imgC = 0;
                const pad = (n) => String(n).padStart(3, '0');

                // Tag headings
                _doc.querySelectorAll('h1,h2,h3,h4,h5,h6').forEach(el => {{
                    const t = (el.textContent || '').trim();
                    if (t) el.setAttribute('data-wom-id', 'h_' + pad(++hC));
                }});

                // Tag links (skip nav/footer to match WOM builder)
                _doc.querySelectorAll('a').forEach(el => {{
                    const t = (el.textContent || '').trim();
                    if (t) el.setAttribute('data-wom-id', 'lnk_' + pad(++lnkC));
                }});

                // Tag buttons
                _doc.querySelectorAll('button, summary, [role="button"], input[type="submit"]').forEach(el => {{
                    const t = (el.textContent || el.value || '').trim();
                    if (t && t.length < 120) el.setAttribute('data-wom-id', 'btn_' + pad(++btnC));
                }});

                // Tag inputs
                _doc.querySelectorAll('input:not([type="hidden"]):not([type="submit"]), textarea, [contenteditable="true"], [role="textbox"], [role="combobox"], [role="searchbox"]').forEach(el => {{
                    el.setAttribute('data-wom-id', 'fld_' + pad(++fldC));
                }});

                // Tag selects
                _doc.querySelectorAll('select').forEach(el => {{
                    el.setAttribute('data-wom-id', 'sel_' + pad(++selC));
                }});

                // Tag checkboxes/radios
                _doc.querySelectorAll('input[type="checkbox"], input[type="radio"]').forEach(el => {{
                    el.setAttribute('data-wom-id', 'fld_' + pad(++fldC));
                }});

                const total = btnC + lnkC + fldC + hC + selC;
                return String(total);
            }})()
            "#,
            active_doc = Self::ACTIVE_DOC_JS,
        );
        let result = self.eval_string(&js).await?;
        let count = result.parse::<u64>().unwrap_or(0);
        Ok(count)
    }

    /// Click an element by its WOM ID (data-wom-id attribute).
    /// Returns true if found and clicked.
    pub async fn click_by_wom_id(&self, wom_id: &str) -> Result<bool, Box<dyn std::error::Error>> {
        let js = format!(
            r#"
            (() => {{
                const _doc = {active_doc};
                const el = _doc.querySelector('[data-wom-id="{wom_id}"]');
                if (!el) return 'not_found';
                el.scrollIntoViewIfNeeded();
                el.click();
                const t = (el.textContent || el.value || el.getAttribute('aria-label') || '').trim();
                return 'clicked[' + '{wom_id}' + ']: ' + t.substring(0, 60);
            }})()
            "#,
            active_doc = Self::ACTIVE_DOC_JS,
            wom_id = wom_id
        );
        let result = self.eval_string(&js).await?;
        let found = !result.starts_with("not_found");
        if found {
            eprintln!("[ENGINE] {result}");
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        } else {
            eprintln!("[ENGINE] wom_id not found: '{wom_id}'");
        }
        Ok(found)
    }

    /// Focus an element by its WOM ID.
    pub async fn focus_by_wom_id(&self, wom_id: &str) -> Result<bool, Box<dyn std::error::Error>> {
        let js = format!(
            r#"
            (() => {{
                const _doc = {active_doc};
                const el = _doc.querySelector('[data-wom-id="{wom_id}"]');
                if (!el) return 'not_found';
                el.scrollIntoViewIfNeeded();
                el.focus();
                el.click();
                const label = el.placeholder || el.getAttribute('aria-label') || el.tagName;
                return 'focused[' + '{wom_id}' + ']: ' + label;
            }})()
            "#,
            active_doc = Self::ACTIVE_DOC_JS,
            wom_id = wom_id
        );
        let result = self.eval_string(&js).await?;
        let found = !result.starts_with("not_found");
        if found {
            eprintln!("[ENGINE] {result}");
        } else {
            eprintln!("[ENGINE] wom_id not found for focus: '{wom_id}'");
        }
        Ok(found)
    }

    /// Capture outerHTML with SPA stability wait.
    async fn capture_html(&self) -> Result<String, Box<dyn std::error::Error>> {
        // Frame-aware: capture HTML from the active document (frame with most content).
        // In a frameset, the top document is just <frameset> tags — we want the frame content.
        let js = format!(
            r#"
            new Promise((resolve) => {{
                const _doc = {active_doc};
                let prev = -1, stable = 0, elapsed = 0;
                const check = () => {{
                    const count = _doc.querySelectorAll('a,button,input,select,textarea,[role]').length;
                    const textLen = (_doc.body ? _doc.body.innerText.length : 0);
                    elapsed += 300;
                    if ((count > 3 && count === prev && textLen > 30) || stable >= 2 || elapsed > 5000) {{
                        resolve(_doc.documentElement.outerHTML);
                    }} else {{
                        if (count === prev) {{ stable++; }} else {{ stable = 0; }}
                        prev = count;
                        setTimeout(check, 300);
                    }}
                }};
                setTimeout(check, 500);
            }})
            "#,
            active_doc = Self::ACTIVE_DOC_JS,
        );
        self.eval_string(&js).await
    }

    /// Raw semantic dump.
    pub async fn see_raw(&self) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let t0 = Instant::now();
        let html = self.capture_html().await?;

        let dom = parse_document(RcDom::default(), Default::default())
            .from_utf8()
            .read_from(&mut html.as_bytes())?;

        let mut output = Vec::new();
        semantic::walk(&dom.document, 0, &mut output);

        let mut stats = semantic::PageStats::new();
        semantic::count_nodes(&dom.document, &mut stats);

        eprintln!(
            "[ENGINE] see: {} lines, {:.1}KB | {}L {}B {}F {}H | {}ms",
            output.len(),
            output.join("\n").len() as f64 / 1024.0,
            stats.links, stats.buttons, stats.forms, stats.headings,
            t0.elapsed().as_millis(),
        );

        Ok(output)
    }

    /// AI Vision — semantic dump + page classification + available actions.
    pub async fn see(&self) -> Result<vision::PageView, Box<dyn std::error::Error>> {
        let t0 = Instant::now();
        let html = self.capture_html().await?;
        let url = self.current_url().await.unwrap_or_default();
        let title = self.current_title().await.unwrap_or_default();

        let dom = parse_document(RcDom::default(), Default::default())
            .from_utf8()
            .read_from(&mut html.as_bytes())?;

        let view = vision::analyze(&dom.document, &url, &title);

        eprintln!(
            "[VISION] {} | {} | {} lines | {} actions | {}ms",
            view.page_type, title, view.content.len(), view.actions.len(),
            t0.elapsed().as_millis(),
        );

        Ok(view)
    }

    /// Lightweight page view — extracts what a user SEES via JS.
    /// No HTML parsing, no WOM, no html5ever. Just runs JS in the active frame
    /// and returns structured text: title, URL, interactive elements, visible text.
    /// This is what agents should use 90% of the time.
    pub async fn see_page(&self) -> Result<String, Box<dyn std::error::Error>> {
        let t0 = Instant::now();
        let js = format!(
            r#"
            (() => {{
                const _doc = {active_doc};
                const url = window.location.href;
                const title = _doc.title || document.title || '';

                // Collect interactive elements (visible only)
                const els = [];
                _doc.querySelectorAll('input, textarea, select, button, a, [role="button"], [role="link"], summary, [onclick]').forEach(el => {{
                    // Skip hidden
                    if (el.offsetParent === null && el.tagName !== 'BODY' && el.tagName !== 'HTML'
                        && !el.closest('frame, iframe')) return;

                    const tag = el.tagName.toLowerCase();
                    const text = (el.textContent || '').trim().substring(0, 80);
                    const placeholder = el.placeholder || '';
                    const name = el.name || el.id || '';
                    const type = el.type || '';
                    const value = el.value || '';
                    const ariaLabel = el.getAttribute('aria-label') || '';
                    const title = el.title || '';
                    const href = el.href || '';

                    // Build label
                    let label = ariaLabel || placeholder || text || name || title;
                    if (!label && tag === 'input') label = type;
                    if (!label) return;
                    label = label.replace(/\s+/g, ' ').trim().substring(0, 60);

                    let entry = '';
                    if (tag === 'input' || tag === 'textarea') {{
                        const t = (type === 'password') ? 'password' : (type === 'hidden') ? null : 'input';
                        if (!t) return;
                        const cur = value ? ' = "' + value.substring(0, 30) + '"' : '';
                        entry = '[' + t + ' "' + label + '"]' + cur;
                    }} else if (tag === 'select') {{
                        const opts = Array.from(el.options).slice(0, 8).map(o => o.text.trim().substring(0, 30));
                        const cur = el.selectedIndex >= 0 ? el.options[el.selectedIndex].text.trim() : '';
                        entry = '[select "' + label + '"] selected="' + cur + '" options: ' + opts.join(' | ');
                    }} else if (tag === 'button' || el.getAttribute('role') === 'button') {{
                        entry = '[button "' + label + '"]';
                    }} else if (tag === 'a') {{
                        if (!text || text.length < 2) return;
                        entry = '[link "' + label + '"]';
                    }} else if (tag === 'summary') {{
                        entry = '[toggle "' + label + '"]';
                    }} else {{
                        entry = '[action "' + label + '"]';
                    }}

                    els.push(entry);
                }});

                // Deduplicate
                const seen = new Set();
                const unique = els.filter(e => {{
                    if (seen.has(e)) return false;
                    seen.add(e);
                    return true;
                }});

                // Extract visible text (headings + paragraphs, skip nav/footer noise)
                const textParts = [];
                let inNav = false;
                _doc.querySelectorAll('h1,h2,h3,h4,h5,h6,p,td,th,li,label,span,div').forEach(el => {{
                    // Skip if inside nav/header/footer
                    const parent = el.closest('nav, header, footer, [role="navigation"], [role="banner"]');
                    if (parent) return;

                    // Skip hidden
                    if (el.offsetParent === null && el.tagName !== 'BODY') return;

                    const tag = el.tagName.toLowerCase();
                    let t = '';

                    // For headings, get direct text
                    if (tag.startsWith('h')) {{
                        t = el.textContent.trim();
                        if (t) t = '#'.repeat(parseInt(tag[1])) + ' ' + t;
                    }} else if (tag === 'p' || tag === 'label') {{
                        t = el.textContent.trim();
                    }} else if (tag === 'td' || tag === 'th') {{
                        t = el.textContent.trim();
                    }} else if (tag === 'li') {{
                        // Only direct text, not nested lists
                        const directText = Array.from(el.childNodes)
                            .filter(n => n.nodeType === 3)
                            .map(n => n.textContent.trim())
                            .join(' ');
                        if (directText) t = '- ' + directText;
                    }} else if (tag === 'div' || tag === 'span') {{
                        // Only leaf divs/spans with meaningful text
                        if (el.children.length === 0) {{
                            t = el.textContent.trim();
                        }}
                    }}

                    if (t && t.length > 2 && t.length < 500) {{
                        textParts.push(t.substring(0, 200));
                    }}
                }});

                // Deduplicate text (many nested elements repeat text)
                const seenText = new Set();
                const uniqueText = textParts.filter(t => {{
                    const key = t.substring(0, 50).toLowerCase();
                    if (seenText.has(key)) return false;
                    seenText.add(key);
                    return true;
                }});

                // Build output
                const lines = [];
                lines.push('Page: ' + title);
                lines.push('URL: ' + url);
                lines.push('');

                if (unique.length > 0) {{
                    lines.push('Interactive:');
                    unique.forEach(e => lines.push('  ' + e));
                    lines.push('');
                }}

                if (uniqueText.length > 0) {{
                    lines.push('Content:');
                    uniqueText.slice(0, 80).forEach(t => lines.push('  ' + t));
                    if (uniqueText.length > 80) {{
                        lines.push('  ... (' + (uniqueText.length - 80) + ' more)');
                    }}
                }}

                return lines.join('\n');
            }})()
            "#,
            active_doc = Self::ACTIVE_DOC_JS,
        );

        let result = self.eval_string(&js).await?;

        eprintln!(
            "[SEE] {}chars | {}ms",
            result.len(),
            t0.elapsed().as_millis(),
        );

        Ok(result)
    }

    /// WOM output — structured for AI agents.
    pub async fn see_wom(&self, revision: u64) -> Result<wom::WomDocument, Box<dyn std::error::Error>> {
        let t0 = Instant::now();
        let html = self.capture_html().await?;
        eprintln!("[WOM] captured {}KB ({}ms)", html.len() / 1024, t0.elapsed().as_millis());

        let url = self.current_url().await.unwrap_or_default();
        let title = self.current_title().await.unwrap_or_default();
        let html_bytes = html.len();

        let dom = parse_document(RcDom::default(), Default::default())
            .from_utf8()
            .read_from(&mut html.as_bytes())?;

        // Tag DOM elements FIRST, then capture HTML so WOM builder can read the IDs
        let tagged = self.tag_dom_elements().await.unwrap_or(0);

        // Re-capture HTML with tags embedded
        let html = self.capture_html().await?;
        let html_bytes = html.len();
        let dom = parse_document(RcDom::default(), Default::default())
            .from_utf8()
            .read_from(&mut html.as_bytes())?;

        let doc = wom::build(&dom.document, &url, &title, html_bytes, "engine", revision);

        eprintln!(
            "[WOM] {} | {} nodes | {} actions | {} tagged | {:.1}x compression | {}ms",
            doc.page.page_class,
            doc.nodes.len(),
            doc.actions.len(),
            tagged,
            doc.compression.compression_ratio,
            t0.elapsed().as_millis(),
        );

        Ok(doc)
    }

    // ─── Interaction ───

    pub async fn click(&self, text: &str) -> Result<bool, Box<dyn std::error::Error>> {
        // Smart click: collect ALL candidates, score them, click the best.
        // Frame-aware: searches inside the active frame if in a frameset.
        let js = format!(
            r#"
            (() => {{
                const _doc = {active_doc};
                const target = {target_json};
                const lower = target.toLowerCase();

                // Detect if an element is inside navigation/header
                function inNav(el) {{
                    let p = el;
                    while (p) {{
                        const tag = (p.tagName || '').toLowerCase();
                        const role = (p.getAttribute && p.getAttribute('role') || '').toLowerCase();
                        if (tag === 'nav' || tag === 'header' || role === 'navigation' || role === 'banner') return true;
                        p = p.parentElement;
                    }}
                    return false;
                }}

                // Collect candidates with scores
                const candidates = [];
                for (const el of _doc.querySelectorAll('button, a, [role="button"], [role="link"], input[type="submit"], summary, [aria-label], [title]')) {{
                    // Get the best text representation
                    const texts = [
                        (el.textContent || '').trim(),
                        el.value || '',
                        el.getAttribute('aria-label') || '',
                        el.getAttribute('title') || '',
                    ];
                    // Find the text that best matches our target
                    let bestText = '';
                    let bestScore = -1;
                    for (const t of texts) {{
                        if (!t) continue;
                        const tl = t.toLowerCase();
                        if (!tl.includes(lower)) continue;

                        let score = 0;
                        // Exact match (text IS the target)
                        if (tl === lower) score += 100;
                        // Tight match (target is most of the text)
                        else if (t.length < lower.length * 3) score += 60;
                        // Starts with target
                        else if (tl.startsWith(lower)) score += 40;
                        // Loose partial
                        else score += 10;

                        if (score > bestScore) {{
                            bestScore = score;
                            bestText = t;
                        }}
                    }}

                    if (bestScore < 0) continue;

                    // Bonus: actionable elements (buttons) over links
                    const tag = (el.tagName || '').toLowerCase();
                    const role = (el.getAttribute('role') || '').toLowerCase();
                    if (tag === 'button' || role === 'button' || tag === 'input') bestScore += 15;

                    // Penalty: elements in nav/header (less likely to be the target action)
                    if (inNav(el)) bestScore -= 30;

                    // Penalty: hidden elements
                    if (el.offsetParent === null && tag !== 'body') bestScore -= 50;

                    candidates.push({{el, score: bestScore, text: bestText}});
                }}

                if (candidates.length === 0) return 'not_found';

                // Sort by score descending, pick the best
                candidates.sort((a, b) => b.score - a.score);
                const best = candidates[0];
                best.el.scrollIntoViewIfNeeded();
                best.el.click();

                const alt = candidates.length > 1 ? ' ('+candidates.length+' candidates, 2nd: '+candidates[1].text.substring(0,40)+')' : '';
                return 'clicked[' + best.score + ']: ' + best.text.substring(0, 60) + alt;
            }})()
            "#,
            active_doc = Self::ACTIVE_DOC_JS,
            target_json = serde_json::to_string(text)?
        );

        let result = self.eval_string(&js).await?;
        let found = !result.starts_with("not_found");
        if found {
            eprintln!("[ENGINE] {result}");
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        } else {
            eprintln!("[ENGINE] click not found: '{text}'");
        }
        Ok(found)
    }

    pub async fn focus(&self, text: &str) -> Result<bool, Box<dyn std::error::Error>> {
        let js = format!(
            r#"
            (() => {{
                const _doc = {active_doc};
                const target = {target_json};
                const lower = target.toLowerCase();

                for (const el of _doc.querySelectorAll('input, textarea, [contenteditable="true"], [role="textbox"]')) {{
                    const p = (el.placeholder || el.getAttribute('aria-label') || el.getAttribute('data-placeholder') || '').toLowerCase();
                    if (p.includes(lower) || lower === '') {{
                        el.scrollIntoViewIfNeeded();
                        el.focus();
                        el.click();
                        return 'focused: ' + (el.placeholder || el.getAttribute('aria-label') || el.tagName);
                    }}
                }}

                for (const el of _doc.querySelectorAll('[contenteditable="true"]')) {{
                    el.scrollIntoViewIfNeeded();
                    el.focus();
                    el.click();
                    return 'focused-contenteditable';
                }}

                return 'not_found';
            }})()
            "#,
            active_doc = Self::ACTIVE_DOC_JS,
            target_json = serde_json::to_string(text)?
        );

        let result = self.eval_string(&js).await?;
        let found = !result.starts_with("not_found");
        if found {
            eprintln!("[ENGINE] {result}");
        } else {
            eprintln!("[ENGINE] focus not found: '{text}'");
        }
        Ok(found)
    }

    pub async fn type_text(&self, text: &str) -> Result<(), Box<dyn std::error::Error>> {
        for ch in text.chars() {
            let s = ch.to_string();
            self.cdp
                .send_to(
                    &self.page_session_id,
                    "Input.dispatchKeyEvent",
                    Some(json!({
                        "type": "keyDown",
                        "text": s,
                        "key": s,
                    })),
                )
                .await?;
            self.cdp
                .send_to(
                    &self.page_session_id,
                    "Input.dispatchKeyEvent",
                    Some(json!({
                        "type": "keyUp",
                        "key": s,
                    })),
                )
                .await?;
            tokio::time::sleep(std::time::Duration::from_millis(15)).await;
        }
        eprintln!("[ENGINE] typed {} chars", text.len());
        Ok(())
    }

    pub async fn press(&self, key: &str) -> Result<(), Box<dyn std::error::Error>> {
        let (key_val, code, vkc) = match key {
            "Enter" | "enter" => ("Enter", "Enter", 13),
            "Tab" | "tab" => ("Tab", "Tab", 9),
            "Escape" | "escape" | "Esc" => ("Escape", "Escape", 27),
            "Backspace" | "backspace" => ("Backspace", "Backspace", 8),
            "Space" | "space" => (" ", "Space", 32),
            "ArrowUp" | "up" => ("ArrowUp", "ArrowUp", 38),
            "ArrowDown" | "down" => ("ArrowDown", "ArrowDown", 40),
            "ArrowLeft" | "left" => ("ArrowLeft", "ArrowLeft", 37),
            "ArrowRight" | "right" => ("ArrowRight", "ArrowRight", 39),
            _ => (key, key, 0),
        };

        self.cdp
            .send_to(
                &self.page_session_id,
                "Input.dispatchKeyEvent",
                Some(json!({
                    "type": "keyDown",
                    "key": key_val,
                    "code": code,
                    "windowsVirtualKeyCode": vkc,
                })),
            )
            .await?;
        self.cdp
            .send_to(
                &self.page_session_id,
                "Input.dispatchKeyEvent",
                Some(json!({
                    "type": "keyUp",
                    "key": key_val,
                    "code": code,
                    "windowsVirtualKeyCode": vkc,
                })),
            )
            .await?;

        eprintln!("[ENGINE] pressed {key}");
        Ok(())
    }

    pub async fn scroll(&self, direction: &str) -> Result<(), Box<dyn std::error::Error>> {
        let delta = match direction {
            "down" | "d" => 400,
            "up" | "u" => -400,
            "bottom" => 99999,
            "top" => -99999,
            _ => 400,
        };
        self.eval_string(&format!("window.scrollBy(0, {delta})"))
            .await?;
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        eprintln!("[ENGINE] scroll {direction}");
        Ok(())
    }

    pub async fn screenshot(&self) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let result = self
            .cdp
            .send_to(
                &self.page_session_id,
                "Page.captureScreenshot",
                Some(json!({"format": "jpeg", "quality": 40})),
            )
            .await?;

        let b64 = result["data"].as_str().ok_or("No screenshot data")?;
        use base64::Engine;
        let data = base64::engine::general_purpose::STANDARD.decode(b64)?;
        eprintln!("[ENGINE] screenshot: {}KB", data.len() / 1024);
        Ok(data)
    }

    pub async fn eval(&self, js: &str) -> Result<String, Box<dyn std::error::Error>> {
        self.eval_string(js).await
    }

    // ─── Tabs / Pages ───

    pub async fn pages(&self) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let result = self.cdp.send("Target.getTargets", None).await?;
        let targets = result["targetInfos"].as_array().ok_or("No targets")?;

        let mut pages = Vec::new();
        for t in targets {
            if t["type"].as_str() == Some("page") {
                let title = t["title"].as_str().unwrap_or("");
                let url = t["url"].as_str().unwrap_or("");
                pages.push(format!("{title} | {url}"));
            }
        }
        Ok(pages)
    }

    pub async fn switch_tab(&mut self, index: usize) -> Result<(), Box<dyn std::error::Error>> {
        let result = self.cdp.send("Target.getTargets", None).await?;
        let targets = result["targetInfos"].as_array().ok_or("No targets")?;

        let page_targets: Vec<&Value> = targets
            .iter()
            .filter(|t| t["type"].as_str() == Some("page"))
            .collect();

        if index >= page_targets.len() {
            return Err(format!("Tab {index} not found (have {})", page_targets.len()).into());
        }

        let target = page_targets[index];
        let new_target_id = target["targetId"]
            .as_str()
            .ok_or("No targetId")?
            .to_string();

        // Attach to the new target
        let result = self
            .cdp
            .send(
                "Target.attachToTarget",
                Some(json!({"targetId": new_target_id, "flatten": true})),
            )
            .await?;
        let session_id = result["sessionId"]
            .as_str()
            .ok_or("No sessionId")?
            .to_string();

        self.cdp
            .send_to(&session_id, "Page.enable", None)
            .await?;
        self.cdp
            .send_to(&session_id, "Runtime.enable", None)
            .await?;

        self.target_id = new_target_id;
        self.page_session_id = session_id;
        self.last_url = target["url"].as_str().unwrap_or("").to_string();

        eprintln!("[ENGINE] Switched to tab {index}: {}", self.last_url);
        Ok(())
    }

    // ─── Dialog handling ───

    pub async fn setup_dialog_handler(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.eval_string(r#"
            window.__neo_dialogs = [];
            window.alert = function(msg) { window.__neo_dialogs.push({type:'alert',message:msg}); };
            window.confirm = function(msg) { window.__neo_dialogs.push({type:'confirm',message:msg}); return true; };
            window.prompt = function(msg,def) { window.__neo_dialogs.push({type:'prompt',message:msg}); return def || ''; };
            window.onbeforeunload = null;
            'ok'
        "#).await?;
        eprintln!("[ENGINE] dialog handler installed");
        Ok(())
    }

    pub async fn get_dialogs(&self) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
        let result = self.eval_string(r#"
            (() => {
                const d = window.__neo_dialogs || [];
                window.__neo_dialogs = [];
                return JSON.stringify(d);
            })()
        "#).await?;
        let dialogs: Vec<Value> = serde_json::from_str(&result).unwrap_or_default();
        Ok(dialogs)
    }

    // ─── Bulk form fill ───

    pub async fn fill_form(
        &self,
        fields: &[(String, String)],
    ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let mut results = Vec::new();
        for (target, value) in fields {
            let focused = self.focus(target).await?;
            if focused {
                self.eval_string(
                    "document.activeElement && (document.activeElement.value = '')",
                )
                .await?;
                self.type_text(value).await?;
                results.push(format!(
                    "filled: {target} = {}",
                    if value.len() > 20 { &value[..20] } else { value }
                ));
            } else {
                results.push(format!("not_found: {target}"));
            }
        }
        eprintln!("[ENGINE] fill_form: {} fields", fields.len());
        Ok(results)
    }

    // ─── Hover ───

    pub async fn hover(&self, text: &str) -> Result<bool, Box<dyn std::error::Error>> {
        let js = format!(
            r#"
            (() => {{
                const _doc = {active_doc};
                const target = {target_json};
                const lower = target.toLowerCase();
                for (const el of _doc.querySelectorAll('a, button, [role="button"], [role="link"], [role="menuitem"]')) {{
                    const t = (el.textContent || el.getAttribute('aria-label') || '').trim();
                    if (t.toLowerCase().includes(lower)) {{
                        el.scrollIntoViewIfNeeded();
                        const rect = el.getBoundingClientRect();
                        return JSON.stringify({{x: rect.x + rect.width/2, y: rect.y + rect.height/2, text: t.substring(0,60)}});
                    }}
                }}
                return 'not_found';
            }})()
            "#,
            active_doc = Self::ACTIVE_DOC_JS,
            target_json = serde_json::to_string(text)?
        );

        let result = self.eval_string(&js).await?;
        if result == "not_found" {
            eprintln!("[ENGINE] hover not found: '{text}'");
            return Ok(false);
        }

        let coords: Value = serde_json::from_str(&result)?;
        let x = coords["x"].as_f64().unwrap_or(0.0);
        let y = coords["y"].as_f64().unwrap_or(0.0);

        self.cdp
            .send_to(
                &self.page_session_id,
                "Input.dispatchMouseEvent",
                Some(json!({
                    "type": "mouseMoved",
                    "x": x,
                    "y": y,
                })),
            )
            .await?;

        eprintln!("[ENGINE] hovered: {}", coords["text"].as_str().unwrap_or(""));
        Ok(true)
    }

    // ─── Select option ───

    pub async fn select_option(
        &self,
        target: &str,
        value: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let js = format!(
            r#"
            (() => {{
                const _doc = {active_doc};
                const target = {target_json};
                const value = {value_json};
                const lower = target.toLowerCase();
                for (const el of _doc.querySelectorAll('select')) {{
                    const label = (el.getAttribute('aria-label') || el.name || el.id || '').toLowerCase();
                    if (label.includes(lower) || lower === '') {{
                        el.value = value;
                        el.dispatchEvent(new Event('change', {{bubbles: true}}));
                        return 'selected: ' + value;
                    }}
                }}
                return 'not_found';
            }})()
            "#,
            active_doc = Self::ACTIVE_DOC_JS,
            target_json = serde_json::to_string(target)?,
            value_json = serde_json::to_string(value)?
        );

        let result = self.eval_string(&js).await?;
        let found = !result.starts_with("not_found");
        if found {
            eprintln!("[ENGINE] {result}");
        } else {
            eprintln!("[ENGINE] select not found: '{target}'");
        }
        Ok(found)
    }

    // ─── Network capture ───

    pub async fn start_network_capture(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.eval_string(r#"
            window.__neo_net = [];
            const origFetch = window.fetch;
            window.fetch = function(...args) {
                const url = typeof args[0] === 'string' ? args[0] : args[0]?.url || '';
                const method = args[1]?.method || 'GET';
                const entry = {type:'fetch', method, url, ts: Date.now()};
                window.__neo_net.push(entry);
                return origFetch.apply(this, args).then(r => { entry.status = r.status; return r; });
            };
            const origXHR = XMLHttpRequest.prototype.open;
            XMLHttpRequest.prototype.open = function(method, url) {
                this.__neo = {type:'xhr', method, url, ts: Date.now()};
                window.__neo_net.push(this.__neo);
                return origXHR.apply(this, arguments);
            };
            const origSend = XMLHttpRequest.prototype.send;
            XMLHttpRequest.prototype.send = function() {
                this.addEventListener('load', () => { if(this.__neo) this.__neo.status = this.status; });
                return origSend.apply(this, arguments);
            };
            'ok'
        "#).await?;
        eprintln!("[ENGINE] network capture started");
        Ok(())
    }

    pub async fn read_network(&self) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
        let result = self.eval_string(r#"
            (() => {
                const d = window.__neo_net || [];
                window.__neo_net = [];
                return JSON.stringify(d);
            })()
        "#).await?;
        let reqs: Vec<Value> = serde_json::from_str(&result).unwrap_or_default();
        Ok(reqs)
    }

    // ─── Console capture ───

    pub async fn start_console_capture(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.eval_string(r#"
            window.__neo_console = [];
            for (const level of ['log','warn','error','info','debug']) {
                const orig = console[level];
                console[level] = function(...args) {
                    window.__neo_console.push({level, message: args.map(a=>String(a)).join(' '), ts: Date.now()});
                    return orig.apply(console, args);
                };
            }
            window.addEventListener('error', (e) => {
                window.__neo_console.push({level:'exception', message: e.message, ts: Date.now()});
            });
            'ok'
        "#).await?;
        eprintln!("[ENGINE] console capture started");
        Ok(())
    }

    pub async fn read_console(&self) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
        let result = self.eval_string(r#"
            (() => {
                const d = window.__neo_console || [];
                window.__neo_console = [];
                return JSON.stringify(d);
            })()
        "#).await?;
        let msgs: Vec<Value> = serde_json::from_str(&result).unwrap_or_default();
        Ok(msgs)
    }

    // ─── Cookie / Storage extraction ───

    pub async fn get_all_cookies(&self) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
        let result = self
            .cdp
            .send_to(&self.page_session_id, "Network.getAllCookies", None)
            .await?;
        let cookies = result["cookies"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        Ok(cookies)
    }

    pub async fn get_local_storage(
        &self,
    ) -> Result<std::collections::HashMap<String, String>, Box<dyn std::error::Error>> {
        let result = self.eval_string(
            "JSON.stringify(Object.fromEntries(Object.keys(localStorage).map(k=>[k,localStorage[k]])))"
        ).await?;
        let data: std::collections::HashMap<String, String> =
            serde_json::from_str(&result).unwrap_or_default();
        Ok(data)
    }

    pub async fn set_local_storage(
        &self,
        data: &std::collections::HashMap<String, String>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        for (key, value) in data {
            let js = format!(
                "localStorage.setItem({}, {})",
                serde_json::to_string(key)?,
                serde_json::to_string(value)?
            );
            self.eval_string(&js).await?;
        }
        eprintln!("[ENGINE] set {} localStorage items", data.len());
        Ok(())
    }

    // ─── Lifecycle ───

    pub async fn wait(&self, secs: f64) {
        tokio::time::sleep(std::time::Duration::from_secs_f64(secs)).await;
    }

    pub async fn close(mut self) -> Result<(), Box<dyn std::error::Error>> {
        // Close the browser
        let _ = self.cdp.send("Browser.close", None).await;

        // Kill Chrome process if we launched it
        if let Some(ref mut child) = self.chrome_process {
            let _ = child.kill().await;
        }

        eprintln!("[ENGINE] Closed");
        Ok(())
    }
}
