//! NeoSession — the main `BrowserEngine` implementation.
//!
//! Wires HTTP, DOM, JS runtime, interaction, extraction, and tracing
//! into the navigation lifecycle.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use neo_dom::DomEngine;
use neo_extract::{Extractor, WomDocument};
use neo_http::{
    CacheDecision, CookieStore, HttpCache, HttpClient, HttpRequest, RequestContext, RequestKind,
};
use neo_interact::{ClickResult, Interactor, SubmitResult};
use neo_runtime::JsRuntime;
use neo_trace::{ExecutionSummary, Tracer};
use neo_types::{NetworkLogEntry, PageState, TraceEntry};

use crate::config::EngineConfig;
use crate::lifecycle::Lifecycle;
use crate::{BrowserEngine, EngineError, PageResult};

/// An entry in the navigation history.
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub url: String,
    pub title: String,
    pub timestamp: u64,
}

/// The main browser engine session.
///
/// Holds all subsystem trait objects and orchestrates the full
/// navigate -> parse -> execute -> extract pipeline.
pub struct NeoSession {
    http: Box<dyn HttpClient>,
    dom: Arc<Mutex<Box<dyn DomEngine>>>,
    runtime: Option<Box<dyn JsRuntime>>,
    interactor: Box<dyn Interactor>,
    extractor: Box<dyn Extractor>,
    tracer: Box<dyn Tracer>,
    lifecycle: Lifecycle,
    config: EngineConfig,
    history_stack: Vec<HistoryEntry>,
    history_index: isize,
    network_log: Vec<NetworkLogEntry>,
    /// Cached WOM from last navigation.
    last_wom: Option<WomDocument>,
    /// Cookie store for cross-navigation persistence.
    cookie_store: Option<Box<dyn CookieStore>>,
    /// HTTP response cache (disk-backed).
    http_cache: Option<Box<dyn HttpCache>>,
}

impl NeoSession {
    /// Create a new session from subsystem implementations.
    ///
    /// The DOM is wrapped in `Arc<Mutex<...>>` internally. Use
    /// [`new_shared`] if you need to share the DOM with the interactor.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        http: Box<dyn HttpClient>,
        dom: Box<dyn DomEngine>,
        runtime: Option<Box<dyn JsRuntime>>,
        interactor: Box<dyn Interactor>,
        extractor: Box<dyn Extractor>,
        tracer: Box<dyn Tracer>,
        lifecycle_tracer: Box<dyn Tracer>,
        config: EngineConfig,
    ) -> Self {
        Self {
            http,
            dom: Arc::new(Mutex::new(dom)),
            runtime,
            interactor,
            extractor,
            tracer,
            lifecycle: Lifecycle::new(lifecycle_tracer),
            config,
            history_stack: Vec::new(),
            history_index: -1,
            network_log: Vec::new(),
            last_wom: None,
            cookie_store: None,
            http_cache: None,
        }
    }

    /// Create a session with a shared DOM reference.
    ///
    /// The same `Arc<Mutex<...>>` can be given to a [`DomInteractor`]
    /// so that interactions mutate the same DOM the session reads from.
    #[allow(clippy::too_many_arguments)]
    pub fn new_shared(
        http: Box<dyn HttpClient>,
        dom: Arc<Mutex<Box<dyn DomEngine>>>,
        runtime: Option<Box<dyn JsRuntime>>,
        interactor: Box<dyn Interactor>,
        extractor: Box<dyn Extractor>,
        tracer: Box<dyn Tracer>,
        lifecycle_tracer: Box<dyn Tracer>,
        config: EngineConfig,
    ) -> Self {
        Self {
            http,
            dom,
            runtime,
            interactor,
            extractor,
            tracer,
            lifecycle: Lifecycle::new(lifecycle_tracer),
            config,
            history_stack: Vec::new(),
            history_index: -1,
            network_log: Vec::new(),
            last_wom: None,
            cookie_store: None,
            http_cache: None,
        }
    }

    /// Attach a cookie store for cross-navigation cookie persistence.
    pub fn with_cookie_store(mut self, store: Box<dyn CookieStore>) -> Self {
        self.cookie_store = Some(store);
        self
    }

    /// Attach an HTTP cache for conditional requests and freshness.
    pub fn with_http_cache(mut self, cache: Box<dyn HttpCache>) -> Self {
        self.http_cache = Some(cache);
        self
    }

    /// Import cookies into the cookie store.
    ///
    /// No-op if no cookie store is attached.
    pub fn import_cookies(&self, cookies: &[neo_types::Cookie]) {
        if let Some(ref store) = self.cookie_store {
            store.import(cookies);
        }
    }

    /// Navigation history as URL list.
    pub fn history_urls(&self) -> Vec<String> {
        self.history_stack.iter().map(|e| e.url.clone()).collect()
    }

    /// Full history stack.
    pub fn history_stack(&self) -> &[HistoryEntry] {
        &self.history_stack
    }

    /// Network log of all requests made.
    pub fn network_log(&self) -> &[NetworkLogEntry] {
        &self.network_log
    }

    /// Finish navigation after the HTTP response (or cache hit) is available.
    ///
    /// Handles DOM parse, JS execution, WOM extraction, tracing, and history.
    fn finish_navigate(
        &mut self,
        url: &str,
        response: neo_types::HttpResponse,
        start: Instant,
        redirect_chain: Vec<String>,
    ) -> Result<PageResult, EngineError> {
        // DOM parse.
        {
            let mut dom = self.dom.lock().expect("dom lock poisoned");
            dom.parse_html(&response.body, &response.url)?;
        }

        // Interactive.
        self.lifecycle
            .transition(PageState::Interactive, "dom parsed");

        // JS execution (if enabled and runtime available).
        let mut js_errors: Vec<String> = Vec::new();
        if self.config.execute_js {
            if let Some(rt) = self.runtime.as_mut() {
                // 1. Initialize the V8 DOM with the page HTML + bootstrap globals.
                rt.set_document_html(&response.body, &response.url)?;

                // 2. Extract and execute inline/external scripts.
                let scripts = extract_scripts(&response.body, &response.url);
                for script in &scripts {
                    match script {
                        ScriptInfo::Inline { content, .. } => {
                            if let Err(e) = rt.execute(content) {
                                js_errors.push(format!("inline script: {e}"));
                            }
                        }
                        ScriptInfo::External { url: src, .. } => {
                            let fetch_req = HttpRequest {
                                method: "GET".to_string(),
                                url: src.clone(),
                                headers: HashMap::new(),
                                body: None,
                                context: RequestContext {
                                    kind: RequestKind::Subresource,
                                    initiator: "parser".to_string(),
                                    referrer: Some(response.url.clone()),
                                    frame_id: None,
                                    top_level_url: Some(response.url.clone()),
                                },
                                timeout_ms: 5000,
                            };
                            match self.http.request(&fetch_req) {
                                Ok(script_resp) => {
                                    if let Err(e) = rt.execute(&script_resp.body) {
                                        js_errors.push(format!(
                                            "script {}: {e}",
                                            src.rsplit('/').next().unwrap_or(src)
                                        ));
                                    }
                                }
                                Err(e) => {
                                    js_errors.push(format!("fetch {src}: {e}"));
                                }
                            }
                        }
                    }
                }

                // 3. Run event loop to settle promises, timers, etc.
                if let Err(e) = rt.run_until_settled(self.config.script_timeout_ms) {
                    js_errors.push(format!("settle: {e}"));
                }

                // 4. Export the JS-mutated DOM and re-parse into html5ever.
                match rt.export_html() {
                    Ok(html) if !html.is_empty() => {
                        let mut dom = self.dom.lock().expect("dom lock poisoned");
                        if let Err(e) = dom.parse_html(&html, &response.url) {
                            js_errors.push(format!("re-parse: {e}"));
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        js_errors.push(format!("export: {e}"));
                    }
                }
            }
        }

        // Settled.
        self.lifecycle
            .transition(PageState::Settled, "scripts executed");

        // Extract WOM.
        let mut wom = {
            let dom = self.dom.lock().expect("dom lock poisoned");
            self.extractor.extract_wom(dom.as_ref())
        };
        if wom.url.is_empty() {
            wom.url = response.url.clone();
        }
        self.last_wom = Some(wom.clone());

        // Trace result.
        self.tracer
            .action_result("navigate", true, "page loaded", None);

        // Complete.
        self.lifecycle
            .transition(PageState::Complete, "extraction done");

        let title = {
            let dom = self.dom.lock().expect("dom lock poisoned");
            dom.title()
        };
        // Track history: truncate forward entries, push new.
        let new_index = self.history_index + 1;
        self.history_stack.truncate(new_index as usize);
        self.history_stack.push(HistoryEntry {
            url: url.to_string(),
            title: title.clone(),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        });
        self.history_index = new_index;

        let elapsed = start.elapsed().as_millis() as u64;

        // Check for meta-refresh redirect.
        if let Some(meta_url) = detect_meta_refresh(&response.body, &response.url) {
            let mut chain = redirect_chain;
            chain.push(response.url.clone());
            return self.navigate_with_chain(&meta_url, chain);
        }

        Ok(PageResult {
            url: response.url,
            title,
            state: self.lifecycle.current(),
            render_ms: elapsed,
            wom,
            errors: js_errors,
            redirect_chain,
        })
    }

    /// Navigate with an existing redirect chain (used for meta-refresh).
    fn navigate_with_chain(&mut self, url: &str, chain: Vec<String>) -> Result<PageResult, EngineError> {
        let start = Instant::now();
        url::Url::parse(url).map_err(|e| EngineError::InvalidUrl(e.to_string()))?;
        self.lifecycle.transition(PageState::Navigating, "meta-refresh redirect");
        let mut req = self.build_nav_request(url);
        if let Some(ref store) = self.cookie_store {
            let is_top = req.context.kind == RequestKind::Navigation;
            let tlu = req.context.top_level_url.clone();
            let cookie_header = store.get_for_request(&req.url, tlu.as_deref(), is_top);
            if !cookie_header.is_empty() { req.headers.insert("cookie".to_string(), cookie_header); }
        }
        let response = self.http.request(&req)?;
        self.network_log.push(NetworkLogEntry {
            url: req.url.clone(), method: req.method.clone(), status: response.status,
            duration_ms: response.duration_ms, kind: format!("{:?}", req.context.kind),
            initiator: "meta-refresh".to_string(),
        });
        self.lifecycle.transition(PageState::Loading, "response received");
        self.finish_navigate(url, response, start, chain)
    }

    /// Build an HTTP GET request for navigation.
    fn build_nav_request(&self, url: &str) -> HttpRequest {
        HttpRequest {
            method: "GET".to_string(),
            url: url.to_string(),
            headers: HashMap::new(),
            body: None,
            context: RequestContext {
                kind: RequestKind::Navigation,
                initiator: "engine".to_string(),
                referrer: self.history_stack.last().map(|e| e.url.clone()),
                frame_id: None,
                top_level_url: Some(url.to_string()),
            },
            timeout_ms: self.config.navigation_timeout_ms,
        }
    }
}

impl BrowserEngine for NeoSession {
    fn navigate(&mut self, url: &str) -> Result<PageResult, EngineError> {
        let start = Instant::now();

        // Validate URL.
        url::Url::parse(url).map_err(|e| EngineError::InvalidUrl(e.to_string()))?;

        // 1. Trace intent.
        self.tracer.intent("navigate", "navigate", url, 1.0);

        // 2. Navigating.
        self.lifecycle
            .transition(PageState::Navigating, "navigate started");

        // 3. Build request, inject cookies and cache headers.
        let mut req = self.build_nav_request(url);

        // 3a. Inject cookies from store.
        if let Some(ref store) = self.cookie_store {
            let is_top = req.context.kind == RequestKind::Navigation;
            let tlu = req.context.top_level_url.clone();
            let cookie_header =
                store.get_for_request(&req.url, tlu.as_deref(), is_top);
            if !cookie_header.is_empty() {
                req.headers.insert("cookie".to_string(), cookie_header);
            }
        }

        // 3b. Check HTTP cache.
        let cached_decision = self.http_cache.as_ref().map(|c| c.lookup(&req));
        if let Some(CacheDecision::Fresh(ref resp)) = cached_decision {
            // Cache hit — skip network entirely.
            let response = resp.clone();
            self.lifecycle
                .transition(PageState::Loading, "cache hit (fresh)");
            return self.finish_navigate(url, response, start, Vec::new());
        }
        // Stale — add conditional headers for revalidation.
        if let Some(CacheDecision::Stale {
            ref etag,
            ref last_modified,
            ..
        }) = cached_decision
        {
            if let Some(ref e) = etag {
                req.headers.insert("if-none-match".to_string(), e.clone());
            }
            if let Some(ref lm) = last_modified {
                req.headers
                    .insert("if-modified-since".to_string(), lm.clone());
            }
        }

        // 3c. HTTP fetch.
        let response = self.http.request(&req)?;

        // 3d. Track redirect chain.
        let mut redirect_chain = Vec::new();
        if response.url != url { redirect_chain.push(url.to_string()); }

        // 3e. Log network request.
        self.network_log.push(NetworkLogEntry {
            url: req.url.clone(), method: req.method.clone(), status: response.status,
            duration_ms: response.duration_ms, kind: format!("{:?}", req.context.kind),
            initiator: req.context.initiator.clone(),
        });

        // 3f. Handle 304 Not Modified — use stale cached body.
        let response = if response.status == 304 {
            if let Some(CacheDecision::Stale {
                response: cached, ..
            }) = cached_decision
            {
                cached
            } else {
                response
            }
        } else {
            // Store cacheable responses.
            if let Some(ref cache) = self.http_cache {
                if response.status >= 200 && response.status < 400 {
                    cache.store(&req, &response);
                }
            }
            response
        };

        // 3g. Store Set-Cookie headers from response.
        if let Some(ref store) = self.cookie_store {
            for (key, value) in &response.headers {
                if key.eq_ignore_ascii_case("set-cookie") {
                    store.store_set_cookie(&response.url, value);
                }
            }
        }

        // 4. Loading -> parse -> extract.
        self.lifecycle
            .transition(PageState::Loading, "response received");
        self.finish_navigate(url, response, start, redirect_chain)
    }

    fn back(&mut self) -> Result<PageResult, EngineError> {
        if self.history_index <= 0 {
            return Err(EngineError::InvalidUrl("no previous page in history".to_string()));
        }
        let target_index = self.history_index - 1;
        let entry = self.history_stack[target_index as usize].clone();
        let start = Instant::now();
        self.lifecycle.transition(PageState::Navigating, "back navigation");
        let req = self.build_nav_request(&entry.url);
        let response = self.http.request(&req)?;
        self.network_log.push(NetworkLogEntry {
            url: req.url.clone(), method: req.method.clone(), status: response.status,
            duration_ms: response.duration_ms, kind: format!("{:?}", req.context.kind),
            initiator: "back".to_string(),
        });
        self.lifecycle.transition(PageState::Loading, "response received");
        // Save history state, finish_navigate will push a new entry.
        let saved_stack = self.history_stack.clone();
        let result = self.finish_navigate(&entry.url, response, start, Vec::new())?;
        // Restore history stack and just move the index.
        self.history_stack = saved_stack;
        self.history_index = target_index;
        Ok(result)
    }

    fn forward(&mut self) -> Result<PageResult, EngineError> {
        let max_index = self.history_stack.len() as isize - 1;
        if self.history_index >= max_index {
            return Err(EngineError::InvalidUrl("no next page in history".to_string()));
        }
        let target_index = self.history_index + 1;
        let entry = self.history_stack[target_index as usize].clone();
        let start = Instant::now();
        self.lifecycle.transition(PageState::Navigating, "forward navigation");
        let req = self.build_nav_request(&entry.url);
        let response = self.http.request(&req)?;
        self.network_log.push(NetworkLogEntry {
            url: req.url.clone(), method: req.method.clone(), status: response.status,
            duration_ms: response.duration_ms, kind: format!("{:?}", req.context.kind),
            initiator: "forward".to_string(),
        });
        self.lifecycle.transition(PageState::Loading, "response received");
        // Save history state, finish_navigate will push a new entry.
        let saved_stack = self.history_stack.clone();
        let result = self.finish_navigate(&entry.url, response, start, Vec::new())?;
        // Restore history stack and just move the index.
        self.history_stack = saved_stack;
        self.history_index = target_index;
        Ok(result)
    }

    fn history(&self) -> Vec<String> { self.history_urls() }

    fn page_state(&self) -> PageState {
        self.lifecycle.current()
    }

    fn eval(&mut self, js: &str) -> Result<String, EngineError> {
        match self.runtime.as_mut() {
            Some(rt) => Ok(rt.eval(js)?),
            None => Err(EngineError::Runtime(neo_runtime::RuntimeError::Eval(
                "no runtime available".into(),
            ))),
        }
    }

    fn click(&mut self, target: &str) -> Result<ClickResult, EngineError> {
        self.tracer.intent("click", "click", target, 1.0);
        let result = self.interactor.click(target)?;
        self.tracer
            .action_result("click", true, &format!("{result:?}"), None);
        Ok(result)
    }

    fn type_text(&mut self, target: &str, text: &str) -> Result<(), EngineError> {
        self.tracer.intent("type", "type_text", target, 1.0);
        self.interactor.type_text(target, text, true)?;
        self.tracer.action_result("type", true, "text typed", None);
        Ok(())
    }

    fn fill_form(&mut self, fields: &HashMap<String, String>) -> Result<(), EngineError> {
        self.tracer.intent("fill", "fill_form", "form", 1.0);
        self.interactor.fill_form(fields)?;
        self.tracer.action_result("fill", true, "form filled", None);
        Ok(())
    }

    fn submit(&mut self, target: Option<&str>) -> Result<SubmitResult, EngineError> {
        let t = target.unwrap_or("form");
        self.tracer.intent("submit", "submit", t, 1.0);
        let result = self.interactor.submit(target)?;
        self.tracer
            .action_result("submit", true, &format!("{result:?}"), None);
        Ok(result)
    }

    fn extract(&self) -> Result<WomDocument, EngineError> {
        let dom = self.dom.lock().expect("dom lock poisoned");
        let wom = self.extractor.extract_wom(dom.as_ref());
        Ok(wom)
    }

    fn trace(&self) -> Vec<TraceEntry> {
        self.tracer.export()
    }

    fn summary(&self) -> ExecutionSummary {
        self.tracer.summary()
    }
}

// ─── Script extraction ───

/// Script extracted from HTML for execution.
enum ScriptInfo {
    /// Inline `<script>` tag with JS source.
    Inline {
        content: String,
        #[allow(dead_code)]
        is_module: bool,
    },
    /// External `<script src="...">` tag.
    External {
        url: String,
        #[allow(dead_code)]
        is_module: bool,
    },
}

/// Extract `<script>` tags from HTML.
///
/// Returns inline content and external URLs in document order.
/// Skips non-JS types (JSON, importmap, template, etc.).
fn extract_scripts(html: &str, base_url: &str) -> Vec<ScriptInfo> {
    use html5ever::parse_document;
    use html5ever::tendril::TendrilSink;
    use markup5ever_rcdom::RcDom;

    let dom = parse_document(RcDom::default(), Default::default()).one(html);

    let mut scripts = Vec::new();
    collect_scripts(&dom.document, base_url, &mut scripts);
    scripts
}

fn collect_scripts(
    node: &markup5ever_rcdom::Handle,
    base: &str,
    scripts: &mut Vec<ScriptInfo>,
) {
    use markup5ever_rcdom::NodeData;

    if let NodeData::Element {
        ref name,
        ref attrs,
        ..
    } = node.data
    {
        if name.local.as_ref() == "script" {
            let attrs_ref = attrs.borrow();
            let script_type = attrs_ref
                .iter()
                .find(|a| a.name.local.as_ref() == "type")
                .map(|a| a.value.to_string())
                .unwrap_or_default();

            // Skip non-JS script types.
            let st = script_type.to_lowercase();
            if st.contains("json")
                || st.contains("importmap")
                || st.contains("template")
                || st.contains("html")
                || st.contains("x-")
            {
                for child in node.children.borrow().iter() {
                    collect_scripts(child, base, scripts);
                }
                return;
            }

            let is_module = script_type == "module";
            let src = attrs_ref
                .iter()
                .find(|a| a.name.local.as_ref() == "src")
                .map(|a| a.value.to_string());

            if let Some(src) = src {
                let full = resolve_script_url(&src, base);
                scripts.push(ScriptInfo::External {
                    url: full,
                    is_module,
                });
            } else {
                drop(attrs_ref);
                let text: String = node
                    .children
                    .borrow()
                    .iter()
                    .filter_map(|c| match &c.data {
                        NodeData::Text { contents } => Some(contents.borrow().to_string()),
                        _ => None,
                    })
                    .collect();
                if !text.trim().is_empty() {
                    scripts.push(ScriptInfo::Inline {
                        content: text,
                        is_module,
                    });
                }
            }
        }
    }
    for child in node.children.borrow().iter() {
        collect_scripts(child, base, scripts);
    }
}


/// Detect `<meta http-equiv="refresh" content="...;url=...">` in HTML.
fn detect_meta_refresh(html: &str, base_url: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let needle = "http-equiv";
    let mut search_from = 0;
    while let Some(pos) = lower[search_from..].find(needle) {
        let abs_pos = search_from + pos;
        search_from = abs_pos + needle.len();
        let surrounding = &lower[abs_pos..std::cmp::min(abs_pos + 100, lower.len())];
        if !surrounding.contains("refresh") { continue; }
        let tag_start = lower[..abs_pos].rfind('<').unwrap_or(abs_pos);
        let tag_end = lower[tag_start..].find('>').map(|p| tag_start + p).unwrap_or(lower.len());
        let tag = &html[tag_start..tag_end];
        let tag_lower = tag.to_lowercase();
        if let Some(ci) = tag_lower.find("content=") {
            let after = &tag[ci + 8..];
            let (delim, start_offset) = if after.starts_with('"') {
                ('"', 1)
            } else if after.starts_with('\'') {
                ('\'', 1)
            } else {
                continue;
            };
            let content_str = &after[start_offset..];
            if let Some(end) = content_str.find(delim) {
                let content_val = &content_str[..end];
                let content_lower = content_val.to_lowercase();
                if let Some(url_pos) = content_lower.find("url=") {
                    let target = content_val[url_pos + 4..].trim();
                    if !target.is_empty() {
                        return Some(resolve_script_url(target, base_url));
                    }
                }
            }
        }
    }
    None
}

/// Resolve a script src URL against a base URL.
fn resolve_script_url(src: &str, base: &str) -> String {
    if src.starts_with("http") {
        src.to_string()
    } else if src.starts_with("//") {
        format!("https:{src}")
    } else if let Ok(base_url) = url::Url::parse(base) {
        base_url
            .join(src)
            .map(|u| u.to_string())
            .unwrap_or_else(|_| src.to_string())
    } else {
        src.to_string()
    }
}
