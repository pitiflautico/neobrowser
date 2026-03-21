//! NeoSession — the main `BrowserEngine` implementation.
//!
//! Wires HTTP, DOM, JS runtime, interaction, extraction, and tracing
//! into the navigation lifecycle.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use neo_dom::DomEngine;
use neo_extract::{Extractor, WomDocument};
use neo_http::{HttpClient, HttpRequest, RequestContext, RequestKind};
use neo_interact::{ClickResult, Interactor, SubmitResult};
use neo_runtime::JsRuntime;
use neo_trace::{ExecutionSummary, Tracer};
use neo_types::{PageState, TraceEntry};

use crate::config::EngineConfig;
use crate::lifecycle::Lifecycle;
use crate::{BrowserEngine, EngineError, PageResult};

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
    history: Vec<String>,
    /// Cached WOM from last navigation.
    last_wom: Option<WomDocument>,
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
            history: Vec::new(),
            last_wom: None,
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
            history: Vec::new(),
            last_wom: None,
        }
    }

    /// Navigation history (all visited URLs).
    pub fn history(&self) -> &[String] {
        &self.history
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
                referrer: self.history.last().cloned(),
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

        // 3. HTTP fetch.
        let req = self.build_nav_request(url);
        let response = self.http.request(&req)?;

        // 4. Loading.
        self.lifecycle
            .transition(PageState::Loading, "response received");

        // 5. DOM parse.
        {
            let mut dom = self.dom.lock().expect("dom lock poisoned");
            dom.parse_html(&response.body, &response.url)?;
        }

        // 6. Interactive.
        self.lifecycle
            .transition(PageState::Interactive, "dom parsed");

        // 7. JS execution (if enabled and runtime available).
        if self.config.execute_js {
            if let Some(rt) = self.runtime.as_mut() {
                rt.set_document_html(&response.body, &response.url)?;
                rt.run_until_settled(self.config.script_timeout_ms)?;
            }
        }

        // 8. Settled.
        self.lifecycle
            .transition(PageState::Settled, "scripts executed");

        // 9. Extract WOM.
        let mut wom = {
            let dom = self.dom.lock().expect("dom lock poisoned");
            self.extractor.extract_wom(dom.as_ref())
        };
        // Patch URL into WOM (extractors don't have access to the response URL).
        if wom.url.is_empty() {
            wom.url = response.url.clone();
        }
        self.last_wom = Some(wom.clone());

        // 10. Trace result.
        self.tracer
            .action_result("navigate", true, "page loaded", None);

        // 11. Complete.
        self.lifecycle
            .transition(PageState::Complete, "extraction done");

        // 12. Track history.
        self.history.push(url.to_string());

        let title = {
            let dom = self.dom.lock().expect("dom lock poisoned");
            dom.title()
        };
        let elapsed = start.elapsed().as_millis() as u64;

        Ok(PageResult {
            url: response.url,
            title,
            state: self.lifecycle.current(),
            render_ms: elapsed,
            wom,
            errors: Vec::new(),
        })
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
