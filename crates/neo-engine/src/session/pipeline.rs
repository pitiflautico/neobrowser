//! Navigation pipeline — DOM parsing, JS execution, WOM extraction.

use std::collections::HashMap;
use std::time::Instant;

use neo_http::{HttpRequest, RequestContext, RequestKind};
use neo_types::{NetworkLogEntry, PageState};

use neo_runtime::neo_trace;

use super::pipeline_phases::{self, DomExportDecision};
use super::prefetch::prefetch_modules;
use super::script_exec::{execute_scripts, fetch_external_scripts};
use super::script_parts::fire_observers;
use super::scripts::{detect_framework, detect_meta_refresh, extract_import_map, extract_scripts};
use super::stub::stub_heavy_modules;
use super::{HistoryEntry, NeoSession};
use crate::pipeline::PipelinePhase;
use crate::{BrowserEngine, EngineError, PageResult};

impl NeoSession {
    /// Finish navigation after the HTTP response (or cache hit) is available.
    ///
    /// Handles DOM parse, JS execution, WOM extraction, tracing, and history.
    pub(crate) fn finish_navigate(
        &mut self,
        url: &str,
        response: neo_types::HttpResponse,
        start: Instant,
        redirect_chain: Vec<String>,
    ) -> Result<PageResult, EngineError> {
        // DOM parse.
        if let Some(ref mut ctx) = self.pipeline_ctx {
            ctx.enter_phase(PipelinePhase::Parse);
        }
        let tparse = Instant::now();
        {
            let mut dom = self.dom.lock().expect("dom lock poisoned");
            dom.parse_html(&response.body, &response.url)?;
        }
        {
            let dom = self.dom.lock().expect("dom lock poisoned");
            let el_count = dom.query_selector_all("*").len();
            eprintln!(
                "[profile] html_parse: {}ms ({}KB, {} elements)",
                tparse.elapsed().as_millis(),
                response.body.len() / 1024,
                el_count,
            );
        }

        // Interactive.
        self.lifecycle
            .transition(PageState::Interactive, "dom parsed");

        // JS execution (if enabled and runtime available).
        if let Some(ref mut ctx) = self.pipeline_ctx {
            ctx.enter_phase(PipelinePhase::Execute);
        }
        let js_errors = self.execute_page_scripts(&response);

        // Settled.
        self.lifecycle
            .transition(PageState::Settled, "scripts executed");

        // Validate hydration — detect if SPA framework actually mounted.
        if let Some(ref mut rt) = self.runtime {
            let hydration_js = r#"(function() {
    var m = { react: false, vue: false, svelte: false, generic: false };
    var roots = document.querySelectorAll('#root, #__next, #app, #factorialRoot, [data-reactroot]');
    for (var i = 0; i < roots.length; i++) {
        var keys = Object.keys(roots[i]);
        for (var j = 0; j < keys.length; j++) {
            if (keys[j].startsWith('__reactFiber') || keys[j].startsWith('__reactInternalInstance')) {
                m.react = true; break;
            }
        }
        if (m.react) break;
    }
    if (document.querySelector('[data-v-app]')) m.vue = true;
    var vueApps = document.querySelectorAll('#app, #__nuxt');
    for (var i = 0; i < vueApps.length; i++) {
        if (vueApps[i].__vue_app__ || vueApps[i].__vue__) m.vue = true;
    }
    if (document.querySelector('[data-svelte-h]')) m.svelte = true;
    var mainRoots = document.querySelectorAll('#root, #__next, #app, main, [role="main"]');
    for (var i = 0; i < mainRoots.length; i++) {
        if (mainRoots[i].children.length > 0 && mainRoots[i].textContent.trim().length > 10) {
            m.generic = true; break;
        }
    }
    return JSON.stringify(m);
})()"#;
            match rt.eval(hydration_js) {
                Ok(markers_json) => {
                    eprintln!("[hydration] markers: {markers_json}");
                }
                Err(e) => {
                    eprintln!("[hydration] check failed: {e}");
                }
            }
        }

        // Extract WOM.
        if let Some(ref mut ctx) = self.pipeline_ctx {
            ctx.enter_phase(PipelinePhase::Extract);
        }
        let twom = Instant::now();
        let mut wom = {
            let dom = self.dom.lock().expect("dom lock poisoned");
            self.extractor.extract_wom(dom.as_ref())
        };
        eprintln!("[profile] wom_extract: {}ms", twom.elapsed().as_millis());
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
            page_id: self.page_id.load(std::sync::atomic::Ordering::Relaxed),
        })
    }

    /// Execute inline and external scripts from the page HTML.
    ///
    /// Pipeline order: fetch externals -> prefetch imports -> stub heavy -> execute.
    fn execute_page_scripts(&mut self, response: &neo_types::HttpResponse) -> Vec<String> {
        let mut js_errors: Vec<String> = Vec::new();
        if !self.config.execute_js {
            return js_errors;
        }

        // Take the runtime out temporarily to avoid self-borrow conflicts.
        let mut rt = match self.runtime.take() {
            Some(r) => r,
            None => return js_errors,
        };

        let result = self.run_script_pipeline(&mut rt, response, &mut js_errors);
        self.runtime = Some(rt);

        if let Err(e) = result {
            js_errors.push(format!("pipeline: {e}"));
        }
        js_errors
    }

    /// Inner pipeline: fetch, prefetch, stub, execute, settle, export.
    fn run_script_pipeline(
        &mut self,
        rt: &mut Box<dyn neo_runtime::JsRuntime>,
        response: &neo_types::HttpResponse,
        js_errors: &mut Vec<String>,
    ) -> Result<(), String> {
        let t0 = Instant::now();
        rt.set_document_html(&response.body, &response.url)
            .map_err(|e| format!("set_document_html: {e}"))?;
        eprintln!("[profile] dom_load: {}ms", t0.elapsed().as_millis());

        let t1 = Instant::now();
        let scripts = extract_scripts(&response.body, &response.url);
        eprintln!(
            "[profile] script_discovery: {}ms ({} scripts)",
            t1.elapsed().as_millis(),
            scripts.len()
        );

        // Framework detection (telemetry only).
        let script_urls: Vec<String> = scripts
            .iter()
            .filter_map(|s| s.url().map(String::from))
            .collect();
        let framework = detect_framework(&response.body, &script_urls);
        neo_trace!("[FRAMEWORK] detected: {framework}");

        // Import map: parse and inject into the module loader.
        if let Some(map) = extract_import_map(&scripts) {
            neo_trace!("[MODULE] import-map loaded ({} entries)", map.imports.len());
            rt.set_import_map(map);
        }

        let trace_id = "nav";

        // Fetch external scripts into the module store.
        let t2 = Instant::now();
        fetch_external_scripts(
            &scripts,
            &response.url,
            rt.as_mut(),
            self.http.as_ref(),
            js_errors,
        );
        eprintln!(
            "[profile] script_fetch: {}ms",
            t2.elapsed().as_millis()
        );

        // R3: Pre-fetch ES module imports (depth 2).
        let t3 = Instant::now();
        let _prefetch = prefetch_modules(
            &scripts,
            &response.url,
            rt.as_mut(),
            self.http.as_ref(),
            self.tracer.as_ref(),
            trace_id,
        );
        eprintln!(
            "[profile] prefetch_modules: {}ms",
            t3.elapsed().as_millis()
        );

        // R4: Stub heavy non-essential modules.
        if self.config.stub_heavy_modules {
            let _stub = stub_heavy_modules(
                &scripts,
                &response.url,
                self.config.stub_threshold_bytes,
                rt.as_mut(),
                self.tracer.as_ref(),
                trace_id,
            );
        }

        // Execute scripts in document order.
        let t4 = Instant::now();
        let trace_events = execute_scripts(
            &scripts,
            &response.url,
            rt.as_mut(),
            self.http.as_ref(),
            self.tracer.as_ref(),
            trace_id,
            js_errors,
        );
        self.trace_events.extend(trace_events);
        eprintln!(
            "[profile] script_exec: {}ms",
            t4.elapsed().as_millis()
        );

        // Reset timer budget before settle — inline scripts may have exhausted it,
        // but React scheduler needs setTimeout to work during hydration.
        // The V8 watchdog (terminate_execution) prevents infinite loops,
        // not the timer budget.
        rt.reset_budgets();

        // Settle: run event loop for promises, timers, etc.
        let t5 = Instant::now();
        if let Err(e) = rt.run_until_settled(self.config.script_timeout_ms) {
            js_errors.push(format!("settle: {e}"));
        }
        eprintln!("[profile] settle: {}ms", t5.elapsed().as_millis());

        // Extended settle: repeatedly pump event loop until DOM stabilizes.
        let t5a = Instant::now();
        let module_count = scripts.iter().filter(|s| s.is_module()).count();
        let settle_result = pipeline_phases::run_settle_loop(rt.as_mut(), module_count);

        // Post-settle: load entry modules (Vite/React Router).
        let _entry_loaded = pipeline_phases::try_load_entry_module(rt.as_mut(), &response.url);

        // Diagnostics: node count after settle.
        let node_count = rt
            .eval("document.querySelectorAll('*').length")
            .unwrap_or_else(|_| "?".to_string());
        neo_trace!(
            "[SETTLE] pumped {} microtask rounds, 0 macrotask rounds ({}ms)",
            settle_result.micro_rounds,
            t5a.elapsed().as_millis()
        );
        neo_trace!("[SETTLE] DOM nodes after settle: {node_count}");

        // Fire synthetic observer callbacks after settle — SPAs use
        // IntersectionObserver for lazy loading and ResizeObserver for layout.
        let (fired_io, fired_ro) = fire_observers(rt.as_mut());
        if fired_io != 0 || fired_ro != 0 {
            neo_trace!("[SETTLE] fired observers: IO={fired_io} RO={fired_ro}");
            // Brief settle after observer callbacks may trigger DOM mutations
            let _ = rt.run_until_settled(1000);
        }

        // Export the JS-mutated DOM and re-parse into html5ever.
        // Only replace the original parse if V8 produced a RICHER DOM.
        let t6 = Instant::now();
        let original_elements = {
            let dom = self.dom.lock().expect("dom lock poisoned");
            dom.query_selector_all("*").len()
        };
        match pipeline_phases::validate_dom_export(rt.as_mut(), original_elements, &response.url) {
            DomExportDecision::Accept { html, .. } => {
                let mut dom = self.dom.lock().expect("dom lock poisoned");
                if let Err(e) = dom.parse_html(&html, &response.url) {
                    js_errors.push(format!("re-parse: {e}"));
                }
            }
            DomExportDecision::Reject { .. } | DomExportDecision::Empty => {
                // Keep original SSR parse.
            }
            DomExportDecision::Error(e) => {
                js_errors.push(e);
            }
        }
        eprintln!("[profile] dom_export: {}ms", t6.elapsed().as_millis());
        Ok(())
    }

    /// Navigate with an existing redirect chain (used for meta-refresh).
    pub(crate) fn navigate_with_chain(
        &mut self,
        url: &str,
        chain: Vec<String>,
    ) -> Result<PageResult, EngineError> {
        let start = Instant::now();
        url::Url::parse(url).map_err(|e| EngineError::InvalidUrl(e.to_string()))?;
        self.lifecycle
            .transition(PageState::Navigating, "meta-refresh redirect");
        let mut req = self.build_nav_request(url);
        if let Some(ref store) = self.cookie_store {
            let is_top = req.context.kind == RequestKind::Navigation;
            let tlu = req.context.top_level_url.clone();
            let cookie_header = store.get_for_request(&req.url, tlu.as_deref(), is_top);
            if !cookie_header.is_empty() {
                req.headers.insert("cookie".to_string(), cookie_header);
            }
        }
        let response = self.http.request(&req)?;
        // Store Set-Cookie headers from meta-refresh response.
        if let Some(ref store) = self.cookie_store {
            for (key, value) in &response.headers {
                if key.eq_ignore_ascii_case("set-cookie") {
                    store.store_set_cookie(&response.url, value);
                }
            }
        }
        self.network_log.push(NetworkLogEntry {
            url: req.url.clone(),
            method: req.method.clone(),
            status: response.status,
            duration_ms: response.duration_ms,
            kind: format!("{:?}", req.context.kind),
            initiator: "meta-refresh".to_string(),
        });
        self.lifecycle
            .transition(PageState::Loading, "response received");
        self.finish_navigate(url, response, start, chain)
    }

    /// Build an HTTP GET request for navigation.
    pub(crate) fn build_nav_request(&self, url: &str) -> HttpRequest {
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

    /// Pump the V8 event loop after an interaction to let microtasks/timers run.
    ///
    /// Uses `run_until_interaction_stable` with relaxed criteria:
    /// - min_settle = 75ms (vs 1500ms for bootstrap)
    /// - Epoch tracking: won't settle mid-React-commit (requires quiet cycle after activity)
    /// - 3s budget, but settles fast if quiet
    pub(crate) fn pump_after_interaction(&mut self) {
        self.pump_after_interaction_with_timeout(10000);
    }

    /// Like `pump_after_interaction` but with a configurable timeout.
    /// Used for long-running evals (e.g. pong: type → send → wait for response).
    pub(crate) fn pump_after_interaction_with_timeout(&mut self, timeout_ms: u64) {
        if let Some(ref mut rt) = self.runtime {
            let start = std::time::Instant::now();
            let _ = rt.run_until_interaction_stable(timeout_ms);
            eprintln!(
                "[NeoRender] pump: {}ms",
                start.elapsed().as_millis()
            );
        }
    }

    /// Drain pending navigation requests from the JS shim and execute the first one.
    ///
    /// Called after click/submit/eval — if JS triggered form.submit() or
    /// location.href = ..., the shim queues a navigation request. This method
    /// picks it up, makes the HTTP request, and reloads the page.
    pub(crate) fn process_pending_navigations(&mut self) {
        let requests = if let Some(ref mut rt) = self.runtime {
            rt.drain_navigation_requests()
        } else {
            return;
        };

        if requests.is_empty() {
            return;
        }

        // Process only the FIRST navigation (subsequent ones are superseded).
        let req_json = &requests[0];
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(req_json);
        let nav = match parsed {
            Ok(v) => v,
            Err(e) => {
                eprintln!("[NeoRender] Failed to parse navigation request: {e}");
                return;
            }
        };

        let raw_url = nav["url"].as_str().unwrap_or("").to_string();
        let nav_type = nav["type"].as_str().unwrap_or("unknown");

        if raw_url.is_empty() {
            return;
        }

        // Resolve relative URLs against the current page URL.
        let url = if raw_url.starts_with("http://") || raw_url.starts_with("https://") {
            raw_url
        } else {
            // Get current base URL from history stack.
            let base = self.history_stack.last()
                .map(|e| e.url.clone())
                .unwrap_or_default();
            if let Ok(base_url) = url::Url::parse(&base) {
                base_url.join(&raw_url)
                    .map(|u| u.to_string())
                    .unwrap_or(raw_url)
            } else {
                raw_url
            }
        };

        // For GET form submits, append form_data as query string.
        let form_method = if nav_type == "form_submit" {
            nav["method"].as_str().unwrap_or("GET").to_uppercase()
        } else {
            "GET".to_string()
        };
        let url = if nav_type == "form_submit" && form_method == "GET" {
            if let Some(form_data) = nav["form_data"].as_object() {
                let params: Vec<String> = form_data.iter()
                    .filter_map(|(k, v)| v.as_str().map(|val| format!("{}={}", k, url::form_urlencoded::byte_serialize(val.as_bytes()).collect::<String>())))
                    .collect();
                if !params.is_empty() {
                    let sep = if url.contains('?') { "&" } else { "?" };
                    format!("{url}{sep}{}", params.join("&"))
                } else {
                    url
                }
            } else {
                url
            }
        } else {
            url // POST: URL stays clean, body sent separately
        };

        eprintln!("[NeoRender] Navigation triggered by JS ({nav_type}): {url}");

        // For POST form submits, build a custom request with body.
        if nav_type == "form_submit" && form_method == "POST" {
            let mut req = self.build_nav_request(&url);
            req.method = "POST".to_string();
            let enctype = nav["enctype"]
                .as_str()
                .unwrap_or("application/x-www-form-urlencoded");

            if let Some(form_data) = nav["form_data"].as_object() {
                if enctype == "multipart/form-data" {
                    // F3a: multipart/form-data encoding
                    let boundary = format!(
                        "----NeoRender{}",
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis()
                    );
                    let mut body = String::new();
                    for (k, v) in form_data.iter() {
                        match v {
                            serde_json::Value::Array(arr) => {
                                for item in arr {
                                    let val = item.as_str().unwrap_or("");
                                    body.push_str(&format!("--{boundary}\r\n"));
                                    body.push_str(&format!(
                                        "Content-Disposition: form-data; name=\"{k}\"\r\n\r\n"
                                    ));
                                    body.push_str(&format!("{val}\r\n"));
                                }
                            }
                            _ => {
                                let val = v.as_str().unwrap_or("");
                                body.push_str(&format!("--{boundary}\r\n"));
                                body.push_str(&format!(
                                    "Content-Disposition: form-data; name=\"{k}\"\r\n\r\n"
                                ));
                                body.push_str(&format!("{val}\r\n"));
                            }
                        }
                    }
                    body.push_str(&format!("--{boundary}--\r\n"));
                    req.body = Some(body);
                    req.headers.insert(
                        "content-type".to_string(),
                        format!("multipart/form-data; boundary={boundary}"),
                    );
                } else if enctype == "text/plain" {
                    // F3a: text/plain encoding
                    let mut body = String::new();
                    for (k, v) in form_data.iter() {
                        let val = v.as_str().unwrap_or("");
                        body.push_str(&format!("{k}={val}\r\n"));
                    }
                    req.body = Some(body);
                    req.headers
                        .insert("content-type".to_string(), "text/plain".to_string());
                } else {
                    // Default: application/x-www-form-urlencoded
                    let mut pairs: Vec<String> = Vec::new();
                    for (k, v) in form_data.iter() {
                        let enc_k: String =
                            url::form_urlencoded::byte_serialize(k.as_bytes()).collect();
                        match v {
                            serde_json::Value::Array(arr) => {
                                for item in arr {
                                    if let Some(val) = item.as_str() {
                                        let enc_v: String =
                                            url::form_urlencoded::byte_serialize(val.as_bytes())
                                                .collect();
                                        pairs.push(format!("{enc_k}={enc_v}"));
                                    }
                                }
                            }
                            _ => {
                                let val = v.as_str().unwrap_or("");
                                let enc_v: String =
                                    url::form_urlencoded::byte_serialize(val.as_bytes()).collect();
                                pairs.push(format!("{enc_k}={enc_v}"));
                            }
                        }
                    }
                    req.body = Some(pairs.join("&"));
                    req.headers.insert(
                        "content-type".to_string(),
                        "application/x-www-form-urlencoded".to_string(),
                    );
                }
            }
            // Inject cookies
            if let Some(ref store) = self.cookie_store {
                let is_top = req.context.kind == RequestKind::Navigation;
                let tlu = req.context.top_level_url.clone();
                let cookie_header = store.get_for_request(&req.url, tlu.as_deref(), is_top);
                if !cookie_header.is_empty() {
                    req.headers.insert("cookie".to_string(), cookie_header);
                }
            }
            let start = Instant::now();
            match self.http.request(&req) {
                Ok(response) => {
                    // Store Set-Cookie headers from POST response.
                    if let Some(ref store) = self.cookie_store {
                        for (key, value) in &response.headers {
                            if key.eq_ignore_ascii_case("set-cookie") {
                                store.store_set_cookie(&response.url, value);
                            }
                        }
                    }
                    self.network_log.push(NetworkLogEntry {
                        url: req.url.clone(),
                        method: req.method.clone(),
                        status: response.status,
                        duration_ms: response.duration_ms,
                        kind: format!("{:?}", req.context.kind),
                        initiator: "form_submit_post".to_string(),
                    });
                    self.lifecycle
                        .transition(PageState::Loading, "POST response received");
                    match self.finish_navigate(&url, response, start, Vec::new()) {
                        Ok(result) => {
                            eprintln!(
                                "[NeoRender] POST navigated: {} ({}, {}ms)",
                                result.title, result.url, result.render_ms
                            );
                        }
                        Err(e) => {
                            eprintln!("[NeoRender] POST navigation failed: {e}");
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[NeoRender] POST request failed: {e}");
                }
            }
            return;
        }

        match self.navigate(&url) {
            Ok(result) => {
                eprintln!(
                    "[NeoRender] Re-navigated: {} ({}, {}ms)",
                    result.title, result.url, result.render_ms
                );
            }
            Err(e) => {
                eprintln!("[NeoRender] Re-navigation failed: {e}");
            }
        }
    }
}
