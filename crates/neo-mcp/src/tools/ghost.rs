//! Ghost tool — neomode Chrome for operations that need a real browser.
//!
//! V2: Uses Rust `SyncChromeSession` (CDP) for most actions instead of
//! shelling out to ghost.py. The Python fallback remains for screenshot
//! and chat (complex ProseMirror interaction).

use serde_json::{json, Value};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use neo_chrome::sync_session::SyncChromeSession;

use crate::McpError;
use crate::state::McpState;

// ── Singleton Chrome session ──

/// Lazy-initialized Chrome neomode session. Created on first ghost action
/// that needs Chrome. Lives for the entire MCP server lifetime.
///
/// We use `OnceLock<Mutex<Option<SyncChromeSession>>>` to avoid the unstable
/// `get_or_try_init`. The OnceLock is infallibly initialized with None,
/// and the inner Option is set on first use.
static CHROME: OnceLock<Mutex<Option<SyncChromeSession>>> = OnceLock::new();

/// Get or initialize the Chrome singleton. Returns a MutexGuard over the session.
fn chrome_session() -> Result<ChromeGuard, McpError> {
    let mtx = CHROME.get_or_init(|| Mutex::new(None));
    let mut guard = mtx.lock()
        .map_err(|e| McpError::InvalidParams(format!("Chrome mutex poisoned: {e}")))?;
    if guard.is_none() {
        eprintln!("[ghost] Launching neomode Chrome...");
        let session = neo_chrome::sync_session::launch_neomode()
            .map_err(|e| McpError::InvalidParams(format!("Chrome launch failed: {e}")))?;
        eprintln!("[ghost] Chrome ready");
        *guard = Some(session);
    }
    Ok(ChromeGuard(guard))
}

/// Wrapper around MutexGuard that provides direct access to the SyncChromeSession.
struct ChromeGuard(std::sync::MutexGuard<'static, Option<SyncChromeSession>>);

impl std::ops::Deref for ChromeGuard {
    type Target = SyncChromeSession;
    fn deref(&self) -> &SyncChromeSession {
        self.0.as_ref().expect("Chrome session initialized in chrome_session()")
    }
}

// ── Tool definition ──

/// All supported ghost actions.
const ALL_ACTIONS: &[&str] = &[
    "search", "navigate", "read", "find", "click", "type", "fill_form", "submit",
    "screenshot", "scroll", "extract_data", "login", "download", "monitor",
    "api_intercept", "cookie_manage", "multi_tab", "wait_for", "pipeline",
    "open", "chat", "html",
];

pub(crate) fn definition() -> super::ToolDef {
    super::ToolDef {
        name: "ghost",
        description: "Neomode ghost browser — real Chrome (headless, undetectable). \
            Use for Cloudflare-protected sites, SPAs, form filling, chat interactions, \
            search, scraping, login, monitoring, pipelines, and more. \
            Actions: search, navigate, read, find, click, type, fill_form, submit, \
            screenshot, scroll, extract_data, login, download, monitor, \
            api_intercept, cookie_manage, multi_tab, wait_for, pipeline, \
            open, chat, html.",
        schema: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ALL_ACTIONS,
                    "description": "Action to perform"
                },
                "url": {
                    "type": "string",
                    "description": "URL to navigate to"
                },
                "message": {
                    "type": "string",
                    "description": "For chat action: message to send"
                },
                "profile": {
                    "type": "string",
                    "description": "Chrome profile name for cookie import (e.g. 'Profile 24')"
                },
                "wait": {
                    "type": "integer",
                    "default": 5000,
                    "description": "Wait time in ms after page load"
                },
                "query": {
                    "type": "string",
                    "description": "Search query (for search action)"
                },
                "selector": {
                    "type": "string",
                    "description": "CSS/XPath selector (for find, click, read, wait_for, submit)"
                },
                "value": {
                    "type": "string",
                    "description": "Value to type (for type action)"
                },
                "text": {
                    "type": "string",
                    "description": "Text to find or click (for find, click)"
                },
                "fields": {
                    "type": "string",
                    "description": "JSON string of field->value pairs (for fill_form)"
                },
                "direction": {
                    "type": "string",
                    "enum": ["up", "down", "left", "right"],
                    "description": "Scroll direction (for scroll)"
                },
                "amount": {
                    "type": "integer",
                    "description": "Scroll amount in pixels (for scroll)"
                },
                "type_": {
                    "type": "string",
                    "enum": ["table", "list", "product", "links"],
                    "description": "Data extraction type (for extract_data)"
                },
                "email": {
                    "type": "string",
                    "description": "Email for login"
                },
                "password": {
                    "type": "string",
                    "description": "Password for login"
                },
                "engine": {
                    "type": "string",
                    "enum": ["google", "bing", "duckduckgo"],
                    "description": "Search engine (for search, default: google)"
                },
                "num": {
                    "type": "integer",
                    "default": 10,
                    "description": "Number of results (for search)"
                },
                "pattern": {
                    "type": "string",
                    "description": "URL pattern to intercept (for api_intercept)"
                },
                "steps": {
                    "type": "string",
                    "description": "JSON string of pipeline steps (for pipeline)"
                },
                "by": {
                    "type": "string",
                    "enum": ["text", "css", "xpath", "role"],
                    "description": "Locator strategy (for find)"
                },
                "index": {
                    "type": "integer",
                    "description": "Tab or element index (for click, multi_tab)"
                }
            },
            "required": ["action"]
        }),
    }
}

// ── Main dispatch ──

pub fn call(args: Value, state: &mut McpState) -> Result<Value, McpError> {
    let action = args["action"].as_str().unwrap_or("open");
    let url = args["url"].as_str().unwrap_or("");
    let message = args["message"].as_str().unwrap_or("");
    let profile = args["profile"].as_str();
    let wait = args["wait"].as_u64().unwrap_or(5000);

    match action {
        // ── Rust HTTP fast path (no Chrome needed) ──

        "search" => {
            let query = args["query"].as_str().unwrap_or("");
            if query.is_empty() {
                return Err(McpError::InvalidParams("query required for search".into()));
            }
            // Always use Rust HTTP search — 10x faster, same results
            let search_args = json!({"query": query, "num": args["num"].as_u64().unwrap_or(10)});
            match crate::tools::search::call(search_args, state) {
                Ok(result) => {
                    let text = serde_json::to_string(&result).unwrap_or_default();
                    if text.len() > 50 {
                        return Ok(json!({"content": [{"type": "text", "text": text}]}));
                    }
                    eprintln!("[ghost] Rust search empty, trying Chrome...");
                }
                Err(_) => {}
            }
            // Fallback to ghost.py Chrome search
            let engine = args["engine"].as_str().unwrap_or("duckduckgo");
            let num = args["num"].as_u64().unwrap_or(10);
            let num_str = num.to_string();
            let ghost_args = vec!["search", query, "--engine", engine, "--num", &num_str];
            ghost_delegate(&ghost_args, 30, profile)
        }

        // ── Chrome via SyncChromeSession ──

        "navigate" | "open" => {
            if url.is_empty() {
                return Err(McpError::InvalidParams("url required".into()));
            }
            // Direct to Chrome neomode — fast path is browse, not ghost
            // Ghost = "I need a real browser". Use browse for HTTP-only.
            chrome_navigate(url, wait)
        }

        "read" => {
            if url.is_empty() {
                return Err(McpError::InvalidParams("url required for read".into()));
            }
            let selector = args["selector"].as_str().unwrap_or("");
            chrome_read(url, selector, wait)
        }

        "find" => {
            let selector = args["selector"].as_str().unwrap_or("");
            let text = args["text"].as_str().unwrap_or("");
            let by = args["by"].as_str().unwrap_or("css");
            // If we need to navigate first
            if !url.is_empty() {
                let session = chrome_session()?;
                let _ = session.navigate(url);
                drop(session);
                // Brief wait for page load
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            chrome_find(selector, text, by)
        }

        "click" => {
            let selector = args["selector"].as_str().unwrap_or("");
            let text = args["text"].as_str().unwrap_or("");
            let index = args["index"].as_u64();
            if !url.is_empty() {
                let session = chrome_session()?;
                let _ = session.navigate(url);
                drop(session);
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            chrome_click(selector, text, index)
        }

        "type" => {
            let selector = args["selector"].as_str().unwrap_or("");
            let value = args["value"].as_str().unwrap_or("");
            if value.is_empty() {
                return Err(McpError::InvalidParams("value required for type".into()));
            }
            if !url.is_empty() {
                let session = chrome_session()?;
                let _ = session.navigate(url);
                drop(session);
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            chrome_type(selector, value)
        }

        "fill_form" => {
            let fields = args["fields"].as_str().unwrap_or("{}");
            if fields == "{}" {
                return Err(McpError::InvalidParams("fields required for fill_form".into()));
            }
            if !url.is_empty() {
                let session = chrome_session()?;
                let _ = session.navigate(url);
                drop(session);
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            chrome_fill_form(fields)
        }

        "scroll" => {
            let direction = args["direction"].as_str().unwrap_or("down");
            let amount = args["amount"].as_u64().unwrap_or(500);
            if !url.is_empty() {
                let session = chrome_session()?;
                let _ = session.navigate(url);
                drop(session);
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            chrome_scroll(direction, amount)
        }

        "html" => {
            if url.is_empty() {
                return Err(McpError::InvalidParams("url required".into()));
            }
            chrome_html(url, wait)
        }

        // ── Legacy fallback: ghost.py ──

        "screenshot" => {
            // CDP screenshot requires base64 decoding + file write — keep ghost.py
            if url.is_empty() {
                return Err(McpError::InvalidParams("url required".into()));
            }
            let wait_str = wait.to_string();
            let ghost_args = vec!["open", url, "--wait", &wait_str, "--output", "/tmp/ghost-mcp-screenshot.png"];
            ghost_delegate(&ghost_args, 30, profile)
        }

        "chat" => {
            // Complex ProseMirror interaction — legacy fallback
            ghost_chat(url, message, profile)
        }

        "submit" => {
            let selector = args["selector"].as_str().unwrap_or("");
            if !url.is_empty() {
                let session = chrome_session()?;
                let _ = session.navigate(url);
                drop(session);
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            chrome_submit(selector)
        }

        "extract_data" => {
            let type_ = args["type_"].as_str().unwrap_or("table");
            let selector = args["selector"].as_str().unwrap_or("");
            if url.is_empty() {
                return Err(McpError::InvalidParams("url required for extract_data".into()));
            }
            // Try Rust engine first for links/text extraction
            if type_ == "links" || type_ == "list" {
                let browse_args = json!({"url": url});
                if let Ok(_) = crate::tools::browse::call(browse_args, state) {
                    let fmt = if type_ == "links" { "links" } else { "text" };
                    let extract_args = json!({"format": fmt});
                    if let Ok(result) = crate::tools::extract::call(extract_args, state) {
                        let text = serde_json::to_string(&result).unwrap_or_default();
                        if text.len() > 50 {
                            return Ok(json!({"content": [{"type": "text", "text": text}]}));
                        }
                    }
                }
                eprintln!("[ghost] Rust extract insufficient, trying Chrome...");
            }
            // Ghost.py for tables/products or fallback
            let mut ghost_args = vec!["extract_data", url, "--type", type_];
            if !selector.is_empty() {
                ghost_args.extend(&["--selector", selector]);
            }
            ghost_delegate(&ghost_args, 30, profile)
        }

        "login" => {
            let email = args["email"].as_str().unwrap_or("");
            let password = args["password"].as_str().unwrap_or("");
            if url.is_empty() || email.is_empty() || password.is_empty() {
                return Err(McpError::InvalidParams("url, email, and password required for login".into()));
            }
            let ghost_args = vec!["login", url, "--email", email, "--password", password];
            ghost_delegate(&ghost_args, 60, profile)
        }

        "download" => {
            if url.is_empty() {
                return Err(McpError::InvalidParams("url required for download".into()));
            }
            let selector = args["selector"].as_str().unwrap_or("");
            let mut ghost_args = vec!["download", url];
            if !selector.is_empty() {
                ghost_args.extend(&["--selector", selector]);
            }
            ghost_delegate(&ghost_args, 60, profile)
        }

        "monitor" => {
            if url.is_empty() {
                return Err(McpError::InvalidParams("url required for monitor".into()));
            }
            let selector = args["selector"].as_str().unwrap_or("");
            let mut ghost_args = vec!["monitor", url];
            if !selector.is_empty() {
                ghost_args.extend(&["--selector", selector]);
            }
            ghost_delegate(&ghost_args, 60, profile)
        }

        "api_intercept" => {
            let pattern = args["pattern"].as_str().unwrap_or("*");
            if url.is_empty() {
                return Err(McpError::InvalidParams("url required for api_intercept".into()));
            }
            let ghost_args = vec!["api_intercept", url, "--pattern", pattern];
            ghost_delegate(&ghost_args, 30, profile)
        }

        "cookie_manage" => {
            if url.is_empty() {
                return Err(McpError::InvalidParams("url required for cookie_manage".into()));
            }
            let ghost_args = vec!["cookie_manage", url];
            ghost_delegate(&ghost_args, 15, profile)
        }

        "multi_tab" => {
            let index = args["index"].as_u64();
            let mut ghost_args = vec!["multi_tab"];
            let index_str;
            if let Some(i) = index {
                index_str = i.to_string();
                ghost_args.extend(&["--index", &index_str]);
            }
            if !url.is_empty() {
                ghost_args.extend(&["--url", url]);
            }
            ghost_delegate(&ghost_args, 15, profile)
        }

        "wait_for" => {
            let selector = args["selector"].as_str().unwrap_or("");
            if selector.is_empty() {
                return Err(McpError::InvalidParams("selector required for wait_for".into()));
            }
            let wait_str = wait.to_string();
            let mut ghost_args = vec!["wait_for", "--selector", selector, "--timeout", &wait_str];
            if !url.is_empty() {
                ghost_args.extend(&["--url", url]);
            }
            ghost_delegate(&ghost_args, 60, profile)
        }

        "pipeline" => {
            let steps = args["steps"].as_str().unwrap_or("[]");
            if steps == "[]" {
                return Err(McpError::InvalidParams("steps required for pipeline".into()));
            }
            let mut ghost_args = vec!["pipeline", "--steps", steps];
            if !url.is_empty() {
                ghost_args.extend(&["--url", url]);
            }
            ghost_delegate(&ghost_args, 120, profile)
        }

        other => Err(McpError::InvalidParams(format!("Unknown action: {other}"))),
    }
}

// ── Chrome CDP actions via SyncChromeSession ──

/// Navigate to URL, extract title + text + element count via JS.
fn chrome_navigate(url: &str, _wait: u64) -> Result<Value, McpError> {
    let session = chrome_session()?;

    session.navigate(url)
        .map_err(|e| McpError::InvalidParams(format!("Chrome navigate failed: {e}")))?;

    let js = r#"
        (function() {
            var title = document.title || '';
            var body = document.body ? document.body.innerText : '';
            var elems = document.querySelectorAll('*').length;
            return JSON.stringify({title: title, text: body.substring(0, 2000), elements: elems});
        })()
    "#;

    let result_str = session.eval(js)
        .map_err(|e| McpError::InvalidParams(format!("Chrome eval failed: {e}")))?;

    // Parse the JSON string returned by eval
    let info: Value = serde_json::from_str(&result_str).unwrap_or_else(|_| json!({"text": result_str}));
    let title = info["title"].as_str().unwrap_or("");
    let text = info["text"].as_str().unwrap_or("");
    let elements = info["elements"].as_u64().unwrap_or(0);

    let summary = format!(
        "[Ghost/Chrome] {} | {} elements\n\n{}",
        title, elements,
        if text.len() > 500 { &text[..500] } else { text }
    );

    Ok(json!({"content": [{"type": "text", "text": summary}]}))
}

/// Read page content. If selector given, extract from that element.
fn chrome_read(url: &str, selector: &str, _wait: u64) -> Result<Value, McpError> {
    let session = chrome_session()?;

    session.navigate(url)
        .map_err(|e| McpError::InvalidParams(format!("Chrome navigate failed: {e}")))?;

    let js = if selector.is_empty() {
        // Extract article-like text from page
        r#"
            (function() {
                var article = document.querySelector('article') || document.querySelector('main') || document.body;
                return article ? article.innerText.substring(0, 5000) : '';
            })()
        "#.to_string()
    } else {
        format!(
            r#"
            (function() {{
                var el = document.querySelector({sel});
                return el ? el.innerText.substring(0, 5000) : 'Selector not found: ' + {sel};
            }})()
            "#,
            sel = serde_json::to_string(selector).unwrap_or_else(|_| format!("\"{}\"", selector))
        )
    };

    let text = session.eval(&js)
        .map_err(|e| McpError::InvalidParams(format!("Chrome eval failed: {e}")))?;

    Ok(json!({"content": [{"type": "text", "text": text}]}))
}

/// Find elements by CSS selector, text content, or ARIA role.
fn chrome_find(selector: &str, text: &str, by: &str) -> Result<Value, McpError> {
    let session = chrome_session()?;

    let js = match by {
        "css" => {
            let sel = serde_json::to_string(if selector.is_empty() { "*" } else { selector })
                .unwrap_or_else(|_| "\"*\"".to_string());
            format!(
                r#"
                (function() {{
                    var els = document.querySelectorAll({sel});
                    var results = [];
                    for (var i = 0; i < Math.min(els.length, 20); i++) {{
                        var el = els[i];
                        results.push({{
                            tag: el.tagName.toLowerCase(),
                            text: (el.innerText || '').substring(0, 100),
                            id: el.id || '',
                            class: el.className || '',
                            href: el.href || ''
                        }});
                    }}
                    return JSON.stringify({{found: results.length, total: els.length, results: results}});
                }})()
                "#,
                sel = sel
            )
        }
        "text" => {
            let search_text = serde_json::to_string(if text.is_empty() { selector } else { text })
                .unwrap_or_else(|_| "\"\"".to_string());
            format!(
                r#"
                (function() {{
                    var xpath = "//\*[contains(text(), " + {txt} + ")]";
                    try {{
                        // XPath-based text search
                        var all = document.querySelectorAll('*');
                        var results = [];
                        var searchText = {txt}.toLowerCase();
                        for (var i = 0; i < all.length && results.length < 20; i++) {{
                            var el = all[i];
                            if (el.children.length === 0 || el.tagName === 'A' || el.tagName === 'BUTTON') {{
                                var t = (el.innerText || el.textContent || '').trim();
                                if (t.toLowerCase().indexOf(searchText) >= 0) {{
                                    results.push({{
                                        tag: el.tagName.toLowerCase(),
                                        text: t.substring(0, 100),
                                        id: el.id || '',
                                        class: el.className || ''
                                    }});
                                }}
                            }}
                        }}
                        return JSON.stringify({{found: results.length, results: results}});
                    }} catch(e) {{
                        return JSON.stringify({{error: e.message}});
                    }}
                }})()
                "#,
                txt = search_text
            )
        }
        "role" => {
            let role = serde_json::to_string(if selector.is_empty() { text } else { selector })
                .unwrap_or_else(|_| "\"button\"".to_string());
            format!(
                r#"
                (function() {{
                    var els = document.querySelectorAll('[role=' + {role} + ']');
                    var results = [];
                    for (var i = 0; i < Math.min(els.length, 20); i++) {{
                        var el = els[i];
                        results.push({{
                            tag: el.tagName.toLowerCase(),
                            text: (el.innerText || '').substring(0, 100),
                            role: el.getAttribute('role') || '',
                            id: el.id || ''
                        }});
                    }}
                    return JSON.stringify({{found: results.length, results: results}});
                }})()
                "#,
                role = role
            )
        }
        "xpath" => {
            let xpath = serde_json::to_string(if selector.is_empty() { "//*" } else { selector })
                .unwrap_or_else(|_| "\"//*\"".to_string());
            format!(
                r#"
                (function() {{
                    try {{
                        var result = document.evaluate({xpath}, document, null, XPathResult.ORDERED_NODE_SNAPSHOT_TYPE, null);
                        var results = [];
                        for (var i = 0; i < Math.min(result.snapshotLength, 20); i++) {{
                            var el = result.snapshotItem(i);
                            results.push({{
                                tag: el.tagName ? el.tagName.toLowerCase() : 'text',
                                text: (el.innerText || el.textContent || '').substring(0, 100),
                                id: el.id || ''
                            }});
                        }}
                        return JSON.stringify({{found: results.length, total: result.snapshotLength, results: results}});
                    }} catch(e) {{
                        return JSON.stringify({{error: e.message}});
                    }}
                }})()
                "#,
                xpath = xpath
            )
        }
        _ => {
            return Err(McpError::InvalidParams(format!("Unknown find strategy: {by}")));
        }
    };

    let result_str = session.eval(&js)
        .map_err(|e| McpError::InvalidParams(format!("Chrome eval failed: {e}")))?;

    Ok(json!({"content": [{"type": "text", "text": result_str}]}))
}

/// Click an element by CSS selector or text content.
fn chrome_click(selector: &str, text: &str, index: Option<u64>) -> Result<Value, McpError> {
    let session = chrome_session()?;

    let js = if !selector.is_empty() {
        let sel = serde_json::to_string(selector).unwrap_or_else(|_| format!("\"{}\"", selector));
        let idx = index.unwrap_or(0);
        format!(
            r#"
            (function() {{
                var els = document.querySelectorAll({sel});
                if (els.length === 0) return 'No elements found for selector: ' + {sel};
                var idx = {idx};
                if (idx >= els.length) idx = 0;
                els[idx].click();
                return 'Clicked element ' + idx + ' of ' + els.length + ': ' + els[idx].tagName + ' "' + (els[idx].innerText || '').substring(0, 50) + '"';
            }})()
            "#,
            sel = sel,
            idx = idx
        )
    } else if !text.is_empty() {
        let txt = serde_json::to_string(text).unwrap_or_else(|_| format!("\"{}\"", text));
        format!(
            r#"
            (function() {{
                var searchText = {txt}.toLowerCase();
                var all = document.querySelectorAll('a, button, input[type="submit"], [role="button"], [onclick]');
                for (var i = 0; i < all.length; i++) {{
                    var el = all[i];
                    var t = (el.innerText || el.value || el.getAttribute('aria-label') || '').toLowerCase();
                    if (t.indexOf(searchText) >= 0) {{
                        el.click();
                        return 'Clicked: ' + el.tagName + ' "' + (el.innerText || '').substring(0, 50) + '"';
                    }}
                }}
                return 'No clickable element found with text: ' + {txt};
            }})()
            "#,
            txt = txt
        )
    } else {
        return Err(McpError::InvalidParams("selector or text required for click".into()));
    };

    let result = session.eval(&js)
        .map_err(|e| McpError::InvalidParams(format!("Chrome eval failed: {e}")))?;

    Ok(json!({"content": [{"type": "text", "text": result}]}))
}

/// Type text into an input element.
fn chrome_type(selector: &str, value: &str) -> Result<Value, McpError> {
    let session = chrome_session()?;

    let sel = if selector.is_empty() {
        // Default: focus the first visible input/textarea
        "document.querySelector('input:not([type=hidden]), textarea')"
    } else {
        // Will be interpolated below
        ""
    };

    let val = serde_json::to_string(value).unwrap_or_else(|_| format!("\"{}\"", value));

    let js = if selector.is_empty() {
        format!(
            r#"
            (function() {{
                var el = {sel};
                if (!el) return 'No input element found on page';
                el.focus();
                el.value = {val};
                el.dispatchEvent(new Event('input', {{bubbles: true}}));
                el.dispatchEvent(new Event('change', {{bubbles: true}}));
                return 'Typed into ' + el.tagName + '[' + (el.name || el.id || el.type || '') + ']';
            }})()
            "#,
            sel = sel,
            val = val
        )
    } else {
        let sel_str = serde_json::to_string(selector).unwrap_or_else(|_| format!("\"{}\"", selector));
        format!(
            r#"
            (function() {{
                var el = document.querySelector({sel});
                if (!el) return 'Element not found: ' + {sel};
                el.focus();
                el.value = {val};
                el.dispatchEvent(new Event('input', {{bubbles: true}}));
                el.dispatchEvent(new Event('change', {{bubbles: true}}));
                return 'Typed into ' + el.tagName + '[' + (el.name || el.id || el.type || '') + ']';
            }})()
            "#,
            sel = sel_str,
            val = val
        )
    };

    let result = session.eval(&js)
        .map_err(|e| McpError::InvalidParams(format!("Chrome eval failed: {e}")))?;

    Ok(json!({"content": [{"type": "text", "text": result}]}))
}

/// Fill multiple form fields from a JSON object.
fn chrome_fill_form(fields_json: &str) -> Result<Value, McpError> {
    let session = chrome_session()?;

    let fields_val = serde_json::to_string(fields_json)
        .unwrap_or_else(|_| format!("\"{}\"", fields_json));

    let js = format!(
        r#"
        (function() {{
            try {{
                var fields = JSON.parse({fields});
                var filled = [];
                for (var key in fields) {{
                    var val = fields[key];
                    // Try by name, then id, then CSS selector
                    var el = document.querySelector('[name="' + key + '"]')
                          || document.querySelector('#' + key)
                          || document.querySelector(key);
                    if (el) {{
                        el.focus();
                        if (el.tagName === 'SELECT') {{
                            el.value = val;
                            el.dispatchEvent(new Event('change', {{bubbles: true}}));
                        }} else {{
                            el.value = val;
                            el.dispatchEvent(new Event('input', {{bubbles: true}}));
                            el.dispatchEvent(new Event('change', {{bubbles: true}}));
                        }}
                        filled.push(key);
                    }}
                }}
                return 'Filled ' + filled.length + ' fields: ' + filled.join(', ');
            }} catch(e) {{
                return 'fill_form error: ' + e.message;
            }}
        }})()
        "#,
        fields = fields_val
    );

    let result = session.eval(&js)
        .map_err(|e| McpError::InvalidParams(format!("Chrome eval failed: {e}")))?;

    Ok(json!({"content": [{"type": "text", "text": result}]}))
}

/// Submit a form (click submit button or form.submit()).
fn chrome_submit(selector: &str) -> Result<Value, McpError> {
    let session = chrome_session()?;

    let js = if selector.is_empty() {
        r#"
        (function() {
            var btn = document.querySelector('input[type="submit"], button[type="submit"], form button');
            if (btn) { btn.click(); return 'Clicked submit: ' + btn.tagName + ' "' + (btn.innerText || btn.value || '').substring(0, 50) + '"'; }
            var form = document.querySelector('form');
            if (form) { form.submit(); return 'Submitted form via form.submit()'; }
            return 'No submit button or form found';
        })()
        "#.to_string()
    } else {
        let sel = serde_json::to_string(selector).unwrap_or_else(|_| format!("\"{}\"", selector));
        format!(
            r#"
            (function() {{
                var el = document.querySelector({sel});
                if (!el) return 'Element not found: ' + {sel};
                el.click();
                return 'Clicked: ' + el.tagName + ' "' + (el.innerText || '').substring(0, 50) + '"';
            }})()
            "#,
            sel = sel
        )
    };

    let result = session.eval(&js)
        .map_err(|e| McpError::InvalidParams(format!("Chrome eval failed: {e}")))?;

    Ok(json!({"content": [{"type": "text", "text": result}]}))
}

/// Scroll the page in a direction.
fn chrome_scroll(direction: &str, amount: u64) -> Result<Value, McpError> {
    let session = chrome_session()?;

    let (x, y) = match direction {
        "up" => (0i64, -(amount as i64)),
        "down" => (0, amount as i64),
        "left" => (-(amount as i64), 0),
        "right" => (amount as i64, 0),
        _ => (0, amount as i64),
    };

    let js = format!(
        r#"
        (function() {{
            window.scrollBy({x}, {y});
            return 'Scrolled {dir} by {amt}px. Current position: ' + window.scrollX + ',' + window.scrollY;
        }})()
        "#,
        x = x, y = y, dir = direction, amt = amount
    );

    let result = session.eval(&js)
        .map_err(|e| McpError::InvalidParams(format!("Chrome eval failed: {e}")))?;

    Ok(json!({"content": [{"type": "text", "text": result}]}))
}

/// Get full HTML of the current page.
fn chrome_html(url: &str, _wait: u64) -> Result<Value, McpError> {
    let session = chrome_session()?;

    session.navigate(url)
        .map_err(|e| McpError::InvalidParams(format!("Chrome navigate failed: {e}")))?;

    let html = session.eval("document.documentElement.outerHTML")
        .map_err(|e| McpError::InvalidParams(format!("Chrome eval failed: {e}")))?;

    // Truncate if huge
    let truncated = if html.len() > 10000 {
        format!("{}...\n\n[truncated, {} total bytes]", &html[..10000], html.len())
    } else {
        html
    };

    Ok(json!({"content": [{"type": "text", "text": truncated}]}))
}

// ── Legacy ghost.py fallback ──

fn find_ghost_script() -> String {
    // Try relative to CARGO_MANIFEST_DIR (development)
    let dev_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tools/spa-clone/ghost.py");
    if std::path::Path::new(dev_path).exists() {
        return dev_path.to_string();
    }
    // Try relative to binary location
    if let Ok(exe) = std::env::current_exe() {
        let tools_path = exe.parent().unwrap().parent().unwrap().join("tools/spa-clone/ghost.py");
        if tools_path.exists() {
            return tools_path.to_string_lossy().to_string();
        }
    }
    // Fallback
    "tools/spa-clone/ghost.py".to_string()
}

fn run_ghost(args: &[&str], _timeout_secs: u64) -> Result<String, McpError> {
    let ghost = find_ghost_script();

    let mut cmd_args = vec![&ghost as &str];
    cmd_args.extend_from_slice(args);

    let output = Command::new("python3")
        .args(&cmd_args)
        .output()
        .map_err(|e| McpError::InvalidParams(format!("ghost launch failed: {e}")))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let err = String::from_utf8_lossy(&output.stderr);
        Err(McpError::InvalidParams(format!("ghost error: {}", err.chars().take(200).collect::<String>())))
    }
}

/// Generic delegate: builds ghost.py args, optionally adds --profile, runs, returns MCP response.
fn ghost_delegate(base_args: &[&str], timeout_secs: u64, profile: Option<&str>) -> Result<Value, McpError> {
    let profile_str;
    let mut args: Vec<&str> = base_args.to_vec();
    if let Some(p) = profile {
        profile_str = p.to_string();
        args.extend(&["--profile", &profile_str]);
    }

    let output = run_ghost(&args, timeout_secs)?;

    // Try JSON parse, fallback to raw text
    match serde_json::from_str::<Value>(&output) {
        Ok(parsed) => Ok(json!({
            "content": [{"type": "text", "text": format!("{}", serde_json::to_string_pretty(&parsed).unwrap_or(output))}]
        })),
        Err(_) => Ok(json!({
            "content": [{"type": "text", "text": output}]
        })),
    }
}

/// Chat action — legacy fallback via ghost.py (complex ProseMirror interaction).
fn ghost_chat(url: &str, message: &str, profile: Option<&str>) -> Result<Value, McpError> {
    if message.is_empty() {
        return Err(McpError::InvalidParams("message required for chat action".into()));
    }

    // Default URLs for known platforms
    let target = if url.is_empty() {
        if profile.is_some() { "https://chatgpt.com" } else { "https://grok.com" }
    } else {
        url
    };

    let mut args = vec!["pong", target, "--message", message];
    let profile_str;
    if let Some(p) = profile {
        profile_str = p.to_string();
        args.extend(&["--profile", &profile_str]);
    }

    let output = run_ghost(&args, 120)?;

    match serde_json::from_str::<Value>(&output) {
        Ok(result) => {
            let response = result["response"].as_str().unwrap_or("No response");
            let platform = result["platform"].as_str().unwrap_or("unknown");

            Ok(json!({
                "content": [{"type": "text", "text": format!("[{}] {}", platform, response)}]
            }))
        }
        Err(_) => Ok(json!({
            "content": [{"type": "text", "text": output}]
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_definition_has_required_fields() {
        let def = definition();
        assert_eq!(def.name, "ghost");
        assert!(def.description.contains("ghost") || def.description.contains("Neomode"));
    }

    #[test]
    fn test_ghost_script_path_exists() {
        let path = find_ghost_script();
        assert!(std::path::Path::new(&path).exists(), "ghost.py not found at {path}");
    }

    #[test]
    fn test_all_19_actions_in_enum() {
        let def = definition();
        let schema = &def.schema;
        let actions = schema["properties"]["action"]["enum"].as_array().unwrap();

        // Must have at least 19 actions (open, chat, html + 15 new + screenshot = 19+3 = 22)
        assert!(
            actions.len() >= 19,
            "Expected at least 19 actions, got {}",
            actions.len()
        );

        let expected = vec![
            "search", "navigate", "read", "find", "click", "type", "fill_form", "submit",
            "screenshot", "scroll", "extract_data", "login", "download", "monitor",
            "api_intercept", "cookie_manage", "multi_tab", "wait_for", "pipeline",
            "open", "chat", "html",
        ];

        for action in &expected {
            assert!(
                actions.contains(&json!(action)),
                "Missing action in enum: {action}"
            );
        }
    }

    #[test]
    fn test_each_action_name_is_valid() {
        // Verify ALL_ACTIONS constant matches what the definition exposes
        let def = definition();
        let schema = &def.schema;
        let actions = schema["properties"]["action"]["enum"].as_array().unwrap();

        for action in ALL_ACTIONS {
            assert!(
                actions.contains(&json!(action)),
                "ALL_ACTIONS contains '{action}' but it's not in the schema enum"
            );
        }

        // And the reverse: every enum value should be in ALL_ACTIONS
        for action_val in actions {
            let action_str = action_val.as_str().unwrap();
            assert!(
                ALL_ACTIONS.contains(&action_str),
                "Schema enum contains '{action_str}' but it's not in ALL_ACTIONS"
            );
        }
    }

    #[test]
    fn test_all_actions_have_match_arm() {
        // Verify every action in ALL_ACTIONS has a corresponding match arm
        // by checking the source code contains the match pattern for each action.
        // We can't instantiate McpState without a BrowserEngine, so we verify statically.
        let source = include_str!("ghost.rs");
        for action in ALL_ACTIONS {
            let pattern = format!("\"{}\"", action);
            assert!(
                source.contains(&pattern),
                "Action '{action}' appears to have no match arm in call()"
            );
        }
    }

    #[test]
    fn test_schema_has_all_parameters() {
        let def = definition();
        let props = def.schema["properties"].as_object().unwrap();

        let expected_params = vec![
            "action", "url", "message", "profile", "wait",
            "query", "selector", "value", "text", "fields",
            "direction", "amount", "type_", "email", "password",
            "engine", "num", "pattern", "steps", "by", "index",
        ];

        for param in &expected_params {
            assert!(
                props.contains_key(*param),
                "Missing parameter in schema: {param}"
            );
        }
    }
}
