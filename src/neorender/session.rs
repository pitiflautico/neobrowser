//! NeoSession — persistent headless browser session.
//!
//! Unlike render_page (create/destroy per call), NeoSession keeps the V8 runtime,
//! HTTP client, and cookie jar alive across navigations. This enables:
//! - Session cookies persisting between pages
//! - Module cache surviving across goto() calls
//! - eval() in the current page context
//! - fetch() using the same Chrome TLS fingerprint + cookies

use deno_core::JsRuntime;
use std::rc::Rc;
use std::cell::RefCell;
use std::sync::Arc;

use crate::ghost;
use super::v8_runtime::{self, ScriptStoreHandle};
use super::net::BrowserNetwork;
use super::storage::BrowserStorage;

/// Shared HTTP client handle — stored in V8's OpState so fetch ops use the session's client.
/// Kept for backward compatibility with code that stores SharedClient directly.
pub type SharedClient = Arc<rquest::Client>;

/// Current page origin — stored in V8's OpState so fetch ops add proper browser headers.
/// Kept for backward compatibility with code that stores PageOrigin directly.
#[derive(Clone)]
pub struct PageOrigin {
    pub origin: String,  // e.g. "https://chatgpt.com"
    pub url: String,     // e.g. "https://chatgpt.com/"
}

/// Persistent browser session: V8 runtime + HTTP client + cookies.
pub struct NeoSession {
    runtime: JsRuntime,
    store: ScriptStoreHandle,
    network: BrowserNetwork,    // Fetch Standard networking (replaces raw client + PageOrigin)
    cookies: ghost::CookieJar,
    storage: Option<Arc<BrowserStorage>>,  // SQLite-backed localStorage
    url: String,
    navigated: bool, // true after first goto()
}

/// Result from a goto() navigation.
pub struct PageResult {
    pub url: String,
    pub status: u16,
    pub title: String,
    pub text: String,
    pub html_len: usize,
    pub scripts_count: usize,
    pub render_time_ms: u64,
    pub errors: Vec<String>,
    /// WOM data extracted directly from linkedom (avoids html5ever re-parse).
    /// Contains links, forms, inputs, buttons, headings, images, meta as JSON.
    pub wom: Option<serde_json::Value>,
}

impl NeoSession {
    /// Create a new session. Optionally loads cookies from a JSON file.
    /// Does NOT navigate — call goto() after creation.
    pub fn new(cookies_file: Option<&str>) -> Result<Self, String> {
        // 1. Build rquest client with Chrome TLS + cookie store
        let client = rquest::Client::builder()
            .impersonate(rquest::Impersonate::Chrome131)
            .cookie_store(true)
            .redirect(rquest::redirect::Policy::limited(10))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| format!("Client build error: {e}"))?;
        let client = Arc::new(client);

        // 2. Create BrowserNetwork
        let network = BrowserNetwork::new(client.clone());

        // 3. Load cookies if provided
        let mut cookies = ghost::CookieJar::new();
        if let Some(path) = cookies_file {
            match cookies.load_file(path) {
                Ok(n) => eprintln!("[NEOSESSION] Loaded {n} cookies from {path}"),
                Err(e) => eprintln!("[NEOSESSION] Cookie load warning: {e}"),
            }
        }
        // Also check NEOBROWSER_COOKIES env
        if let Ok(paths) = std::env::var("NEOBROWSER_COOKIES") {
            for path in paths.split(',') {
                let path = path.trim();
                if !path.is_empty() {
                    cookies.load_file(path).ok();
                }
            }
        }

        // 4. Create V8 runtime with empty HTML (no navigation yet)
        let empty_html = "<html><head></head><body></body></html>";
        let empty_url = "about:blank";
        let (mut runtime, store) = v8_runtime::create_runtime_with_html(
            empty_html, empty_url, &cookies, None,
        )?;

        // 5. Initialize SQLite-backed localStorage
        let storage = match BrowserStorage::new() {
            Ok(s) => {
                eprintln!("[NEOSESSION] SQLite localStorage ready");
                Some(Arc::new(s))
            }
            Err(e) => {
                eprintln!("[NEOSESSION] localStorage warning (in-memory fallback): {e}");
                None
            }
        };

        // 6. Store BrowserNetwork handle + storage in V8's OpState
        {
            let op_state = runtime.op_state();
            let mut state = op_state.borrow_mut();
            state.put::<super::net::BrowserNetworkHandle>(network.to_handle());
            if let Some(ref s) = storage {
                state.put::<super::ops::StorageHandle>(super::ops::StorageHandle(s.clone()));
                state.put::<super::ops::StorageDomain>(super::ops::StorageDomain(String::new()));
            }
        }

        Ok(Self {
            runtime,
            store,
            network,
            cookies,
            storage,
            url: String::new(),
            navigated: false,
        })
    }

    /// Navigate to a URL. Reuses the existing V8 runtime — just replaces the DOM.
    pub async fn goto(&mut self, url: &str) -> Result<PageResult, String> {
        let start = std::time::Instant::now();

        // 1. Build headers with cookies
        let mut headers = rquest::header::HeaderMap::new();
        if let Some(domain) = url::Url::parse(url).ok().and_then(|u| u.host_str().map(|s| s.to_string())) {
            if let Some(cookie_header) = self.cookies.header_for(&domain) {
                if let Ok(v) = rquest::header::HeaderValue::from_str(&cookie_header) {
                    headers.insert(rquest::header::COOKIE, v);
                }
            }
        }

        // 2. Fetch with the persistent client
        let resp = self.network.client().get(url)
            .headers(headers)
            .send()
            .await
            .map_err(|e| format!("HTTP error: {e}"))?;

        let status = resp.status().as_u16();
        let final_url = resp.url().to_string();

        // Store response cookies
        if let Some(domain) = resp.url().host_str() {
            for cookie in resp.headers().get_all(rquest::header::SET_COOKIE) {
                if let Ok(s) = cookie.to_str() {
                    self.cookies.store_from_header(domain, s);
                }
            }
        }

        let html = resp.text().await.map_err(|e| format!("Body error: {e}"))?;

        // 3. WAF check
        if let Some(waf) = super::detect_waf_challenge(&html) {
            return Ok(PageResult {
                url: final_url,
                status,
                title: String::new(),
                text: String::new(),
                html_len: html.len(),
                scripts_count: 0,
                render_time_ms: 0,
                errors: vec![format!("WAF challenge: {waf}")],
                wom: None,
            });
        }

        let html_len = html.len();

        // 4. Extract scripts
        let mut all_scripts = super::extract_all_scripts(&html, &final_url);
        let ext_count = all_scripts.iter().filter(|s| s.url.is_some()).count();
        let mod_count = all_scripts.iter().filter(|s| s.is_module).count();
        eprintln!("[NEOSESSION] {} scripts ({} external, {} modules) in {}",
            all_scripts.len(), ext_count, mod_count, &final_url[..final_url.len().min(80)]);

        // 5. Fetch external scripts using the persistent client
        for script in all_scripts.iter_mut() {
            if let Some(script_url) = &script.url {
                match tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    self.network.client().get(script_url).send(),
                ).await {
                    Ok(Ok(resp)) => {
                        if let Ok(text) = resp.text().await {
                            script.content = Some(text);
                        }
                    }
                    _ => eprintln!("[NEOSESSION] Skip slow script: {}", script_url),
                }
            }
        }

        // 6. Pre-populate module store with fetched scripts
        {
            let mut s = self.store.borrow_mut();
            for script in &all_scripts {
                if let (Some(url), Some(content)) = (&script.url, &script.content) {
                    s.scripts.insert(url.clone(), content.clone());
                }
            }
        }

        // 7. Pre-fetch ES module imports (depth 3)
        {
            let mut to_scan: Vec<(String, String)> = Vec::new();
            for script in &all_scripts {
                if script.is_module {
                    if let (Some(url), Some(content)) = (&script.url, &script.content) {
                        to_scan.push((url.clone(), content.clone()));
                    }
                }
            }
            for _depth in 0..3 {
                let mut next_round = Vec::new();
                for (script_url, content) in &to_scan {
                    let imports = super::extract_es_imports(content, script_url);
                    for import_url in imports {
                        if self.store.borrow().scripts.contains_key(&import_url) { continue; }
                        match tokio::time::timeout(
                            std::time::Duration::from_secs(15),
                            self.network.client().get(&import_url).send(),
                        ).await {
                            Ok(Ok(resp)) => {
                                if let Ok(text) = resp.text().await {
                                    self.store.borrow_mut().scripts.insert(import_url.clone(), text.clone());
                                    next_round.push((import_url, text));
                                }
                            }
                            _ => eprintln!("[NEOSESSION] Skip slow import: {}", import_url),
                        }
                    }
                }
                to_scan = next_round;
                if to_scan.is_empty() { break; }
            }
        }

        // 8. Replace DOM content in the existing runtime
        //    Instead of creating a new document (breaks references), update the existing one
        let html_json = serde_json::to_string(&html).unwrap_or_default();
        let inject_js = format!("globalThis.__neo_html = {};", html_json);
        self.runtime.execute_script("<neosession:inject_html>", inject_js)
            .map_err(|e| format!("HTML injection error: {e}"))?;

        let reparse_js = r#"{
            // Parse into a fresh document
            const { document: freshDoc } = __linkedom_parseHTML(globalThis.__neo_html);
            // Copy head and body content into the existing document
            // (preserves document identity — no broken references)
            if (freshDoc.head) document.head.innerHTML = freshDoc.head.innerHTML;
            if (freshDoc.body) document.body.innerHTML = freshDoc.body.innerHTML;
            // Copy <html> attributes
            for (const attr of freshDoc.documentElement.attributes || []) {
                try { document.documentElement.setAttribute(attr.name, attr.value); } catch {}
            }
            try { Object.defineProperty(document, 'currentScript', { value: null, writable: true, configurable: true }); } catch {}
            if (document.cookie === undefined) document.cookie = '';
            try { document.defaultView = globalThis; } catch {}
            try { document.location = location; } catch {}
            delete globalThis.__neo_html;
        }"#;
        self.runtime.execute_script("<neosession:reparse>", reparse_js.to_string())
            .map_err(|e| format!("DOM reparse error: {e}"))?;

        // 9. Update location
        v8_runtime::set_location(&mut self.runtime, &final_url)?;
        self.runtime.execute_script("<neosession:doc_location>",
            "document.location = location; try { document.baseURI = location.href; } catch {}".to_string()
        ).map_err(|e| format!("doc.location sync: {e}"))?;

        // 9b. Update BrowserNetwork page context + sync handle to OpState
        self.network.set_page(&final_url);
        let domain = url::Url::parse(&final_url).ok()
            .and_then(|u| u.host_str().map(|s| s.to_string()))
            .unwrap_or_default();
        {
            let op_state = self.runtime.op_state();
            let mut state = op_state.borrow_mut();
            state.put::<super::net::BrowserNetworkHandle>(self.network.to_handle());
            // Update storage domain so localStorage ops use the right namespace
            state.put::<super::ops::StorageDomain>(super::ops::StorageDomain(domain.clone()));
        }

        // 10. Update cookie injection for JS-side fetch
        let cookie_map = self.cookies.all_headers();
        if !cookie_map.is_empty() {
            let cookies_json = serde_json::to_string(&cookie_map).unwrap_or_default();
            let js = format!("globalThis.__neorender_cookies = {};", cookies_json);
            self.runtime.execute_script("<neosession:cookies>", js)
                .map_err(|e| format!("Cookie update: {e}"))?;
        }

        // 11. Execute scripts in document order
        let scripts_count = all_scripts.len();
        let mut errors = Vec::new();
        let mut first_module = true;
        for (i, script) in all_scripts.into_iter().enumerate() {
            let Some(content) = script.content else { continue };
            let script_url = script.url.as_deref().unwrap_or(&final_url);
            let name = if script.url.is_some() { format!("script:{i}") } else { format!("inline:{i}") };

            let err = if script.is_module {
                if first_module {
                    first_module = false;
                    v8_runtime::execute_module(&mut self.runtime, script_url, name).await
                } else {
                    v8_runtime::execute_side_module(&mut self.runtime, script_url, name).await
                }
            } else {
                v8_runtime::execute_script(&mut self.runtime, content, name)
            };
            if let Some(e) = err {
                errors.push(e);
            }
        }

        // 12. Fire lifecycle events
        let lifecycle_js = r#"
            try { document.dispatchEvent(new Event('DOMContentLoaded', {bubbles:true})); } catch(e){}
            try { dispatchEvent(new Event('DOMContentLoaded', {bubbles:true})); } catch(e){}
            try { dispatchEvent(new Event('load')); } catch(e){}
            try { document.readyState = 'interactive'; } catch(e){}
            try { document.readyState = 'complete'; } catch(e){}
        "#;
        self.runtime.execute_script("<neosession:lifecycle>", lifecycle_js.to_string())
            .map_err(|e| format!("Lifecycle error: {e}"))?;

        // 13. Run event loop with stability detection
        //     Poll DOM node count every 100ms. Stable = no change for 500ms (5 consecutive checks).
        //     Timeout at 15s to avoid hanging on pages with infinite timers.
        {
            let stability_timeout = std::time::Duration::from_secs(15);
            let poll_interval = std::time::Duration::from_millis(100);
            let stable_threshold = 5u32; // 5 polls * 100ms = 500ms of no change
            let stability_start = std::time::Instant::now();
            let mut last_node_count: i64 = -1;
            let mut stable_count: u32 = 0;

            loop {
                // Run event loop for one poll interval
                v8_runtime::run_event_loop(&mut self.runtime, poll_interval.as_millis() as u64).await?;

                // Check DOM node count
                let count_str = self.eval_internal(
                    "document.querySelectorAll('*').length"
                ).unwrap_or_else(|_| "0".to_string());
                let node_count: i64 = count_str.parse().unwrap_or(0);

                if node_count == last_node_count {
                    stable_count += 1;
                    if stable_count >= stable_threshold {
                        eprintln!("[NEOSESSION] DOM stable at {} nodes after {:?}",
                            node_count, stability_start.elapsed());
                        break;
                    }
                } else {
                    stable_count = 0;
                    last_node_count = node_count;
                }

                if stability_start.elapsed() >= stability_timeout {
                    eprintln!("[NEOSESSION] Stability timeout ({}ms) at {} nodes",
                        stability_timeout.as_millis(), node_count);
                    break;
                }
            }
        }

        // 14. Extract WOM (title, text, links, forms, etc.) directly from linkedom DOM
        let wom_json_str = self.eval_internal("__wom_extract()")?;
        let wom: Option<serde_json::Value> = serde_json::from_str(&wom_json_str).ok();

        let title = wom.as_ref()
            .and_then(|w| w["title"].as_str())
            .unwrap_or("")
            .to_string();
        let text = wom.as_ref()
            .and_then(|w| w["text"].as_str())
            .unwrap_or("")
            .to_string();

        let render_time = start.elapsed();
        self.url = final_url.clone();
        self.navigated = true;

        eprintln!("[NEOSESSION] Rendered in {:?} — {} bytes, {} scripts, {} errors",
            render_time, html_len, scripts_count, errors.len());

        Ok(PageResult {
            url: final_url,
            status,
            title,
            text,
            html_len,
            scripts_count,
            render_time_ms: render_time.as_millis() as u64,
            errors,
            wom,
        })
    }

    /// Execute arbitrary JS in the current page context.
    /// For async code (Promises, fetch), use eval_async.
    pub fn eval(&mut self, js: &str) -> Result<String, String> {
        if !self.navigated {
            return Err("No page loaded — call goto() first".to_string());
        }
        self.eval_internal(js)
    }

    /// Execute async JS — runs the event loop to resolve Promises.
    /// Wraps the result in a global and polls until resolved.
    pub async fn eval_async(&mut self, js: &str, timeout_ms: u64) -> Result<String, String> {
        if !self.navigated {
            return Err("No page loaded — call goto() first".to_string());
        }
        // Wrap in async executor: store result in global when resolved
        let wrapped = format!(
            r#"globalThis.__eval_done = false; globalThis.__eval_result = "pending";
            Promise.resolve((async () => {{ {} }})()).then(
                r => {{ globalThis.__eval_result = typeof r === 'string' ? r : JSON.stringify(r); globalThis.__eval_done = true; }},
                e => {{ globalThis.__eval_result = "error:" + e; globalThis.__eval_done = true; }}
            );"#,
            js
        );
        self.runtime.execute_script("<neosession:eval_async>", wrapped)
            .map_err(|e| format!("Eval async error: {e}"))?;

        // Run event loop until done or timeout
        v8_runtime::run_event_loop(&mut self.runtime, timeout_ms).await?;

        // Read result
        self.eval_internal("globalThis.__eval_result")
    }

    /// Internal eval — works even before navigation.
    fn eval_internal(&mut self, js: &str) -> Result<String, String> {
        let result = self.runtime.execute_script("<neosession:eval>", js.to_string())
            .map_err(|e| format!("Eval error: {e}"))?;

        let scope = &mut self.runtime.handle_scope();
        let local = deno_core::v8::Local::new(scope, result);
        if let Some(s) = local.to_string(scope) {
            Ok(s.to_rust_string_lossy(scope))
        } else {
            Ok("undefined".to_string())
        }
    }

    /// HTTP request using the session's BrowserNetwork (same TLS fingerprint + Fetch Standard headers).
    pub async fn fetch(
        &self,
        url: &str,
        method: &str,
        body: Option<&str>,
        headers: Option<&str>,
    ) -> Result<String, String> {
        // Build a merged headers JSON that includes session cookies + any custom headers
        let merged_headers = {
            let mut hdrs = serde_json::Map::new();

            // Inject session cookies
            if let Some(domain) = url::Url::parse(url).ok().and_then(|u| u.host_str().map(|s| s.to_string())) {
                if let Some(cookie_header) = self.cookies.header_for(&domain) {
                    hdrs.insert("cookie".to_string(), serde_json::Value::String(cookie_header));
                }
            }

            // Merge custom headers (override cookies if explicitly set)
            if let Some(headers_json) = headers {
                if !headers_json.is_empty() {
                    if let Ok(custom) = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(headers_json) {
                        for (k, v) in custom {
                            hdrs.insert(k, v);
                        }
                    }
                }
            }

            if hdrs.is_empty() { None } else { Some(serde_json::Value::Object(hdrs).to_string()) }
        };

        let resp = self.network.fetch(
            url,
            &method.to_uppercase(),
            body,
            merged_headers.as_deref(),
            super::net::RequestMode::Cors,
            super::net::RequestDestination::Empty,
        ).await?;

        Ok(serde_json::json!({
            "status": resp.status,
            "body": resp.body,
        }).to_string())
    }

    /// Export current DOM as HTML string.
    pub fn export_html(&mut self) -> Result<String, String> {
        self.eval_internal("globalThis.document.documentElement.outerHTML")
    }

    /// Export visible text from current DOM.
    pub fn export_text(&mut self) -> Result<String, String> {
        self.eval_internal(
            "document.body ? document.body.innerText || document.body.textContent || '' : ''"
        )
    }

    /// Extract WOM (Web Object Model) from current DOM — links, forms, buttons, etc.
    /// Runs __wom_extract() in V8, returns parsed JSON.
    pub fn extract_wom(&mut self) -> Result<serde_json::Value, String> {
        let json_str = self.eval_internal("__wom_extract()")?;
        serde_json::from_str(&json_str).map_err(|e| format!("WOM parse error: {e}"))
    }

    /// Current URL.
    pub fn current_url(&self) -> &str {
        &self.url
    }

    /// Is the session navigated to a page?
    pub fn is_navigated(&self) -> bool {
        self.navigated
    }

    /// Get a reference to the cookie jar.
    pub fn cookies(&self) -> &ghost::CookieJar {
        &self.cookies
    }

    /// Get a mutable reference to the cookie jar (for loading additional cookies).
    pub fn cookies_mut(&mut self) -> &mut ghost::CookieJar {
        &mut self.cookies
    }
}
