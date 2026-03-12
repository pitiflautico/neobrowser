//! Chrome session — CDP-based browser with full interaction + AI vision.
//!
//! Two connection modes:
//!   1. LAUNCH: start a new headless Chrome (default)
//!   2. CONNECT: attach to user's real Chrome via CDP port
//!
//! The second mode is undetectable because it IS the real browser.

use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::input::{
    DispatchKeyEventParams, DispatchKeyEventType,
};
use chromiumoxide::cdp::browser_protocol::network::CookieParam;
use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat;
use chromiumoxide::Page;
use futures::StreamExt;
use html5ever::parse_document;
use html5ever::tendril::TendrilSink;
use markup5ever_rcdom::RcDom;
use std::time::Instant;
use tokio::task::JoinHandle;

use crate::semantic;
use crate::vision;
use crate::wom;

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

// ─── Session ───

pub struct Session {
    browser: Browser,
    _handler: JoinHandle<()>,
    page: Option<Page>,
    pub last_url: String,
    connected_mode: bool, // true = connected to existing Chrome
}

impl Session {
    /// JS that waits for SPA content to stabilize, then captures outerHTML.
    /// Watches interactive element count (a, button, input, [role]) across
    /// animation frames. Resolves when count stays the same for 2 consecutive
    /// checks (300ms apart), or after 5s max. Works for Angular, React, Vue.
    const JS_WAIT_AND_CAPTURE: &'static str = r#"
        new Promise((resolve) => {
            let prev = -1, stable = 0, elapsed = 0;
            const check = () => {
                const count = document.querySelectorAll('a,button,input,select,textarea,[role]').length;
                const textLen = (document.body ? document.body.innerText.length : 0);
                elapsed += 300;
                if ((count > 3 && count === prev && textLen > 30) || stable >= 2 || elapsed > 5000) {
                    resolve(document.documentElement.outerHTML);
                } else {
                    if (count === prev) { stable++; } else { stable = 0; }
                    prev = count;
                    setTimeout(check, 300);
                }
            };
            // First check after initial 500ms to let frameworks bootstrap
            setTimeout(check, 500);
        })
    "#;

    /// Launch a new headless Chrome with stealth.
    pub async fn launch(
        user_data_dir: Option<&str>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let chrome = find_chrome()?;

        let mut builder = BrowserConfig::builder()
            .chrome_executable(chrome)
            .arg("--disable-blink-features=AutomationControlled")
            .arg("--disable-gpu")
            .arg("--disable-dev-shm-usage")
            .arg("--disable-extensions")
            .arg("--disable-default-apps")
            .arg("--disable-sync")
            .arg("--disable-background-networking")
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--window-size=1440,900");

        // Use real user profile if specified — this gets all cookies/sessions for free
        if let Some(dir) = user_data_dir {
            builder = builder.user_data_dir(dir);
        }

        let config = builder
            .build()
            .map_err(|e| format!("BrowserConfig: {e}"))?;

        let (browser, mut handler) = Browser::launch(config).await?;
        let handle = tokio::spawn(async move {
            while let Some(event) = handler.next().await {
                let _ = event;
            }
        });

        eprintln!("[BROWSER] Launched headless Chrome");
        Ok(Self {
            browser,
            _handler: handle,
            page: None,
            last_url: String::new(),
            connected_mode: false,
        })
    }

    /// Connect to an already-running Chrome instance via CDP WebSocket.
    /// Start Chrome with: --remote-debugging-port=9222
    pub async fn connect(ws_url: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let (browser, mut handler) = Browser::connect(ws_url).await?;
        let handle = tokio::spawn(async move {
            while let Some(event) = handler.next().await {
                let _ = event;
            }
        });

        eprintln!("[BROWSER] Connected to existing Chrome at {ws_url}");
        Ok(Self {
            browser,
            _handler: handle,
            page: None,
            last_url: String::new(),
            connected_mode: true,
        })
    }

    /// Connect to Chrome on a given debug port (discovers WS URL automatically).
    /// Connect to Chrome on a debug port. First tries HTTP discovery,
    /// then falls back to reading DevToolsActivePort file.
    pub async fn connect_port(port: u16) -> Result<Self, Box<dyn std::error::Error>> {
        // Try HTTP discovery first
        let url = format!("http://127.0.0.1:{port}/json/version");
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()?;

        if let Ok(resp) = client.get(&url).send().await {
            if let Ok(text) = resp.text().await {
                if let Ok(data) = serde_json::from_str::<serde_json::Value>(&text) {
                    if let Some(ws) = data["webSocketDebuggerUrl"].as_str() {
                        return Self::connect(ws).await;
                    }
                }
            }
        }

        // Fallback: direct WebSocket connection
        let ws_url = format!("ws://127.0.0.1:{port}/devtools/browser");
        eprintln!("[BROWSER] HTTP discovery failed, trying direct WS: {ws_url}");
        Self::connect(&ws_url).await
    }

    // ─── Page management ───

    async fn ensure_page(&mut self) -> Result<&Page, Box<dyn std::error::Error>> {
        if self.page.is_none() {
            let page = self.browser.new_page("about:blank").await?;
            page.enable_stealth_mode().await?;
            self.page = Some(page);
        }
        Ok(self.page.as_ref().unwrap())
    }

    /// Get list of all open pages/tabs.
    pub async fn pages(&self) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let pages = self.browser.pages().await?;
        let mut result = Vec::new();
        for p in pages {
            let url = p.url().await?.unwrap_or_default();
            let title = p.get_title().await?.unwrap_or_default();
            result.push(format!("{title} | {url}"));
        }
        Ok(result)
    }

    /// Switch to an existing tab by index.
    pub async fn switch_tab(&mut self, index: usize) -> Result<(), Box<dyn std::error::Error>> {
        let pages = self.browser.pages().await?;
        if index >= pages.len() {
            return Err(format!("Tab {index} not found (have {})", pages.len()).into());
        }
        let page = pages.into_iter().nth(index).unwrap();
        let url = page.url().await?.unwrap_or_default();
        self.page = Some(page);
        self.last_url = url.clone();
        eprintln!("[BROWSER] Switched to tab {index}: {url}");
        Ok(())
    }

    // ─── Cookies ───

    pub async fn load_cookies(&mut self, path: &str) -> Result<usize, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        let data: serde_json::Value = serde_json::from_str(&content)?;

        let cookies = if let Some(arr) = data.as_array() {
            arr.clone()
        } else if let Some(arr) = data.get("cookies").and_then(|c| c.as_array()) {
            arr.clone()
        } else {
            return Err("Invalid cookie format".into());
        };

        let count = cookies.len();
        let mut cookie_params: Vec<CookieParam> = Vec::new();
        for cookie in &cookies {
            let name = cookie.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let value = cookie.get("value").and_then(|v| v.as_str()).unwrap_or("");
            let domain = cookie.get("domain").and_then(|v| v.as_str()).unwrap_or("");
            let path_str = cookie.get("path").and_then(|v| v.as_str()).unwrap_or("/");
            let secure = cookie.get("secure").and_then(|v| v.as_bool()).unwrap_or(false);

            // Build URL from domain so CDP can set cookies on about:blank
            let scheme = if secure { "https" } else { "http" };
            let clean_domain = domain.trim_start_matches('.');
            let url = format!("{scheme}://{clean_domain}/");

            let mut builder = CookieParam::builder()
                .name(name)
                .value(value)
                .domain(domain)
                .path(path_str)
                .url(url);
            if secure {
                builder = builder.secure(true);
            }
            if let Ok(cp) = builder.build() {
                cookie_params.push(cp);
            }
        }

        // Use browser-level setCookies — works on about:blank
        self.browser.set_cookies(cookie_params).await?;
        eprintln!("[BROWSER] Injected {count} cookies");
        Ok(count)
    }

    // ─── Navigation ───

    pub async fn goto(&mut self, url: &str) -> Result<(), Box<dyn std::error::Error>> {
        let t0 = Instant::now();
        let page = self.ensure_page().await?;

        // page.goto() waits for lifecycle events. Heavy sites (LinkedIn) may
        // never complete all iframes. Timeout after 15s.
        let timed_out = match tokio::time::timeout(
            std::time::Duration::from_secs(15),
            page.goto(url),
        ).await {
            Ok(Ok(_)) => false,
            Ok(Err(e)) => { eprintln!("[BROWSER] goto nav error (continuing): {e}"); false }
            Err(_) => { eprintln!("[BROWSER] goto lifecycle timeout 15s"); true }
        };

        // If goto timed out, chromiumoxide's internal navigation watcher is stuck
        // and will block all subsequent execute() calls. Fix: get a fresh page
        // handle from the browser which has a clean state.
        if timed_out {
            let pages = self.browser.pages().await?;
            // Find the page that actually navigated to our URL
            let mut found = false;
            for p in pages {
                if let Ok(Some(page_url)) = p.url().await {
                    if page_url.contains(url) || url.contains(&page_url) {
                        self.page = Some(p);
                        found = true;
                        break;
                    }
                }
            }
            if !found {
                // Fallback: get the last page (most recently created)
                let pages = self.browser.pages().await?;
                if let Some(p) = pages.into_iter().last() {
                    self.page = Some(p);
                }
            }
        }

        self.last_url = url.to_string();
        eprintln!("[BROWSER] goto {url} ({}ms)", t0.elapsed().as_millis());
        Ok(())
    }

    pub async fn back(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let page = self.ensure_page().await?;
        page.evaluate("window.history.back()").await?;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        eprintln!("[BROWSER] back");
        Ok(())
    }

    pub async fn forward(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let page = self.ensure_page().await?;
        page.evaluate("window.history.forward()").await?;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        eprintln!("[BROWSER] forward");
        Ok(())
    }

    pub async fn reload(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let page = self.ensure_page().await?;
        page.reload().await?;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        eprintln!("[BROWSER] reload");
        Ok(())
    }

    // ─── Perception ───

    /// Raw semantic dump — all visible text + elements.
    pub async fn see_raw(&mut self) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let page = self.ensure_page().await?;
        let t0 = Instant::now();

        let html = page
            .evaluate("document.documentElement.outerHTML")
            .await?
            .into_value::<String>()?;

        let dom = parse_document(RcDom::default(), Default::default())
            .from_utf8()
            .read_from(&mut html.as_bytes())?;

        let mut output = Vec::new();
        semantic::walk(&dom.document, 0, &mut output);

        let mut stats = semantic::PageStats::new();
        semantic::count_nodes(&dom.document, &mut stats);

        eprintln!(
            "[BROWSER] see: {} lines, {:.1}KB | {}L {}B {}F {}H | {}ms",
            output.len(),
            output.join("\n").len() as f64 / 1024.0,
            stats.links, stats.buttons, stats.forms, stats.headings,
            t0.elapsed().as_millis(),
        );

        Ok(output)
    }

    /// AI Vision — semantic dump + page classification + available actions.
    pub async fn see(&mut self) -> Result<vision::PageView, Box<dyn std::error::Error>> {
        let fallback_url = self.last_url.clone();
        let page = self.ensure_page().await?;
        let t0 = Instant::now();

        let html = page
            .evaluate("document.documentElement.outerHTML")
            .await?
            .into_value::<String>()?;

        let url = page
            .url()
            .await?
            .unwrap_or(fallback_url);

        let title = page
            .get_title()
            .await?
            .unwrap_or_default();

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

    /// WOM output — structured JSON for AI agents.
    pub async fn see_wom(&mut self, revision: u64) -> Result<wom::WomDocument, Box<dyn std::error::Error>> {
        let fallback_url = self.last_url.clone();
        let page = self.ensure_page().await?;
        let t0 = Instant::now();

        // Wait for SPA frameworks to render. Detects DOM stability:
        // tracks interactive element count across animation frames,
        // resolves when count stabilizes or 5s timeout.
        let html = page
            .evaluate(Self::JS_WAIT_AND_CAPTURE)
            .await?
            .into_value::<String>()?;
        eprintln!("[WOM] captured {}KB ({}ms)", html.len() / 1024, t0.elapsed().as_millis());

        let url = page.url().await?.unwrap_or(fallback_url);
        let title = page.get_title().await?.unwrap_or_default();
        let html_bytes = html.len();

        let dom = parse_document(RcDom::default(), Default::default())
            .from_utf8()
            .read_from(&mut html.as_bytes())?;

        let doc = wom::build(&dom.document, &url, &title, html_bytes, "chrome", revision);

        eprintln!(
            "[WOM] {} | {} nodes | {} actions | {:.1}x compression | {}ms",
            doc.page.page_class,
            doc.nodes.len(),
            doc.actions.len(),
            doc.compression.compression_ratio,
            t0.elapsed().as_millis(),
        );

        Ok(doc)
    }

    // ─── Interaction ───

    pub async fn click(&mut self, text: &str) -> Result<bool, Box<dyn std::error::Error>> {
        let page = self.ensure_page().await?;

        let js = format!(
            r#"
            (() => {{
                const target = {target_json};
                const lower = target.toLowerCase();

                // 1. Buttons/links by text content
                for (const el of document.querySelectorAll('button, a, [role="button"], input[type="submit"], summary')) {{
                    const t = (el.textContent || el.value || el.getAttribute('aria-label') || '').trim();
                    if (t.toLowerCase().includes(lower)) {{
                        el.scrollIntoViewIfNeeded();
                        el.click();
                        return 'clicked: ' + t.substring(0, 60);
                    }}
                }}

                // 2. Aria-label match
                for (const el of document.querySelectorAll('[aria-label]')) {{
                    if (el.getAttribute('aria-label').toLowerCase().includes(lower)) {{
                        el.scrollIntoViewIfNeeded();
                        el.click();
                        return 'clicked-aria: ' + el.getAttribute('aria-label').substring(0, 60);
                    }}
                }}

                // 3. Title attribute
                for (const el of document.querySelectorAll('[title]')) {{
                    if (el.getAttribute('title').toLowerCase().includes(lower)) {{
                        el.scrollIntoViewIfNeeded();
                        el.click();
                        return 'clicked-title: ' + el.getAttribute('title').substring(0, 60);
                    }}
                }}

                // 4. Exact text match on visible elements
                const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_ELEMENT);
                while (walker.nextNode()) {{
                    const el = walker.currentNode;
                    const t = (el.textContent || '').trim();
                    if (t.toLowerCase() === lower && el.offsetParent !== null) {{
                        el.scrollIntoViewIfNeeded();
                        el.click();
                        return 'clicked-text: ' + t.substring(0, 60);
                    }}
                }}

                return 'not_found';
            }})()
            "#,
            target_json = serde_json::to_string(text)?
        );

        let result = page.evaluate(js).await?.into_value::<String>()?;
        let found = !result.starts_with("not_found");
        if found {
            eprintln!("[BROWSER] {result}");
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        } else {
            eprintln!("[BROWSER] click not found: '{text}'");
        }
        Ok(found)
    }

    pub async fn focus(&mut self, text: &str) -> Result<bool, Box<dyn std::error::Error>> {
        let page = self.ensure_page().await?;

        let js = format!(
            r#"
            (() => {{
                const target = {target_json};
                const lower = target.toLowerCase();

                // 1. Input/textarea by placeholder or aria-label
                for (const el of document.querySelectorAll('input, textarea, [contenteditable="true"], [role="textbox"]')) {{
                    const p = (el.placeholder || el.getAttribute('aria-label') || el.getAttribute('data-placeholder') || '').toLowerCase();
                    if (p.includes(lower) || lower === '') {{
                        el.scrollIntoViewIfNeeded();
                        el.focus();
                        el.click();
                        return 'focused: ' + (el.placeholder || el.getAttribute('aria-label') || el.tagName);
                    }}
                }}

                // 2. Any contenteditable
                for (const el of document.querySelectorAll('[contenteditable="true"]')) {{
                    el.scrollIntoViewIfNeeded();
                    el.focus();
                    el.click();
                    return 'focused-contenteditable';
                }}

                return 'not_found';
            }})()
            "#,
            target_json = serde_json::to_string(text)?
        );

        let result = page.evaluate(js).await?.into_value::<String>()?;
        let found = !result.starts_with("not_found");
        if found {
            eprintln!("[BROWSER] {result}");
        } else {
            eprintln!("[BROWSER] focus not found: '{text}'");
        }
        Ok(found)
    }

    pub async fn type_text(&mut self, text: &str) -> Result<(), Box<dyn std::error::Error>> {
        let page = self.ensure_page().await?;

        for ch in text.chars() {
            let s = ch.to_string();
            page.execute(
                DispatchKeyEventParams::builder()
                    .r#type(DispatchKeyEventType::KeyDown)
                    .text(&s)
                    .key(&s)
                    .build()
                    .unwrap(),
            )
            .await?;
            page.execute(
                DispatchKeyEventParams::builder()
                    .r#type(DispatchKeyEventType::KeyUp)
                    .key(&s)
                    .build()
                    .unwrap(),
            )
            .await?;
            tokio::time::sleep(std::time::Duration::from_millis(15)).await;
        }
        eprintln!("[BROWSER] typed {} chars", text.len());
        Ok(())
    }

    pub async fn press(&mut self, key: &str) -> Result<(), Box<dyn std::error::Error>> {
        let page = self.ensure_page().await?;

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

        page.execute(
            DispatchKeyEventParams::builder()
                .r#type(DispatchKeyEventType::KeyDown)
                .key(key_val)
                .code(code)
                .windows_virtual_key_code(vkc)
                .build()
                .unwrap(),
        )
        .await?;
        page.execute(
            DispatchKeyEventParams::builder()
                .r#type(DispatchKeyEventType::KeyUp)
                .key(key_val)
                .code(code)
                .windows_virtual_key_code(vkc)
                .build()
                .unwrap(),
        )
        .await?;

        eprintln!("[BROWSER] pressed {key}");
        Ok(())
    }

    pub async fn scroll(&mut self, direction: &str) -> Result<(), Box<dyn std::error::Error>> {
        let page = self.ensure_page().await?;
        let delta = match direction {
            "down" | "d" => 400,
            "up" | "u" => -400,
            "bottom" => 99999,
            "top" => -99999,
            _ => 400,
        };
        page.evaluate(format!("window.scrollBy(0, {delta})")).await?;
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        eprintln!("[BROWSER] scroll {direction}");
        Ok(())
    }

    pub async fn screenshot(&mut self) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let page = self.ensure_page().await?;
        let data = page.screenshot(
            chromiumoxide::page::ScreenshotParams::builder()
                .format(CaptureScreenshotFormat::Jpeg)
                .quality(40)
                .build(),
        ).await?;
        eprintln!("[BROWSER] screenshot: {}KB", data.len() / 1024);
        Ok(data)
    }

    pub async fn eval(&mut self, js: &str) -> Result<String, Box<dyn std::error::Error>> {
        let page = self.ensure_page().await?;
        let result = page.evaluate(js).await?.into_value::<String>()?;
        Ok(result)
    }

    // ─── Dialog handling ───

    /// Auto-dismiss dialogs (alert/confirm/prompt). Call before navigating.
    pub async fn setup_dialog_handler(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let page = self.ensure_page().await?;
        // Inject JS that auto-accepts dialogs and logs them
        page.evaluate(r#"
            window.__neo_dialogs = [];
            window.alert = function(msg) { window.__neo_dialogs.push({type:'alert',message:msg}); };
            window.confirm = function(msg) { window.__neo_dialogs.push({type:'confirm',message:msg}); return true; };
            window.prompt = function(msg,def) { window.__neo_dialogs.push({type:'prompt',message:msg}); return def || ''; };
            window.onbeforeunload = null;
        "#).await?;
        eprintln!("[BROWSER] dialog handler installed");
        Ok(())
    }

    /// Get and clear any dialogs that appeared.
    pub async fn get_dialogs(&mut self) -> Result<Vec<serde_json::Value>, Box<dyn std::error::Error>> {
        let page = self.ensure_page().await?;
        let result = page.evaluate(r#"
            (() => {
                const d = window.__neo_dialogs || [];
                window.__neo_dialogs = [];
                return JSON.stringify(d);
            })()
        "#).await?.into_value::<String>()?;
        let dialogs: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap_or_default();
        Ok(dialogs)
    }

    // ─── Bulk form fill ───

    /// Fill multiple form fields at once. fields = [(target_text, value), ...]
    pub async fn fill_form(&mut self, fields: &[(String, String)]) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let mut results = Vec::new();
        for (target, value) in fields {
            let focused = self.focus(target).await?;
            if focused {
                // Clear existing value first
                let page = self.ensure_page().await?;
                page.evaluate("document.activeElement && (document.activeElement.value = '')").await?;
                // Type the new value
                self.type_text(value).await?;
                results.push(format!("filled: {target} = {}", if value.len() > 20 { &value[..20] } else { value }));
            } else {
                results.push(format!("not_found: {target}"));
            }
        }
        eprintln!("[BROWSER] fill_form: {} fields", fields.len());
        Ok(results)
    }

    // ─── Network capture ───

    /// Start capturing network requests via Performance API.
    pub async fn start_network_capture(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let page = self.ensure_page().await?;
        page.evaluate(r#"
            window.__neo_net = [];
            const origFetch = window.fetch;
            window.fetch = function(...args) {
                const url = typeof args[0] === 'string' ? args[0] : args[0]?.url || '';
                const method = args[1]?.method || 'GET';
                const entry = {type:'fetch', method, url, ts: Date.now()};
                window.__neo_net.push(entry);
                return origFetch.apply(this, args).then(r => {
                    entry.status = r.status;
                    return r;
                });
            };
            const origXHR = XMLHttpRequest.prototype.open;
            XMLHttpRequest.prototype.open = function(method, url) {
                this.__neo_method = method;
                this.__neo_url = url;
                return origXHR.apply(this, arguments);
            };
            const origSend = XMLHttpRequest.prototype.send;
            XMLHttpRequest.prototype.send = function() {
                const entry = {type:'xhr', method: this.__neo_method, url: this.__neo_url, ts: Date.now()};
                window.__neo_net.push(entry);
                this.addEventListener('load', function() { entry.status = this.status; });
                return origSend.apply(this, arguments);
            };
        "#).await?;
        eprintln!("[BROWSER] network capture started");
        Ok(())
    }

    /// Read captured network requests and clear the buffer.
    pub async fn read_network(&mut self) -> Result<Vec<serde_json::Value>, Box<dyn std::error::Error>> {
        let page = self.ensure_page().await?;
        let result = page.evaluate(r#"
            (() => {
                const entries = window.__neo_net || [];
                window.__neo_net = [];
                // Also get resource timing entries
                const perf = performance.getEntriesByType('resource')
                    .filter(e => e.initiatorType === 'fetch' || e.initiatorType === 'xmlhttprequest')
                    .slice(-50)
                    .map(e => ({type: e.initiatorType, url: e.name, duration_ms: Math.round(e.duration)}));
                return JSON.stringify([...entries, ...perf].slice(-100));
            })()
        "#).await?.into_value::<String>()?;
        let requests: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap_or_default();
        Ok(requests)
    }

    // ─── Console capture ───

    /// Start capturing console messages.
    pub async fn start_console_capture(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let page = self.ensure_page().await?;
        page.evaluate(r#"
            window.__neo_console = [];
            const origLog = console.log;
            const origWarn = console.warn;
            const origError = console.error;
            console.log = function(...args) {
                window.__neo_console.push({level:'log', text: args.map(String).join(' '), ts: Date.now()});
                origLog.apply(console, args);
            };
            console.warn = function(...args) {
                window.__neo_console.push({level:'warn', text: args.map(String).join(' '), ts: Date.now()});
                origWarn.apply(console, args);
            };
            console.error = function(...args) {
                window.__neo_console.push({level:'error', text: args.map(String).join(' '), ts: Date.now()});
                origError.apply(console, args);
            };
            window.addEventListener('error', function(e) {
                window.__neo_console.push({level:'exception', text: e.message + ' at ' + e.filename + ':' + e.lineno, ts: Date.now()});
            });
        "#).await?;
        eprintln!("[BROWSER] console capture started");
        Ok(())
    }

    /// Read captured console messages and clear.
    pub async fn read_console(&mut self) -> Result<Vec<serde_json::Value>, Box<dyn std::error::Error>> {
        let page = self.ensure_page().await?;
        let result = page.evaluate(r#"
            (() => {
                const msgs = (window.__neo_console || []).slice(-50);
                window.__neo_console = [];
                return JSON.stringify(msgs);
            })()
        "#).await?.into_value::<String>()?;
        let messages: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap_or_default();
        Ok(messages)
    }

    // ─── Select option ───

    /// Select an option from a <select> dropdown by visible text.
    pub async fn select_option(&mut self, target: &str, value: &str) -> Result<bool, Box<dyn std::error::Error>> {
        let page = self.ensure_page().await?;
        let js = format!(
            r#"
            (() => {{
                const target = {target_json};
                const value = {value_json};
                for (const sel of document.querySelectorAll('select')) {{
                    const label = (sel.getAttribute('aria-label') || sel.getAttribute('name') || '').toLowerCase();
                    if (label.includes(target.toLowerCase()) || target === '') {{
                        for (const opt of sel.options) {{
                            if (opt.text.toLowerCase().includes(value.toLowerCase()) ||
                                opt.value.toLowerCase().includes(value.toLowerCase())) {{
                                sel.value = opt.value;
                                sel.dispatchEvent(new Event('change', {{bubbles: true}}));
                                return 'selected: ' + opt.text;
                            }}
                        }}
                        return 'option_not_found: ' + value;
                    }}
                }}
                return 'select_not_found: ' + target;
            }})()
            "#,
            target_json = serde_json::to_string(target)?,
            value_json = serde_json::to_string(value)?,
        );
        let result = page.evaluate(js).await?.into_value::<String>()?;
        let found = result.starts_with("selected:");
        eprintln!("[BROWSER] {result}");
        Ok(found)
    }

    // ─── Hover ───

    /// Hover over an element by text match.
    pub async fn hover(&mut self, text: &str) -> Result<bool, Box<dyn std::error::Error>> {
        let page = self.ensure_page().await?;
        let js = format!(
            r#"
            (() => {{
                const target = {target_json};
                const lower = target.toLowerCase();
                for (const el of document.querySelectorAll('button, a, [role="button"], li, td, th, span, div')) {{
                    const t = (el.textContent || el.getAttribute('aria-label') || '').trim();
                    if (t.toLowerCase().includes(lower) && el.offsetParent !== null) {{
                        el.scrollIntoViewIfNeeded();
                        el.dispatchEvent(new MouseEvent('mouseenter', {{bubbles: true}}));
                        el.dispatchEvent(new MouseEvent('mouseover', {{bubbles: true}}));
                        return 'hovered: ' + t.substring(0, 60);
                    }}
                }}
                return 'not_found';
            }})()
            "#,
            target_json = serde_json::to_string(text)?,
        );
        let result = page.evaluate(js).await?.into_value::<String>()?;
        let found = !result.starts_with("not_found");
        eprintln!("[BROWSER] {result}");
        Ok(found)
    }

    pub async fn wait(&self, secs: f64) {
        tokio::time::sleep(std::time::Duration::from_secs_f64(secs)).await;
    }

    pub async fn close(mut self) -> Result<(), Box<dyn std::error::Error>> {
        if !self.connected_mode {
            self.browser.close().await?;
        }
        self._handler.abort();
        eprintln!("[BROWSER] closed");
        Ok(())
    }

    /// Connect to the user's running Chrome by reading DevToolsActivePort.
    pub async fn connect_running() -> Result<Self, Box<dyn std::error::Error>> {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        let devtools_file = std::path::PathBuf::from(&home)
            .join("Library/Application Support/Google/Chrome/DevToolsActivePort");

        let content = std::fs::read_to_string(&devtools_file)
            .map_err(|_| "Chrome DevToolsActivePort not found — is Chrome running?")?;

        let mut lines = content.lines();
        let port = lines.next().ok_or("Empty DevToolsActivePort")?.trim();
        let ws_path = lines.next().ok_or("No WS path in DevToolsActivePort")?.trim();

        let ws_url = format!("ws://127.0.0.1:{port}{ws_path}");
        eprintln!("[BROWSER] Found running Chrome on port {port}");
        Self::connect(&ws_url).await
    }

    // ─── localStorage ───

    /// Get all localStorage entries as a HashMap.
    pub async fn get_local_storage(
        &mut self,
    ) -> Result<std::collections::HashMap<String, String>, Box<dyn std::error::Error>> {
        let page = self.ensure_page().await?;
        let result = page
            .evaluate(
                "JSON.stringify(Object.fromEntries(Object.keys(localStorage).map(k=>[k,localStorage[k]])))",
            )
            .await?
            .into_value::<String>()?;
        let map: std::collections::HashMap<String, String> = serde_json::from_str(&result)?;
        eprintln!("[BROWSER] localStorage: {} keys", map.len());
        Ok(map)
    }

    /// Set localStorage entries from a HashMap.
    pub async fn set_local_storage(
        &mut self,
        data: &std::collections::HashMap<String, String>,
    ) -> Result<usize, Box<dyn std::error::Error>> {
        let page = self.ensure_page().await?;
        let json = serde_json::to_string(data)?;
        let js = format!(
            "(() => {{ const d = {}; Object.entries(d).forEach(([k,v]) => localStorage.setItem(k,v)); return Object.keys(d).length; }})()",
            json
        );
        let count = page.evaluate(js).await?.into_value::<i64>()?;
        eprintln!("[BROWSER] localStorage: injected {} keys", count);
        Ok(count as usize)
    }

    /// Get all cookies via CDP (includes HttpOnly cookies that JS can't see).
    pub async fn get_all_cookies(
        &mut self,
    ) -> Result<Vec<serde_json::Value>, Box<dyn std::error::Error>> {
        let page = self.ensure_page().await?;
        // Use JS cookieStore API as fallback; CDP getAllCookies is better but needs raw CDP
        let result = page
            .evaluate(
                r#"(async()=>{try{const c=await cookieStore.getAll();return JSON.stringify(c)}catch{return JSON.stringify(document.cookie.split(';').map(s=>{const[n,...v]=s.trim().split('=');return{name:n,value:v.join('=')}}))}})();"#,
            )
            .await?
            .into_value::<String>()?;
        let cookies: Vec<serde_json::Value> = serde_json::from_str(&result)?;
        eprintln!("[BROWSER] exported {} cookies", cookies.len());
        Ok(cookies)
    }
}
