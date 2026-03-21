//! BrowserEngine trait implementation for NeoSession.

use std::collections::HashMap;
use std::time::Instant;

use neo_extract::WomDocument;
use neo_http::{CacheDecision, RequestKind};
use neo_interact::{ClickResult, SubmitResult};
use neo_trace::ExecutionSummary;
use neo_types::{NetworkLogEntry, PageState, TraceEntry};

use super::NeoSession;
use crate::pipeline::{PipelineContext, PipelineDecision, PipelinePhase};
use crate::{BrowserEngine, EngineError, PageResult};

impl BrowserEngine for NeoSession {
    fn navigate(&mut self, url: &str) -> Result<PageResult, EngineError> {
        let start = Instant::now();

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
        let req = self.build_nav_request(&entry.url);
        let response = self.http.request(&req)?;
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
        let req = self.build_nav_request(&entry.url);
        let response = self.http.request(&req)?;
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
