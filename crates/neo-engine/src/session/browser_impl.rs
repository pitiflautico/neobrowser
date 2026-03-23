//! BrowserEngine trait implementation for NeoSession.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::time::Instant;

use neo_extract::WomDocument;
use neo_http::{CacheDecision, RequestKind};
use neo_interact::{ClickResult, SubmitResult};
use neo_trace::ExecutionSummary;
use neo_types::{NetworkLogEntry, PageState, SessionState, TraceEntry};

use super::NeoSession;
use crate::live_dom::LiveDom;
use crate::pipeline::{PipelineContext, PipelineDecision, PipelinePhase};
use crate::{BrowserEngine, EngineError, PageResult};

impl BrowserEngine for NeoSession {
    fn navigate(&mut self, url: &str) -> Result<PageResult, EngineError> {
        let start = Instant::now();

        // Increment page_id at the start of every navigation.
        self.page_id.fetch_add(1, Ordering::Relaxed);

        // Validate URL.
        url::Url::parse(url).map_err(|e| EngineError::InvalidUrl(e.to_string()))?;

        // Pipeline context — tracks decisions across all phases.
        let mut ctx = PipelineContext::new(url);
        ctx.enter_phase(PipelinePhase::Fetch);

        // 1. Trace intent.
        self.tracer.intent("navigate", "navigate", url, 1.0);

        // 2. Navigating.
        self.lifecycle
            .transition(PageState::Navigating, "navigate started");

        // 3. Build request, inject cookies and cache headers.
        let mut req = self.build_nav_request(url);

        // 3a. Inject cookies from store (auto-import from Chrome if empty).
        if let Some(ref store) = self.cookie_store {
            let is_top = req.context.kind == RequestKind::Navigation;
            let tlu = req.context.top_level_url.clone();
            let mut cookie_header = store.get_for_request(&req.url, tlu.as_deref(), is_top);

            // Auto-import from Chrome if no cookies found for this domain.
            if cookie_header.is_empty() {
                if let Some(domain) = url::Url::parse(url)
                    .ok()
                    .and_then(|u| u.host_str().map(|h| h.to_string()))
                {
                    let profile = std::env::var("NEORENDER_CHROME_PROFILE")
                        .unwrap_or_else(|_| "Profile 24".to_string());
                    let importer =
                        neo_http::ChromeCookieImporter::new(&profile, Some(&domain));
                    if let Ok(cookies) = importer.import() {
                        if !cookies.is_empty() {
                            eprintln!(
                                "[NeoRender] Auto-imported {} cookies for {domain} from Chrome \"{profile}\"",
                                cookies.len()
                            );
                            store.import(&cookies);
                            cookie_header =
                                store.get_for_request(&req.url, tlu.as_deref(), is_top);
                        }
                    }
                    // Silently ignore import failures.
                }
            }

            if !cookie_header.is_empty() {
                req.headers.insert("cookie".to_string(), cookie_header);
            }
        }

        // 3b. Check HTTP cache.
        let cached_decision = self.http_cache.as_ref().map(|c| c.lookup(&req));
        if let Some(CacheDecision::Fresh(ref resp)) = cached_decision {
            // Cache hit — skip network entirely.
            ctx.record(PipelineDecision::CacheHit {
                url: url.to_string(),
                cache_type: "disk".into(),
            });
            let response = resp.clone();
            self.lifecycle
                .transition(PageState::Loading, "cache hit (fresh)");
            self.pipeline_ctx = Some(ctx);
            return self.finish_navigate(url, response, start, Vec::new());
        }
        if cached_decision.is_some() {
            ctx.record(PipelineDecision::CacheMiss {
                url: url.to_string(),
            });
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
        let tfetch = Instant::now();
        let response = self.http.request(&req)?;
        eprintln!(
            "[profile] html_fetch: {}ms (status {})",
            tfetch.elapsed().as_millis(),
            response.status
        );

        // 3d. Track redirect chain.
        let mut redirect_chain = Vec::new();
        if response.url != url {
            redirect_chain.push(url.to_string());
        }

        // 3e. Log network request.
        self.network_log.push(NetworkLogEntry {
            url: req.url.clone(),
            method: req.method.clone(),
            status: response.status,
            duration_ms: response.duration_ms,
            kind: format!("{:?}", req.context.kind),
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
        self.pipeline_ctx = Some(ctx);
        self.finish_navigate(url, response, start, redirect_chain)
    }

    fn back(&mut self) -> Result<PageResult, EngineError> {
        if self.history_index <= 0 {
            return Err(EngineError::InvalidUrl(
                "no previous page in history".to_string(),
            ));
        }
        let target_index = self.history_index - 1;
        let entry = self.history_stack[target_index as usize].clone();
        let start = Instant::now();
        self.lifecycle
            .transition(PageState::Navigating, "back navigation");
        let mut req = self.build_nav_request(&entry.url);
        if let Some(ref store) = self.cookie_store {
            let is_top = req.context.kind == RequestKind::Navigation;
            let tlu = req.context.top_level_url.clone();
            let cookie_header = store.get_for_request(&req.url, tlu.as_deref(), is_top);
            if !cookie_header.is_empty() {
                req.headers.insert("cookie".to_string(), cookie_header);
            }
        }
        let response = self.http.request(&req)?;
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
            initiator: "back".to_string(),
        });
        self.lifecycle
            .transition(PageState::Loading, "response received");
        let saved_stack = self.history_stack.clone();
        let result = self.finish_navigate(&entry.url, response, start, Vec::new())?;
        self.history_stack = saved_stack;
        self.history_index = target_index;
        Ok(result)
    }

    fn forward(&mut self) -> Result<PageResult, EngineError> {
        let max_index = self.history_stack.len() as isize - 1;
        if self.history_index >= max_index {
            return Err(EngineError::InvalidUrl(
                "no next page in history".to_string(),
            ));
        }
        let target_index = self.history_index + 1;
        let entry = self.history_stack[target_index as usize].clone();
        let start = Instant::now();
        self.lifecycle
            .transition(PageState::Navigating, "forward navigation");
        let mut req = self.build_nav_request(&entry.url);
        if let Some(ref store) = self.cookie_store {
            let is_top = req.context.kind == RequestKind::Navigation;
            let tlu = req.context.top_level_url.clone();
            let cookie_header = store.get_for_request(&req.url, tlu.as_deref(), is_top);
            if !cookie_header.is_empty() {
                req.headers.insert("cookie".to_string(), cookie_header);
            }
        }
        let response = self.http.request(&req)?;
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
            initiator: "forward".to_string(),
        });
        self.lifecycle
            .transition(PageState::Loading, "response received");
        let saved_stack = self.history_stack.clone();
        let result = self.finish_navigate(&entry.url, response, start, Vec::new())?;
        self.history_stack = saved_stack;
        self.history_index = target_index;
        Ok(result)
    }

    fn history(&self) -> Vec<String> {
        self.history_urls()
    }

    fn page_state(&self) -> PageState {
        self.lifecycle.current()
    }

    fn eval(&mut self, js: &str) -> Result<String, EngineError> {
        // Reset callback budget before interactive eval.
        // Page load may exhaust the 5000-callback budget (React + modules),
        // which silently kills queueMicrotask and breaks Promise.then.
        if let Some(rt) = self.runtime.as_mut() {
            let _ = rt.execute("if(typeof __neo_resetBudget==='function')__neo_resetBudget()");
        }

        let result = match self.runtime.as_mut() {
            Some(rt) => {
                let r = rt.eval_and_settle(js, 5_000)?;
                Ok(r.value)
            }
            None => Err(EngineError::Runtime(neo_runtime::RuntimeError::Eval(
                "no runtime available".into(),
            ))),
        }?;
        // Pump event loop for async side effects (fetch, timers, React renders).
        self.pump_after_interaction();
        self.process_pending_navigations();
        Ok(result)
    }

    fn click(&mut self, target: &str) -> Result<ClickResult, EngineError> {
        self.tracer.intent("click", "click", target, 1.0);
        // Try LiveDom (V8) first, fallback to static DOM interactor.
        let result = if let Some(rt) = self.runtime.as_mut() {
            let mut live = LiveDom::new(rt.as_mut());
            match live.click(target) {
                Ok(r) => ClickResult::DomChanged(r.mutations),
                Err(_) => self.interactor.click(target)?,
            }
        } else {
            self.interactor.click(target)?
        };
        self.tracer
            .action_result("click", true, &format!("{result:?}"), None);
        self.pump_after_interaction();
        self.process_pending_navigations();
        Ok(result)
    }

    fn type_text(&mut self, target: &str, text: &str) -> Result<(), EngineError> {
        self.tracer.intent("type", "type_text", target, 1.0);
        if let Some(rt) = self.runtime.as_mut() {
            let mut live = LiveDom::new(rt.as_mut());
            live.type_text(target, text).map_err(|e| {
                EngineError::Runtime(neo_runtime::RuntimeError::Eval(e.to_string()))
            })?;
        } else {
            self.interactor.type_text(target, text, true)?;
        }
        self.tracer.action_result("type", true, "text typed", None);
        self.pump_after_interaction();
        self.process_pending_navigations();
        Ok(())
    }

    fn fill_form(&mut self, fields: &HashMap<String, String>) -> Result<(), EngineError> {
        self.tracer.intent("fill", "fill_form", "form", 1.0);
        if let Some(rt) = self.runtime.as_mut() {
            let mut live = LiveDom::new(rt.as_mut());
            // Use smart fill (name/label/placeholder/aria-label lookup + React compat)
            live.fill_form_smart(fields).map_err(|e| {
                EngineError::Runtime(neo_runtime::RuntimeError::Eval(e.to_string()))
            })?;
        } else {
            self.interactor.fill_form(fields)?;
        }
        self.tracer.action_result("fill", true, "form filled", None);
        self.pump_after_interaction();
        self.process_pending_navigations();
        Ok(())
    }

    fn find_element(&mut self, query: &str) -> Result<Vec<crate::FoundElement>, EngineError> {
        if let Some(rt) = self.runtime.as_mut() {
            let mut live = LiveDom::new(rt.as_mut());
            live.find_element(query).map_err(|e| {
                EngineError::Runtime(neo_runtime::RuntimeError::Eval(e.to_string()))
            })
        } else {
            // No JS runtime — can't search live DOM
            Ok(vec![])
        }
    }

    fn submit(&mut self, target: Option<&str>) -> Result<SubmitResult, EngineError> {
        let t = target.unwrap_or("form");
        self.tracer.intent("submit", "submit", t, 1.0);
        if let Some(rt) = self.runtime.as_mut() {
            let mut live = LiveDom::new(rt.as_mut());
            if live.submit(t).is_err() {
                let _ = self.interactor.submit(target);
            }
        } else {
            let _result = self.interactor.submit(target)?;
        }
        self.tracer
            .action_result("submit", true, "submitted", None);
        self.pump_after_interaction();
        self.process_pending_navigations();
        Ok(SubmitResult::Navigation(String::new()))
    }

    fn extract(&self) -> Result<WomDocument, EngineError> {
        let dom = self.dom.lock().expect("dom lock poisoned");
        let wom = self.extractor.extract_wom(dom.as_ref());
        Ok(wom)
    }

    fn press_key(&mut self, target: &str, key: &str) -> Result<(), EngineError> {
        self.tracer.intent("press_key", "press_key", target, 1.0);
        match self.runtime.as_mut() {
            Some(rt) => {
                let mut live = LiveDom::new(rt.as_mut());
                let _result = live.press_key(target, key).map_err(|e| {
                    EngineError::Runtime(neo_runtime::RuntimeError::Eval(e.to_string()))
                })?;
                self.tracer
                    .action_result("press_key", true, &format!("key={key}"), None);
            }
            None => {
                return Err(EngineError::Runtime(neo_runtime::RuntimeError::Eval(
                    "no runtime available".into(),
                )));
            }
        }
        self.pump_after_interaction();
        self.process_pending_navigations();
        Ok(())
    }

    fn wait_for(&mut self, selector: &str, timeout_ms: u32) -> Result<bool, EngineError> {
        match self.runtime.as_mut() {
            Some(rt) => {
                let mut live = LiveDom::new(rt.as_mut());
                match live.wait_for(selector, timeout_ms) {
                    Ok(found) => Ok(found),
                    Err(crate::LiveDomError::Timeout { .. }) => Ok(false),
                    Err(e) => Err(EngineError::Runtime(neo_runtime::RuntimeError::Eval(
                        e.to_string(),
                    ))),
                }
            }
            None => {
                // Without runtime, check DOM directly.
                let dom = self.dom.lock().expect("dom lock poisoned");
                Ok(dom.query_selector(selector).is_some())
            }
        }
    }

    fn extract_text(&mut self) -> Result<String, EngineError> {
        match self.runtime.as_mut() {
            Some(rt) => {
                let mut live = LiveDom::new(rt.as_mut());
                live.page_text().map_err(|e| {
                    EngineError::Runtime(neo_runtime::RuntimeError::Eval(e.to_string()))
                })
            }
            None => {
                // Fallback: extract from WOM.
                let dom = self.dom.lock().expect("dom lock poisoned");
                let wom = self.extractor.extract_wom(dom.as_ref());
                let mut buf = String::new();
                for node in &wom.nodes {
                    buf.push_str(&node.label);
                    buf.push(' ');
                }
                Ok(buf.trim().to_string())
            }
        }
    }

    fn extract_links(&mut self) -> Result<Vec<(String, String)>, EngineError> {
        match self.runtime.as_mut() {
            Some(rt) => {
                let mut live = LiveDom::new(rt.as_mut());
                live.links().map_err(|e| {
                    EngineError::Runtime(neo_runtime::RuntimeError::Eval(e.to_string()))
                })
            }
            None => {
                // Fallback: extract from WOM nodes with href.
                let dom = self.dom.lock().expect("dom lock poisoned");
                let wom = self.extractor.extract_wom(dom.as_ref());
                Ok(wom
                    .nodes
                    .iter()
                    .filter_map(|n| {
                        n.href
                            .as_ref()
                            .map(|href| (n.label.clone(), href.clone()))
                    })
                    .collect())
            }
        }
    }

    fn extract_semantic(&mut self) -> Result<String, EngineError> {
        match self.runtime.as_mut() {
            Some(rt) => {
                let mut live = LiveDom::new(rt.as_mut());
                live.semantic_text().map_err(|e| {
                    EngineError::Runtime(neo_runtime::RuntimeError::Eval(e.to_string()))
                })
            }
            None => {
                // Fallback: WOM summary.
                let dom = self.dom.lock().expect("dom lock poisoned");
                let wom = self.extractor.extract_wom(dom.as_ref());
                Ok(wom.summary)
            }
        }
    }

    fn current_url(&mut self) -> Result<String, EngineError> {
        match self.runtime.as_mut() {
            Some(rt) => {
                let mut live = LiveDom::new(rt.as_mut());
                live.current_url().map_err(|e| {
                    EngineError::Runtime(neo_runtime::RuntimeError::Eval(e.to_string()))
                })
            }
            None => {
                // Fallback: last history entry.
                Ok(self
                    .history_stack
                    .last()
                    .map(|e| e.url.clone())
                    .unwrap_or_default())
            }
        }
    }

    fn session_state(&self) -> SessionState {
        match self.lifecycle.current() {
            PageState::Idle => SessionState::Idle,
            PageState::Navigating | PageState::Loading => SessionState::Navigating,
            _ => {
                if self.history_stack.is_empty() {
                    SessionState::Idle
                } else {
                    SessionState::Ready
                }
            }
        }
    }

    fn trace(&self) -> Vec<TraceEntry> {
        self.tracer.export()
    }

    fn summary(&self) -> ExecutionSummary {
        self.tracer.summary()
    }

    fn page_id(&self) -> u64 {
        self.page_id.load(Ordering::Relaxed)
    }
}
