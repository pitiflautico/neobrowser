//! MockBrowserEngine — for testing neo-mcp without a real browser.
//!
//! Returns configurable results for every operation.

use std::collections::HashMap;

use neo_extract::wom::WomDocument;
use neo_interact::{ClickResult, SubmitResult};
use neo_trace::ExecutionSummary;
use neo_types::{PageState, TraceEntry};

use crate::{BrowserEngine, EngineError, PageResult};

/// Mock browser engine for testing consumers (e.g. neo-mcp).
///
/// Every method returns a configurable default. No real HTTP, DOM, or JS.
pub struct MockBrowserEngine {
    /// Current lifecycle state.
    pub state: PageState,
    /// Pre-configured navigate result.
    pub navigate_result: Option<PageResult>,
    /// Pre-configured eval result.
    pub eval_result: String,
    /// Pre-configured click result.
    pub click_result: ClickResult,
    /// Pre-configured submit result.
    pub submit_result: SubmitResult,
    /// Pre-configured WOM.
    pub wom: WomDocument,
    /// Recorded actions for assertions.
    pub actions: Vec<String>,
    pub history: Vec<String>,
    pub history_index: isize,
}

impl Default for MockBrowserEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl MockBrowserEngine {
    /// Create a new mock engine with sensible defaults.
    pub fn new() -> Self {
        Self {
            state: PageState::Idle,
            navigate_result: None,
            eval_result: "undefined".to_string(),
            click_result: ClickResult::NoEffect,
            submit_result: SubmitResult::NoAction,
            wom: WomDocument {
                url: String::new(),
                title: String::new(),
                nodes: Vec::new(),
                page_type: "mock".to_string(),
                summary: "mock page".to_string(),
            },
            actions: Vec::new(),
            history: Vec::new(),
            history_index: -1,
        }
    }
}

impl BrowserEngine for MockBrowserEngine {
    fn navigate(&mut self, url: &str) -> Result<PageResult, EngineError> {
        self.actions.push(format!("navigate:{url}"));
        self.state = PageState::Complete;
        let new_index = self.history_index + 1;
        self.history.truncate(new_index as usize);
        self.history.push(url.to_string());
        self.history_index = new_index;
        Ok(self.navigate_result.clone().unwrap_or(PageResult {
            url: url.to_string(),
            title: "Mock Page".to_string(),
            state: PageState::Complete,
            render_ms: 0,
            wom: self.wom.clone(),
            errors: Vec::new(),
            redirect_chain: Vec::new(),
        }))
    }

    fn back(&mut self) -> Result<PageResult, EngineError> {
        if self.history_index <= 0 {
            return Err(EngineError::InvalidUrl(
                "no previous page in history".to_string(),
            ));
        }
        self.history_index -= 1;
        let url = self.history[self.history_index as usize].clone();
        self.actions.push(format!("back:{url}"));
        Ok(PageResult {
            url,
            title: "Mock Page".to_string(),
            state: PageState::Complete,
            render_ms: 0,
            wom: self.wom.clone(),
            errors: Vec::new(),
            redirect_chain: Vec::new(),
        })
    }

    fn forward(&mut self) -> Result<PageResult, EngineError> {
        let max_index = self.history.len() as isize - 1;
        if self.history_index >= max_index {
            return Err(EngineError::InvalidUrl(
                "no next page in history".to_string(),
            ));
        }
        self.history_index += 1;
        let url = self.history[self.history_index as usize].clone();
        self.actions.push(format!("forward:{url}"));
        Ok(PageResult {
            url,
            title: "Mock Page".to_string(),
            state: PageState::Complete,
            render_ms: 0,
            wom: self.wom.clone(),
            errors: Vec::new(),
            redirect_chain: Vec::new(),
        })
    }

    fn history(&self) -> Vec<String> {
        self.history.clone()
    }

    fn page_state(&self) -> PageState {
        self.state
    }

    fn eval(&mut self, js: &str) -> Result<String, EngineError> {
        self.actions.push(format!("eval:{js}"));
        Ok(self.eval_result.clone())
    }

    fn click(&mut self, target: &str) -> Result<ClickResult, EngineError> {
        self.actions.push(format!("click:{target}"));
        Ok(self.click_result.clone())
    }

    fn type_text(&mut self, target: &str, text: &str) -> Result<(), EngineError> {
        self.actions.push(format!("type:{target}={text}"));
        Ok(())
    }

    fn fill_form(&mut self, fields: &HashMap<String, String>) -> Result<(), EngineError> {
        self.actions.push(format!("fill:{} fields", fields.len()));
        Ok(())
    }

    fn submit(&mut self, target: Option<&str>) -> Result<SubmitResult, EngineError> {
        self.actions
            .push(format!("submit:{}", target.unwrap_or("none")));
        Ok(self.submit_result.clone())
    }

    fn extract(&self) -> Result<WomDocument, EngineError> {
        Ok(self.wom.clone())
    }

    fn trace(&self) -> Vec<TraceEntry> {
        Vec::new()
    }

    fn summary(&self) -> ExecutionSummary {
        ExecutionSummary {
            total_actions: self.actions.len(),
            succeeded: self.actions.len(),
            failed: 0,
            total_requests: 0,
            blocked_requests: 0,
            dom_changes: 0,
            js_errors: 0,
            duration_ms: 0,
            warnings: Vec::new(),
            state: self.state,
        }
    }
}
