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
use super::cookie_jar::{UnifiedCookieJar, CookieJarHandle};
use super::http_cache::HttpCache;

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
    cookies: ghost::CookieJar,  // Legacy jar (backward compat for ghost path)
    cookie_jar: CookieJarHandle, // Unified SQLite-backed jar (source of truth)
    storage: Option<Arc<BrowserStorage>>,  // SQLite-backed localStorage
    cache: HttpCache,            // HTTP response cache (50MB default, LRU eviction)
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
    /// AI-optimized fields (PDR v4)
    pub page_type: String,
    pub compressed_content: String,
    pub actions: Vec<Action>,
}

/// Interactable element extracted from the page.
pub struct Action {
    pub action_type: String,  // "link", "button", "input", "select"
    pub text: String,
    pub target: String,       // selector or href
}

/// Check if a URL should be skipped during resource fetching (images, CSS, fonts, analytics).
fn should_skip_resource(url: &str) -> bool {
    let lower = url.to_lowercase();
    // Skip non-JS resources
    lower.ends_with(".css") || lower.ends_with(".png") || lower.ends_with(".jpg") ||
    lower.ends_with(".jpeg") || lower.ends_with(".gif") || lower.ends_with(".svg") ||
    lower.ends_with(".ico") || lower.ends_with(".woff") || lower.ends_with(".woff2") ||
    lower.ends_with(".ttf") || lower.ends_with(".eot") || lower.ends_with(".mp4") ||
    lower.ends_with(".webm") || lower.ends_with(".mp3") ||
    // Skip analytics/tracking
    lower.contains("google-analytics") || lower.contains("googletagmanager") ||
    lower.contains("gtag/js") || lower.contains("analytics") ||
    lower.contains("tracking") || lower.contains("pixel") ||
    lower.contains("facebook.net/en_us/fbevents") ||
    lower.contains("hotjar") || lower.contains("sentry") ||
    lower.contains("newrelic") || lower.contains("segment.com") ||
    lower.contains("doubleclick") || lower.contains("adsense") ||
    lower.contains("adsbygoogle")
}

impl NeoSession {
    /// Create a new session. Optionally loads cookies from a JSON file.
    /// Does NOT navigate — call goto() after creation.
    pub fn new(cookies_file: Option<&str>) -> Result<Self, String> {
        // 1. Build rquest client with Chrome TLS + cookie store
        let client = rquest::Client::builder()
            .emulation(rquest_util::Emulation::Chrome136)
            .cookie_store(true)
            .redirect(rquest::redirect::Policy::limited(10))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| format!("Client build error: {e}"))?;
        let client = Arc::new(client);

        // 2. Create BrowserNetwork
        let network = BrowserNetwork::new(client.clone());

        // 3. Create UnifiedCookieJar (SQLite-backed, source of truth)
        let cookie_jar = Arc::new(UnifiedCookieJar::new()?);

        // 3b. Load cookies into unified jar
        let mut cookies = ghost::CookieJar::new(); // Legacy jar kept for compat
        if let Some(path) = cookies_file {
            match cookie_jar.load_from_file(path) {
                Ok(n) => eprintln!("[NEOSESSION] Loaded {n} cookies into unified jar from {path}"),
                Err(e) => eprintln!("[NEOSESSION] Cookie load warning: {e}"),
            }
            // Also load into legacy jar for backward compat
            cookies.load_file(path).ok();
        }
        // Also check NEOBROWSER_COOKIES env
        if let Ok(paths) = std::env::var("NEOBROWSER_COOKIES") {
            for path in paths.split(',') {
                let path = path.trim();
                if !path.is_empty() {
                    cookie_jar.load_from_file(path).ok();
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

        // 6. Store BrowserNetwork handle + storage + cookie jar in V8's OpState
        {
            let op_state = runtime.op_state();
            let mut state = op_state.borrow_mut();
            state.put::<super::net::BrowserNetworkHandle>(network.to_handle());
            state.put::<super::ops::CookieJarOpHandle>(super::ops::CookieJarOpHandle(cookie_jar.clone()));
            state.put::<super::ops::StorageDomain>(super::ops::StorageDomain(String::new()));
            if let Some(ref s) = storage {
                state.put::<super::ops::StorageHandle>(super::ops::StorageHandle(s.clone()));
            }
        }

        Ok(Self {
            runtime,
            store,
            network,
            cookies,
            cookie_jar,
            storage,
            cache: HttpCache::new(50),
            url: String::new(),
            navigated: false,
        })
    }

    /// Navigate to a URL. Reuses the existing V8 runtime — just replaces the DOM.
    pub async fn goto(&mut self, url: &str) -> Result<PageResult, String> {
        self.navigate(url, "GET", None, None).await
    }

    /// Submit a form via POST (or GET). Called by goto() for GET, and by form submission for POST.
    pub async fn submit_raw(&mut self, url: &str, method: &str, body: Option<&str>, content_type: Option<&str>) -> Result<PageResult, String> {
        self.navigate(url, method, body, content_type).await
    }

    /// Unified navigation pipeline — all navigations go through here.
    /// 1. Build request with cookies from unified jar
    /// 2. Check HTTP cache / send HTTP request
    /// 3. Process Set-Cookie → unified jar
    /// 4. Handle redirects (rquest follows automatically with cookies)
    /// 5. Parse HTML
    /// 6. Execute scripts
    /// 7. Auto-consent
    /// 8. Extract WOM
    async fn navigate(&mut self, url: &str, method: &str, body: Option<&str>, content_type: Option<&str>) -> Result<PageResult, String> {
        let start = std::time::Instant::now();

        // 1. Build headers with cookies from unified jar + Chrome navigation headers
        let mut headers = rquest::header::HeaderMap::new();
        // Navigation headers — match a real Chrome 136 browser
        headers.insert(rquest::header::ACCEPT,
            rquest::header::HeaderValue::from_static("text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7"));
        headers.insert(rquest::header::ACCEPT_LANGUAGE,
            rquest::header::HeaderValue::from_static("es-ES,es;q=0.9,en;q=0.8"));
        headers.insert(rquest::header::ACCEPT_ENCODING,
            rquest::header::HeaderValue::from_static("gzip, deflate, br, zstd"));
        headers.insert(rquest::header::UPGRADE_INSECURE_REQUESTS,
            rquest::header::HeaderValue::from_static("1"));
        headers.insert(rquest::header::CACHE_CONTROL,
            rquest::header::HeaderValue::from_static("max-age=0"));
        headers.insert("Sec-Ch-Ua",
            rquest::header::HeaderValue::from_static("\"Chromium\";v=\"136\", \"Not_A Brand\";v=\"24\", \"Google Chrome\";v=\"136\""));
        headers.insert("Sec-Ch-Ua-Mobile",
            rquest::header::HeaderValue::from_static("?0"));
        headers.insert("Sec-Ch-Ua-Platform",
            rquest::header::HeaderValue::from_static("\"macOS\""));
        headers.insert("Sec-Fetch-Dest",
            rquest::header::HeaderValue::from_static("document"));
        headers.insert("Sec-Fetch-Mode",
            rquest::header::HeaderValue::from_static("navigate"));
        headers.insert("Sec-Fetch-Site",
            rquest::header::HeaderValue::from_static("none"));
        headers.insert("Sec-Fetch-User",
            rquest::header::HeaderValue::from_static("?1"));
        if let Some(cookie_header) = self.cookie_jar.cookie_header_for(url) {
            if let Ok(v) = rquest::header::HeaderValue::from_str(&cookie_header) {
                headers.insert(rquest::header::COOKIE, v);
            }
        }
        if let Some(ct) = content_type {
            if let Ok(v) = rquest::header::HeaderValue::from_str(ct) {
                headers.insert(rquest::header::CONTENT_TYPE, v);
            }
        }

        let is_get = method.eq_ignore_ascii_case("GET");

        // 2. HTTP cache check + send request
        let (status, final_url, html): (u16, String, String) = if is_get {
            // Check for fresh cached response
            if let Some(cached) = self.cache.get(url) {
                eprintln!("[NEOSESSION] Cache HIT: {}", &url[..url.len().min(80)]);
                (cached.status, url.to_string(), cached.body)
            } else {
                // Add conditional headers (If-None-Match, If-Modified-Since) for revalidation
                let cond_headers: std::collections::HashMap<String, String> = self.cache.conditional_headers(url);
                for (k, v) in &cond_headers {
                    if let (Ok(name), Ok(val)) = (
                        rquest::header::HeaderName::from_bytes(k.as_bytes()),
                        rquest::header::HeaderValue::from_str(v),
                    ) {
                        headers.insert(name, val);
                    }
                }

                let resp = self.network.client().get(url).headers(headers).send().await
                    .map_err(|e| format!("HTTP error: {e}"))?;
                let resp_status = resp.status().as_u16();
                let resp_final_url = resp.url().to_string();

                // Process Set-Cookie
                for cookie in resp.headers().get_all(rquest::header::SET_COOKIE) {
                    if let Ok(s) = cookie.to_str() {
                        self.cookie_jar.store_from_header(&resp_final_url, s);
                        if let Some(domain) = resp.url().host_str() {
                            self.cookies.store_from_header(domain, s);
                        }
                    }
                }

                // 304 Not Modified — use cached body
                if resp_status == 304 {
                    self.cache.touch(url);
                    if let Some(cached) = self.cache.get(url) {
                        eprintln!("[NEOSESSION] Cache 304 revalidated: {}", &url[..url.len().min(80)]);
                        (cached.status, resp_final_url, cached.body)
                    } else {
                        let resp_html = resp.text().await.map_err(|e| format!("Body error: {e}"))?;
                        (resp_status, resp_final_url, resp_html)
                    }
                } else {
                    // Collect response headers for cache decision
                    let resp_headers_map: std::collections::HashMap<String, String> = resp.headers().iter()
                        .filter_map(|(k, v)| {
                            v.to_str().ok().map(|vs| (k.as_str().to_string(), vs.to_string()))
                        })
                        .collect();
                    let resp_html = resp.text().await.map_err(|e| format!("Body error: {e}"))?;
                    self.cache.store(&resp_final_url, resp_status, &resp_headers_map, &resp_html);
                    (resp_status, resp_final_url, resp_html)
                }
            }
        } else {
            // Non-GET: no caching
            let resp = {
                let mut req = self.network.client().post(url).headers(headers);
                if let Some(b) = body { req = req.body(b.to_string()); }
                req.send().await.map_err(|e| format!("HTTP error: {e}"))?
            };
            let resp_status = resp.status().as_u16();
            let resp_final_url = resp.url().to_string();
            for cookie in resp.headers().get_all(rquest::header::SET_COOKIE) {
                if let Ok(s) = cookie.to_str() {
                    self.cookie_jar.store_from_header(&resp_final_url, s);
                    if let Some(domain) = resp.url().host_str() {
                        self.cookies.store_from_header(domain, s);
                    }
                }
            }
            let resp_html = resp.text().await.map_err(|e| format!("Body error: {e}"))?;
            (resp_status, resp_final_url, resp_html)
        };

        // 2b. Log this navigation request in V8's network log (so __neo_get_network_log captures it)
        let log_js = format!(
            "if(globalThis.__neo_network_log)globalThis.__neo_network_log.push({{method:'GET',url:{},status:{},size:{},duration:0,timestamp:Date.now()}});",
            serde_json::to_string(&final_url).unwrap_or_default(), status, html.len()
        );
        self.runtime.execute_script("<neosession:netlog>", log_js).ok();

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
                page_type: String::new(),
                compressed_content: String::new(),
                actions: Vec::new(),
            });
        }

        let html_len = html.len();

        // 4. Extract scripts
        let mut all_scripts = super::extract_all_scripts(&html, &final_url);
        let ext_count = all_scripts.iter().filter(|s| s.url.is_some()).count();
        let mod_count = all_scripts.iter().filter(|s| s.is_module).count();
        eprintln!("[NEOSESSION] {} scripts ({} external, {} modules) in {}",
            all_scripts.len(), ext_count, mod_count, &final_url[..final_url.len().min(80)]);

        // 5. Fetch external scripts using the persistent client (skip non-essential resources)
        let mut skipped_resources = 0usize;
        for script in all_scripts.iter_mut() {
            if let Some(script_url) = &script.url {
                if should_skip_resource(script_url) {
                    skipped_resources += 1;
                    continue;
                }
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
        if skipped_resources > 0 {
            eprintln!("[NEOSESSION] Skipped {} non-essential resources", skipped_resources);
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

        // 10. Update cookie injection for JS-side fetch (uses both unified jar and legacy map)
        let cookie_map = self.cookie_jar.all_headers();
        if !cookie_map.is_empty() {
            let cookies_json = serde_json::to_string(&cookie_map).unwrap_or_default();
            let js = format!("globalThis.__neorender_cookies = {};", cookies_json);
            self.runtime.execute_script("<neosession:cookies>", js)
                .map_err(|e| format!("Cookie update: {e}"))?;
        }

        // 10b. After inline scripts create __reactRouterContext.stream with data,
        // replace it with a pre-resolved stream that doesn't use pipeThrough.
        // The original stream has chunks in _queue but pipeThrough creates async
        // V8 promises that block module evaluation.
        self.runtime.execute_script("<neosession:fix_ssr_stream>", r#"
            try {
                const ctx = window.__reactRouterContext;
                if (ctx?.stream?._queue?.length > 0) {
                    // Extract raw chunks from queue before pipeThrough consumed them
                    const rawChunks = [...ctx.stream._queue];
                    const encoder = new TextEncoder();
                    // Create a new simple stream with all data pre-loaded + closed
                    ctx.stream = new ReadableStream({
                        start(controller) {
                            for (const chunk of rawChunks) {
                                // Encode strings to Uint8Array (turbo-stream expects bytes)
                                if (typeof chunk === 'string') {
                                    controller.enqueue(encoder.encode(chunk));
                                } else if (chunk instanceof Uint8Array) {
                                    controller.enqueue(chunk);
                                } else {
                                    // Object with numeric keys = byte-like
                                    const bytes = new Uint8Array(Object.keys(chunk).length);
                                    for (const [k, v] of Object.entries(chunk)) bytes[parseInt(k)] = v;
                                    controller.enqueue(bytes);
                                }
                            }
                            controller.close();
                        }
                    });
                }
            } catch {}
        "#.to_string()).ok();

        // 10c. Swallow unhandled promise rejections (non-fatal).
        // React Router streaming hydration causes null.then() errors that are
        // caught by deno_core as fatal. This tells deno_core to handle them silently.
        self.runtime.execute_script("<neosession:rejection_handler>", r#"
            globalThis.__neo_swallowed_rejections = [];
            Deno.core.setUnhandledPromiseRejectionHandler((promise, reason) => {
                const msg = reason?.message || String(reason);
                const stack = reason?.stack || '';
                globalThis.__neo_swallowed_rejections.push({msg, stack: stack.split('\n').slice(0,6)});
                return true;
            });
        "#.to_string()).ok();

        // 11. Execute scripts in document order
        let scripts_count = all_scripts.len();
        let mut errors = Vec::new();
        let mut first_module = true;
        for (i, script) in all_scripts.into_iter().enumerate() {
            // modulepreload scripts are pre-fetched to store but not executed.
            // They'll be loaded by V8 when the inline module imports them.
            if script.preload_only { continue; }
            let Some(content) = script.content else { continue };
            let script_url = script.url.as_deref().unwrap_or(&final_url);
            let name = if script.url.is_some() { format!("script:{i}") } else { format!("inline:{i}") };

            eprintln!("[NEOSESSION] Executing {name} module={} preload={} url={}",
                script.is_module, script.preload_only, script.url.as_deref().unwrap_or("inline"));

            // Before first module: patch React Router context + fix SSR stream
            if script.is_module && first_module {
                // Task 1.1: Make getAll() available on ANY object that gets called with it.
                // The object is created inside vendor module closure — unreachable via
                // __reactRouterContext patching. Instead, define getAll as a default
                // method on Object.prototype that returns empty array.
                // The object has 'availableHints' but no 'getAll' (not Headers — it's
                // React Router's SSR response context). In headless mode, no 103 hints.
                // NOTE: Promise.allSettled is handled via source-level transform in
                // v8_runtime.rs NeoModuleLoader — polyfill injection doesn't work in
                // deno_core 0.311 module evaluation contexts.

                // Approach: define getAll on Object.prototype as a non-enumerable fallback.
                // This catches ANY object that doesn't have its own getAll.
                // Returns [] (no early hints in headless mode).
                self.runtime.execute_script("<neosession:patch_getall>", r#"
                    if (!Object.prototype.getAll) {
                        Object.defineProperty(Object.prototype, 'getAll', {
                            value: function(name) { return []; },
                            configurable: true,
                            writable: true,
                            enumerable: false,
                        });
                    }
                "#.to_string()).ok();

                self.runtime.execute_script("<neosession:fix_stream_before_module>", r#"
                    try {
                        const ctx = window.__reactRouterContext;
                        if (ctx?.stream?._queue?.length > 0) {
                            const rawChunks = [...ctx.stream._queue];
                            const encoder = new TextEncoder();
                            ctx.stream = new ReadableStream({
                                start(controller) {
                                    for (const chunk of rawChunks) {
                                        if (typeof chunk === 'string') controller.enqueue(encoder.encode(chunk));
                                        else if (chunk instanceof Uint8Array) controller.enqueue(chunk);
                                        else {
                                            const bytes = new Uint8Array(Object.keys(chunk).length);
                                            for (const [k,v] of Object.entries(chunk)) bytes[parseInt(k)] = v;
                                            controller.enqueue(bytes);
                                        }
                                    }
                                    controller.close();
                                }
                            });
                        }
                    } catch {}
                "#.to_string()).ok();
            }

            let err = if script.is_module {
                // For inline modules: convert static imports to dynamic import()
                // to avoid top-level await blocking mod_evaluate.
                if script.url.is_none() {
                    // Convert inline ES module to async IIFE script.
                    // Static imports → dynamic import() to avoid TLA blocking.
                    use regex_lite::Regex;
                    let base = url::Url::parse(&final_url).ok()
                        .map(|u| u.origin().ascii_serialization())
                        .unwrap_or_default();

                    let mut code = content.clone();

                    // import "path" → await import("path")
                    let re_bare = Regex::new(r#"import\s*"([^"]+)""#).unwrap();
                    code = re_bare.replace_all(&code, |caps: &regex_lite::Captures| {
                        let path = &caps[1];
                        let full = if path.starts_with('/') { format!("{}{}", base, path) } else { path.to_string() };
                        format!("await import(\"{}\")", full)
                    }).to_string();

                    // import * as name from "path" → const name = await import("path")
                    let re_star = Regex::new(r#"import\s*\*\s*as\s+(\w+)\s+from\s*"([^"]+)""#).unwrap();
                    code = re_star.replace_all(&code, |caps: &regex_lite::Captures| {
                        let name = &caps[1];
                        let path = &caps[2];
                        let full = if path.starts_with('/') { format!("{}{}", base, path) } else { path.to_string() };
                        format!("const {} = await import(\"{}\")", name, full)
                    }).to_string();

                    // import { a as b, c } from "path" → const { a: b, c } = await import("path")
                    let re_named = Regex::new(r#"import\s*\{([^}]+)\}\s*from\s*"([^"]+)""#).unwrap();
                    code = re_named.replace_all(&code, |caps: &regex_lite::Captures| {
                        let imports = caps[1].replace(" as ", ": ");
                        let path = &caps[2];
                        let full = if path.starts_with('/') { format!("{}{}", base, path) } else { path.to_string() };
                        format!("const {{{}}} = await import(\"{}\")", imports, full)
                    }).to_string();

                    // Dynamic import() — add base URL, fire-and-forget (no await — TLA blocks)
                    let re_dynamic = Regex::new(r#"import\("(/[^"]+)"\)"#).unwrap();
                    code = re_dynamic.replace_all(&code, |caps: &regex_lite::Captures| {
                        let path = &caps[1];
                        format!("import(\"{}{}\").catch(()=>{{}})", base, path)
                    }).to_string();

                    // Source-level transform for Promise.allSettled (polyfill doesn't
                    // work in deno_core 0.311 module contexts)
                    let code_patched = if code.contains("Promise.allSettled(") {
                        code.replace(
                            "Promise.allSettled(",
                            "((ps)=>Promise.all([...ps].map(p=>Promise.resolve(p).then(v=>({status:'fulfilled',value:v}),r=>({status:'rejected',reason:r})))))("
                        )
                    } else {
                        code
                    };
                    let script_js = format!("(async () => {{ try {{ {}; window.__neo_iife_ok = true; }} catch(e) {{ window.__neo_iife_error = e.message + ' | ' + (e.stack?.split('\\n').slice(0,3).join(' | ') || ''); console.error?.('IIFE error:', e.message); }} }})();", code_patched);
                    eprintln!("[NEOSESSION] Inline module → async script: {}B", script_js.len());
                    v8_runtime::execute_script(&mut self.runtime, script_js, name)
                } else {
                    let module_url = script_url.to_string();
                    if first_module {
                        first_module = false;
                        v8_runtime::execute_module(&mut self.runtime, &module_url, name).await
                    } else {
                        v8_runtime::execute_side_module(&mut self.runtime, &module_url, name).await
                    }
                }
            } else {
                v8_runtime::execute_script(&mut self.runtime, content, name)
            };
            if let Some(e) = err {
                errors.push(e);
            }
        }

        // 11b. Run event loop to resolve dynamic imports from async IIFE scripts
        // The inline module converted to async script fires import() that needs event loop.
        v8_runtime::run_event_loop(&mut self.runtime, 15000).await.ok();

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
            let stability_timeout = std::time::Duration::from_secs(30);
            let poll_interval = std::time::Duration::from_millis(200);
            let stable_threshold = 15u32; // 15 polls * 200ms = 3s of no change (SPAs need time)
            let stability_start = std::time::Instant::now();
            let mut last_node_count: i64 = -1;
            let mut stable_count: u32 = 0;

            loop {
                // Run event loop for one poll interval
                v8_runtime::run_event_loop(&mut self.runtime, poll_interval.as_millis() as u64).await?;

                // Poll dynamic scripts — fetch and execute any scripts added to DOM by JS
                if let Ok(pending_json) = self.eval_internal("__neo_pending_scripts()") {
                    if let Ok(pending) = serde_json::from_str::<Vec<serde_json::Value>>(&pending_json) {
                        for entry in &pending {
                            let id = entry["id"].as_i64().unwrap_or(0);
                            let src = entry["src"].as_str().unwrap_or("");
                            let is_module = entry["module"].as_bool().unwrap_or(false);
                            if src.is_empty() { continue; }

                            // Resolve relative URLs
                            let abs_url = url::Url::parse(&final_url).ok()
                                .and_then(|base| base.join(src).ok())
                                .map(|u| u.to_string())
                                .unwrap_or_else(|| src.to_string());

                            // Fetch the script
                            match tokio::time::timeout(
                                std::time::Duration::from_secs(10),
                                self.network.client().get(&abs_url).send(),
                            ).await {
                                Ok(Ok(resp)) => {
                                    if let Ok(text) = resp.text().await {
                                        // Store in module store for ES imports
                                        self.store.borrow_mut().scripts.insert(abs_url.clone(), text.clone());
                                        // Execute
                                        let name = format!("dynamic:{id}");
                                        if is_module {
                                            v8_runtime::execute_side_module(&mut self.runtime, &abs_url, name).await;
                                        } else {
                                            v8_runtime::execute_script(&mut self.runtime, text, name);
                                        }
                                        // Notify JS
                                        let notify = format!("__neo_script_loaded({id},{});", serde_json::to_string(src).unwrap_or_default());
                                        self.runtime.execute_script("<neosession:script_loaded>", notify).ok();
                                    }
                                }
                                _ => {
                                    let notify = format!("__neo_script_error({id},{},'fetch timeout');", serde_json::to_string(src).unwrap_or_default());
                                    self.runtime.execute_script("<neosession:script_error>", notify).ok();
                                }
                            }
                        }
                        if !pending.is_empty() {
                            stable_count = 0; // Reset stability — new scripts may change DOM
                        }
                    }
                }

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

        // 13b. Auto-accept consent dialogs (cookie banners, GDPR prompts)
        if let Ok(consent_result) = self.eval_internal("__neo_auto_consent()") {
            if consent_result.contains("\"ok\":true") {
                eprintln!("[NEOSESSION] Auto-accepted consent dialog: {}", consent_result);
                // Run event loop briefly to process any DOM changes from consent acceptance
                v8_runtime::run_event_loop(&mut self.runtime, 500).await?;
            }
        }

        // 13c. Google consent redirect detection
        //      Google shows "Before you continue" / "Antes de ir a Google" on consent.google.com
        //      or on the main page itself. Detect via title and submit the consent form via HTTP,
        //      then re-navigate to get the actual content.
        {
            let title_check = self.eval_internal("document.title || ''").unwrap_or_default();
            let is_google_consent = title_check.contains("Before you continue")
                || title_check.contains("Antes de ir a Google")
                || title_check.contains("Avant d'accéder à Google")
                || title_check.contains("Bevor Sie zu Google")
                || final_url.contains("consent.google");

            if is_google_consent {
                eprintln!("[NEOSESSION] Google consent page detected, attempting bypass");
                // Try to extract and submit the consent form action
                let form_data = self.eval_internal(r#"
                    (function() {
                        const form = document.querySelector('form[action*="consent"]')
                            || document.querySelector('form[action*="save"]')
                            || document.querySelector('form');
                        if (!form) return JSON.stringify({found: false});
                        const action = form.getAttribute('action') || '';
                        const inputs = {};
                        form.querySelectorAll('input[type="hidden"]').forEach(i => {
                            if (i.name) inputs[i.name] = i.value || '';
                        });
                        return JSON.stringify({found: true, action: action, inputs: inputs});
                    })()
                "#).unwrap_or_default();

                if let Ok(form_info) = serde_json::from_str::<serde_json::Value>(&form_data) {
                    if form_info["found"] == true {
                        let action = form_info["action"].as_str().unwrap_or("");
                        let consent_url = if action.starts_with("http") {
                            action.to_string()
                        } else if action.starts_with('/') {
                            // Resolve relative to origin
                            url::Url::parse(&final_url)
                                .ok()
                                .map(|u| format!("{}://{}{}", u.scheme(), u.host_str().unwrap_or(""), action))
                                .unwrap_or_default()
                        } else {
                            String::new()
                        };

                        if !consent_url.is_empty() {
                            // Build form body using url::form_urlencoded
                            let mut serializer = url::form_urlencoded::Serializer::new(String::new());
                            let mut has_consent_field = false;
                            if let Some(inputs) = form_info["inputs"].as_object() {
                                for (k, v) in inputs {
                                    let val = v.as_str().unwrap_or("");
                                    serializer.append_pair(k, val);
                                    if k == "set_eom" || k == "consent" {
                                        has_consent_field = true;
                                    }
                                }
                            }
                            if !has_consent_field {
                                serializer.append_pair("set_eom", "true");
                            }
                            let body_str = serializer.finish();
                            eprintln!("[NEOSESSION] Submitting Google consent form to {}", consent_url);

                            // Build headers with cookies from unified jar
                            let mut consent_headers = rquest::header::HeaderMap::new();
                            if let Some(ch) = self.cookie_jar.cookie_header_for(&consent_url) {
                                if let Ok(v) = rquest::header::HeaderValue::from_str(&ch) {
                                    consent_headers.insert(rquest::header::COOKIE, v);
                                }
                            }
                            consent_headers.insert(
                                rquest::header::CONTENT_TYPE,
                                rquest::header::HeaderValue::from_static("application/x-www-form-urlencoded"),
                            );

                            // POST consent form
                            if let Ok(consent_resp) = self.network.client()
                                .post(&consent_url)
                                .headers(consent_headers)
                                .body(body_str)
                                .send()
                                .await
                            {
                                // Store consent cookies in unified jar + legacy jar
                                let consent_final_url = consent_resp.url().to_string();
                                for cookie in consent_resp.headers().get_all(rquest::header::SET_COOKIE) {
                                    if let Ok(s) = cookie.to_str() {
                                        self.cookie_jar.store_from_header(&consent_final_url, s);
                                        if let Some(d) = consent_resp.url().host_str() {
                                            self.cookies.store_from_header(d, s);
                                        }
                                    }
                                }
                                eprintln!("[NEOSESSION] Google consent submitted (status {}), re-fetching {}",
                                    consent_resp.status(), url);

                                // Re-fetch the original URL with consent cookies now set.
                                // We update cookies in V8 and return the second page result
                                // by performing a second goto via goto_consent_retry.
                                return self.goto_consent_retry(url, start).await;
                            }
                        }
                    }
                }
            }
        }

        // 13d. Remove noise (chat widgets, ads, popups) BEFORE WOM extraction
        if let Ok(noise_result) = self.eval_internal("__neo_remove_noise()") {
            if !noise_result.contains("\"removed\":0") && !noise_result.contains("\"removed\": 0") {
                eprintln!("[NEOSESSION] Noise removal: {}", noise_result);
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

        // 15. AI-optimized: page classification
        let page_type = self.eval_internal("__neo_classify()")
            .unwrap_or_else(|_| "content".to_string());

        // 16. AI-optimized: semantic compression (2000 chars default)
        let compressed_content = self.eval_internal("__neo_compress(2000)")
            .unwrap_or_else(|_| "[]".to_string());

        // 17. AI-optimized: extract structured actions from WOM
        let actions = Self::extract_actions_from_wom(&wom);

        let render_time = start.elapsed();
        self.url = final_url.clone();
        self.navigated = true;

        eprintln!("[NEOSESSION] Rendered in {:?} — {} bytes, {} scripts, {} errors, type={}",
            render_time, html_len, scripts_count, errors.len(), page_type);

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
            page_type,
            compressed_content,
            actions,
        })
    }

    /// Re-fetch after consent form submission. Avoids async recursion by duplicating
    /// the essential goto logic (fetch + render + WOM). Reuses the same start time
    /// so render_time_ms reflects the total including consent.
    async fn goto_consent_retry(&mut self, url: &str, start: std::time::Instant) -> Result<PageResult, String> {
        // 1. Fetch with updated consent cookies from unified jar + navigation headers
        let mut headers = super::net::navigation_headers();
        if let Some(cookie_header) = self.cookie_jar.cookie_header_for(url) {
            if let Ok(v) = rquest::header::HeaderValue::from_str(&cookie_header) {
                headers.insert(rquest::header::COOKIE, v);
            }
        }

        let resp = self.network.client().get(url)
            .headers(headers)
            .send()
            .await
            .map_err(|e| format!("HTTP error (consent retry): {e}"))?;

        let status = resp.status().as_u16();
        let final_url = resp.url().to_string();

        // Store response cookies in unified jar + legacy jar
        for cookie in resp.headers().get_all(rquest::header::SET_COOKIE) {
            if let Ok(s) = cookie.to_str() {
                self.cookie_jar.store_from_header(&final_url, s);
                if let Some(domain) = resp.url().host_str() {
                    self.cookies.store_from_header(domain, s);
                }
            }
        }

        let html = resp.text().await.map_err(|e| format!("Body error: {e}"))?;
        let html_len = html.len();

        // 2. Replace DOM
        let html_json = serde_json::to_string(&html).unwrap_or_default();
        let inject_js = format!("globalThis.__neo_html = {};", html_json);
        self.runtime.execute_script("<neosession:inject_html>", inject_js)
            .map_err(|e| format!("HTML injection error: {e}"))?;

        let reparse_js = r#"{
            const { document: freshDoc } = __linkedom_parseHTML(globalThis.__neo_html);
            if (freshDoc.head) document.head.innerHTML = freshDoc.head.innerHTML;
            if (freshDoc.body) document.body.innerHTML = freshDoc.body.innerHTML;
            for (const attr of freshDoc.documentElement.attributes || []) {
                try { document.documentElement.setAttribute(attr.name, attr.value); } catch {}
            }
            try { document.location = location; } catch {}
            delete globalThis.__neo_html;
        }"#;
        self.runtime.execute_script("<neosession:reparse>", reparse_js.to_string())
            .map_err(|e| format!("DOM reparse error: {e}"))?;

        // 3. Update location
        v8_runtime::set_location(&mut self.runtime, &final_url)?;

        // 4. Brief event loop
        v8_runtime::run_event_loop(&mut self.runtime, 500).await?;

        // 4b. Remove noise before WOM
        self.eval_internal("__neo_remove_noise()").ok();

        // 5. Extract WOM
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

        // AI-optimized fields
        let page_type = self.eval_internal("__neo_classify()")
            .unwrap_or_else(|_| "content".to_string());
        let compressed_content = self.eval_internal("__neo_compress(2000)")
            .unwrap_or_else(|_| "[]".to_string());
        let actions = Self::extract_actions_from_wom(&wom);

        self.url = final_url.clone();
        self.navigated = true;

        let render_time = start.elapsed();
        eprintln!("[NEOSESSION] Consent retry rendered in {:?} — {} bytes", render_time, html_len);

        Ok(PageResult {
            url: final_url,
            status,
            title,
            text,
            html_len,
            scripts_count: 0,
            render_time_ms: render_time.as_millis() as u64,
            errors: Vec::new(),
            wom,
            page_type,
            compressed_content,
            actions,
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

            // Inject session cookies from unified jar
            if let Some(cookie_header) = self.cookie_jar.cookie_header_for(url) {
                hdrs.insert("cookie".to_string(), serde_json::Value::String(cookie_header));
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

    /// Extract structured actions from WOM JSON (links, buttons, inputs, selects).
    fn extract_actions_from_wom(wom: &Option<serde_json::Value>) -> Vec<Action> {
        let mut actions = Vec::new();
        let Some(wom) = wom else { return actions };

        // Links
        if let Some(links) = wom["links"].as_array() {
            for link in links.iter().take(30) {
                let text = link["text"].as_str().unwrap_or("").trim().to_string();
                let href = link["href"].as_str().unwrap_or("").to_string();
                if !text.is_empty() && !href.is_empty() {
                    actions.push(Action {
                        action_type: "link".to_string(),
                        text,
                        target: href,
                    });
                }
            }
        }

        // Buttons
        if let Some(buttons) = wom["buttons"].as_array() {
            for btn in buttons.iter().take(20) {
                let text = btn["text"].as_str()
                    .or_else(|| btn["label"].as_str())
                    .unwrap_or("").trim().to_string();
                if !text.is_empty() {
                    actions.push(Action {
                        action_type: "button".to_string(),
                        text: text.clone(),
                        target: format!("button:has-text(\"{}\")", text),
                    });
                }
            }
        }

        // Inputs
        if let Some(inputs) = wom["inputs"].as_array() {
            for input in inputs.iter().take(20) {
                let name = input["name"].as_str().unwrap_or("").to_string();
                let input_type = input["type"].as_str().unwrap_or("text").to_string();
                let placeholder = input["placeholder"].as_str().unwrap_or("").to_string();
                let label = if !placeholder.is_empty() { placeholder } else { name.clone() };
                if !name.is_empty() || !label.is_empty() {
                    actions.push(Action {
                        action_type: "input".to_string(),
                        text: label,
                        target: if !name.is_empty() {
                            format!("input[name=\"{}\"]", name)
                        } else {
                            format!("input[type=\"{}\"]", input_type)
                        },
                    });
                }
            }
        }

        actions
    }

    /// Current URL.
    pub fn current_url(&self) -> &str {
        &self.url
    }

    /// Is the session navigated to a page?
    pub fn is_navigated(&self) -> bool {
        self.navigated
    }

    /// Get a reference to the legacy cookie jar (backward compat).
    pub fn cookies(&self) -> &ghost::CookieJar {
        &self.cookies
    }

    /// Get a mutable reference to the legacy cookie jar (for loading additional cookies).
    pub fn cookies_mut(&mut self) -> &mut ghost::CookieJar {
        &mut self.cookies
    }

    /// Get a reference to the unified cookie jar (source of truth, SQLite-backed).
    pub fn cookie_jar(&self) -> &CookieJarHandle {
        &self.cookie_jar
    }
}
