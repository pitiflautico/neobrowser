//! Navigation pipeline — DOM parsing, JS execution, WOM extraction.

use std::collections::HashMap;
use std::time::Instant;

use neo_http::{HttpRequest, RequestContext, RequestKind};
use neo_types::{NetworkLogEntry, PageState};

use super::scripts::{detect_meta_refresh, extract_scripts, ScriptInfo};
use super::{HistoryEntry, NeoSession};
use crate::pipeline::PipelinePhase;
use crate::{EngineError, PageResult};

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
        {
            let mut dom = self.dom.lock().expect("dom lock poisoned");
            dom.parse_html(&response.body, &response.url)?;
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

        // Extract WOM.
        if let Some(ref mut ctx) = self.pipeline_ctx {
            ctx.enter_phase(PipelinePhase::Extract);
        }
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

    /// Execute inline and external scripts from the page HTML.
    fn execute_page_scripts(&mut self, response: &neo_types::HttpResponse) -> Vec<String> {
        let mut js_errors: Vec<String> = Vec::new();
        if !self.config.execute_js {
            return js_errors;
        }
        let Some(rt) = self.runtime.as_mut() else {
            return js_errors;
        };

        // 1. Initialize the V8 DOM with the page HTML + bootstrap globals.
        if let Err(e) = rt.set_document_html(&response.body, &response.url) {
            js_errors.push(format!("set_document_html: {e}"));
            return js_errors;
        }

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

        js_errors
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
}
