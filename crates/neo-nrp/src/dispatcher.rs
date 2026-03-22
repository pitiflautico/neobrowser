//! NRP command dispatcher — routes JSON-RPC requests to engine operations.
//!
//! Maps NRP domain.method names to `BrowserEngine` trait calls,
//! translating between NRP types and engine types.

use neo_engine::BrowserEngine;

use crate::types::{
    ActionOutcomeKind, ActionResult, NrpError, NrpPageState, NrpRequest, NrpResponse, PageInfo,
};

/// NRP command dispatcher.
///
/// Holds a `BrowserEngine` and a monotonically increasing sequence ID
/// for event ordering. Routes `NrpRequest` messages to the appropriate
/// engine method and wraps results in `NrpResponse`.
pub struct NrpDispatcher {
    engine: Box<dyn BrowserEngine>,
    sequence_id: u64,
    document_epoch: u64,
}

impl NrpDispatcher {
    /// Create a new dispatcher wrapping the given engine.
    pub fn new(engine: Box<dyn BrowserEngine>) -> Self {
        Self {
            engine,
            sequence_id: 0,
            document_epoch: 1,
        }
    }

    /// Next sequence ID (monotonically increasing).
    pub fn next_sequence_id(&mut self) -> u64 {
        self.sequence_id += 1;
        self.sequence_id
    }

    /// Dispatch a request and return a response.
    pub fn dispatch(&mut self, request: NrpRequest) -> NrpResponse {
        match request.method.as_str() {
            "Page.navigate" => self.page_navigate(request),
            "Page.getInfo" => self.page_get_info(request),
            "SemanticTree.get" => self.semantic_tree_get(request),
            "SemanticTree.find" => self.semantic_tree_find(request),
            "Interact.click" => self.interact_click(request),
            "Interact.type" => self.interact_type(request),
            "Runtime.evaluate" => self.runtime_evaluate(request),
            "Wait.forSelector" => self.wait_for_selector(request),
            "Session.getCapabilities" => self.session_capabilities(request),
            _ => NrpResponse::err(
                request.id,
                NrpError::METHOD_NOT_FOUND,
                format!("unknown method: {}", request.method),
            ),
        }
    }

    // ─── Page domain ───

    fn page_navigate(&mut self, req: NrpRequest) -> NrpResponse {
        let url = match req.params.get("url").and_then(|v| v.as_str()) {
            Some(u) => u.to_string(),
            None => {
                return NrpResponse::err(
                    req.id,
                    NrpError::INVALID_PARAMS,
                    "missing required param: url",
                )
            }
        };

        match self.engine.navigate(&url) {
            Ok(result) => {
                self.document_epoch += 1;
                let info = PageInfo {
                    page_id: result.page_id,
                    document_epoch: self.document_epoch,
                    url: result.url,
                    title: result.title,
                    status: 200, // Engine doesn't expose HTTP status directly
                    page_state: page_state_to_nrp(self.engine.page_state()),
                };
                match serde_json::to_value(&info) {
                    Ok(val) => NrpResponse::ok(req.id, val),
                    Err(e) => NrpResponse::err(req.id, NrpError::ENGINE_ERROR, e.to_string()),
                }
            }
            Err(e) => NrpResponse::err(req.id, NrpError::NAVIGATION_FAILED, e.to_string()),
        }
    }

    fn page_get_info(&mut self, req: NrpRequest) -> NrpResponse {
        let url = self
            .engine
            .current_url()
            .unwrap_or_else(|_| "about:blank".to_string());

        // Title requires extract — use WOM if available
        let title = self
            .engine
            .extract()
            .map(|w| w.title)
            .unwrap_or_default();

        let info = PageInfo {
            page_id: self.engine.page_id(),
            document_epoch: self.document_epoch,
            url,
            title,
            status: 200,
            page_state: page_state_to_nrp(self.engine.page_state()),
        };

        match serde_json::to_value(&info) {
            Ok(val) => NrpResponse::ok(req.id, val),
            Err(e) => NrpResponse::err(req.id, NrpError::ENGINE_ERROR, e.to_string()),
        }
    }

    // ─── SemanticTree domain ───

    fn semantic_tree_get(&mut self, req: NrpRequest) -> NrpResponse {
        // For now, return the semantic text representation.
        // Full tree building requires DOM access which the engine
        // doesn't expose directly yet.
        match self.engine.extract_semantic() {
            Ok(semantic) => {
                let result = serde_json::json!({
                    "document_epoch": self.document_epoch,
                    "semantic_text": semantic,
                });
                NrpResponse::ok(req.id, result)
            }
            Err(e) => NrpResponse::err(req.id, NrpError::ENGINE_ERROR, e.to_string()),
        }
    }

    fn semantic_tree_find(&mut self, req: NrpRequest) -> NrpResponse {
        // Delegate to extract_semantic for now — proper find requires
        // the full SemanticNode tree which needs DOM access.
        let role = req.params.get("role").and_then(|v| v.as_str());
        let name = req.params.get("name").and_then(|v| v.as_str());
        let text = req.params.get("text").and_then(|v| v.as_str());

        // Use engine query methods where possible
        if let Some(text_query) = text {
            match self.engine.extract_semantic() {
                Ok(semantic) => {
                    let result = serde_json::json!({
                        "query": { "text": text_query, "role": role, "name": name },
                        "semantic_text": semantic,
                        "document_epoch": self.document_epoch,
                    });
                    NrpResponse::ok(req.id, result)
                }
                Err(e) => NrpResponse::err(req.id, NrpError::ENGINE_ERROR, e.to_string()),
            }
        } else if role.is_some() || name.is_some() {
            match self.engine.extract_semantic() {
                Ok(semantic) => {
                    let result = serde_json::json!({
                        "query": { "role": role, "name": name },
                        "semantic_text": semantic,
                        "document_epoch": self.document_epoch,
                    });
                    NrpResponse::ok(req.id, result)
                }
                Err(e) => NrpResponse::err(req.id, NrpError::ENGINE_ERROR, e.to_string()),
            }
        } else {
            NrpResponse::err(
                req.id,
                NrpError::INVALID_PARAMS,
                "at least one of role, name, or text is required",
            )
        }
    }

    // ─── Interact domain ───

    fn interact_click(&mut self, req: NrpRequest) -> NrpResponse {
        let node_id = match req.params.get("node_id").and_then(|v| v.as_str()) {
            Some(id) => id.to_string(),
            None => {
                return NrpResponse::err(
                    req.id,
                    NrpError::INVALID_PARAMS,
                    "missing required param: node_id",
                )
            }
        };

        match self.engine.click(&node_id) {
            Ok(click_result) => {
                let (outcome, mutations, navigation) = match click_result {
                    neo_interact::ClickResult::NoEffect => {
                        (ActionOutcomeKind::NoEffect, None, None)
                    }
                    neo_interact::ClickResult::DomChanged(count) => {
                        (ActionOutcomeKind::DomChanged, Some(count as u32), None)
                    }
                    neo_interact::ClickResult::Navigation(url) => (
                        ActionOutcomeKind::HttpNavigation,
                        None,
                        Some(crate::types::NavigationInfo {
                            url,
                            method: "GET".to_string(),
                            status: None,
                        }),
                    ),
                };

                let action_result = ActionResult {
                    outcome,
                    page_id: self.engine.page_id(),
                    document_epoch: self.document_epoch,
                    dom_changed: !matches!(outcome, ActionOutcomeKind::NoEffect),
                    value_after: None,
                    selection_after: None,
                    navigation,
                    mutations,
                    focus_change: None,
                    error: None,
                };

                match serde_json::to_value(&action_result) {
                    Ok(val) => NrpResponse::ok(req.id, val),
                    Err(e) => NrpResponse::err(req.id, NrpError::ENGINE_ERROR, e.to_string()),
                }
            }
            Err(e) => NrpResponse::err(req.id, NrpError::TARGET_NOT_FOUND, e.to_string()),
        }
    }

    fn interact_type(&mut self, req: NrpRequest) -> NrpResponse {
        let node_id = match req.params.get("node_id").and_then(|v| v.as_str()) {
            Some(id) => id.to_string(),
            None => {
                return NrpResponse::err(
                    req.id,
                    NrpError::INVALID_PARAMS,
                    "missing required param: node_id",
                )
            }
        };
        let text = match req.params.get("text").and_then(|v| v.as_str()) {
            Some(t) => t.to_string(),
            None => {
                return NrpResponse::err(
                    req.id,
                    NrpError::INVALID_PARAMS,
                    "missing required param: text",
                )
            }
        };

        match self.engine.type_text(&node_id, &text) {
            Ok(()) => {
                let action_result = ActionResult {
                    outcome: ActionOutcomeKind::DomChanged,
                    page_id: self.engine.page_id(),
                    document_epoch: self.document_epoch,
                    dom_changed: true,
                    value_after: Some(text),
                    selection_after: None,
                    navigation: None,
                    mutations: None,
                    focus_change: None,
                    error: None,
                };
                match serde_json::to_value(&action_result) {
                    Ok(val) => NrpResponse::ok(req.id, val),
                    Err(e) => NrpResponse::err(req.id, NrpError::ENGINE_ERROR, e.to_string()),
                }
            }
            Err(e) => NrpResponse::err(req.id, NrpError::TARGET_NOT_FOUND, e.to_string()),
        }
    }

    // ─── Runtime domain ───

    fn runtime_evaluate(&mut self, req: NrpRequest) -> NrpResponse {
        let expression = match req.params.get("expression").and_then(|v| v.as_str()) {
            Some(e) => e.to_string(),
            None => {
                return NrpResponse::err(
                    req.id,
                    NrpError::INVALID_PARAMS,
                    "missing required param: expression",
                )
            }
        };

        match self.engine.eval(&expression) {
            Ok(result) => {
                let val = serde_json::json!({
                    "result": result,
                    "type": "string",
                });
                NrpResponse::ok(req.id, val)
            }
            Err(e) => NrpResponse::err(req.id, NrpError::ENGINE_ERROR, e.to_string()),
        }
    }

    // ─── Wait domain ───

    fn wait_for_selector(&mut self, req: NrpRequest) -> NrpResponse {
        let css = match req.params.get("css").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => {
                return NrpResponse::err(
                    req.id,
                    NrpError::INVALID_PARAMS,
                    "missing required param: css",
                )
            }
        };
        let timeout_ms = req
            .params
            .get("timeout_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(5000) as u32;

        match self.engine.wait_for(&css, timeout_ms) {
            Ok(found) => {
                let result = serde_json::json!({
                    "found": found,
                });
                NrpResponse::ok(req.id, result)
            }
            Err(e) => NrpResponse::err(req.id, NrpError::ENGINE_ERROR, e.to_string()),
        }
    }

    // ─── Session domain ───

    fn session_capabilities(&self, req: NrpRequest) -> NrpResponse {
        let caps = serde_json::json!({
            "protocol_version": "0.1.0",
            "engine_version": env!("CARGO_PKG_VERSION"),
            "domains": [
                "Page", "SemanticTree", "Interact",
                "Runtime", "Wait", "Session"
            ],
            "features": {
                "semantic_tree": true,
                "target_resolution": true,
                "network_interception": false,
                "event_subscriptions": false,
                "cookies": false,
                "storage": false,
            }
        });
        NrpResponse::ok(req.id, caps)
    }
}

/// Convert engine `PageState` to NRP `NrpPageState`.
fn page_state_to_nrp(state: neo_types::PageState) -> NrpPageState {
    match state {
        neo_types::PageState::Idle => NrpPageState::Idle,
        neo_types::PageState::Navigating | neo_types::PageState::Loading => NrpPageState::Loading,
        neo_types::PageState::Interactive | neo_types::PageState::Hydrated => {
            NrpPageState::Interactive
        }
        neo_types::PageState::Settled | neo_types::PageState::Complete => NrpPageState::Settled,
        neo_types::PageState::Blocked | neo_types::PageState::Failed => NrpPageState::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use neo_engine::MockBrowserEngine;

    fn make_request(method: &str, params: serde_json::Value) -> NrpRequest {
        NrpRequest {
            id: 1,
            method: method.to_string(),
            params,
        }
    }

    #[test]
    fn test_dispatch_unknown_method() {
        let engine = MockBrowserEngine::new();
        let mut dispatcher = NrpDispatcher::new(Box::new(engine));
        let req = make_request("Unknown.method", serde_json::json!({}));
        let resp = dispatcher.dispatch(req);
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, NrpError::METHOD_NOT_FOUND);
    }

    #[test]
    fn test_page_navigate() {
        let engine = MockBrowserEngine::new();
        let mut dispatcher = NrpDispatcher::new(Box::new(engine));

        let req = make_request(
            "Page.navigate",
            serde_json::json!({"url": "https://example.com"}),
        );
        let resp = dispatcher.dispatch(req);
        assert!(resp.error.is_none());

        let result = resp.result.unwrap();
        assert_eq!(result["url"], "https://example.com");
        assert_eq!(result["page_state"], "Settled");
    }

    #[test]
    fn test_page_navigate_missing_url() {
        let engine = MockBrowserEngine::new();
        let mut dispatcher = NrpDispatcher::new(Box::new(engine));

        let req = make_request("Page.navigate", serde_json::json!({}));
        let resp = dispatcher.dispatch(req);
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, NrpError::INVALID_PARAMS);
    }

    #[test]
    fn test_page_get_info() {
        let mut engine = MockBrowserEngine::new();
        // Navigate first so there's a URL
        engine.navigate("https://example.com").unwrap();

        let mut dispatcher = NrpDispatcher::new(Box::new(engine));
        let req = make_request("Page.getInfo", serde_json::json!({}));
        let resp = dispatcher.dispatch(req);
        assert!(resp.error.is_none());

        let result = resp.result.unwrap();
        assert!(result.get("url").is_some());
        assert!(result.get("page_state").is_some());
    }

    #[test]
    fn test_session_capabilities() {
        let engine = MockBrowserEngine::new();
        let mut dispatcher = NrpDispatcher::new(Box::new(engine));

        let req = make_request("Session.getCapabilities", serde_json::json!({}));
        let resp = dispatcher.dispatch(req);
        assert!(resp.error.is_none());

        let result = resp.result.unwrap();
        assert_eq!(result["protocol_version"], "0.1.0");
        assert!(result["domains"].is_array());
    }

    #[test]
    fn test_interact_click() {
        let engine = MockBrowserEngine::new();
        let mut dispatcher = NrpDispatcher::new(Box::new(engine));

        let req = make_request(
            "Interact.click",
            serde_json::json!({"node_id": "n1"}),
        );
        let resp = dispatcher.dispatch(req);
        assert!(resp.error.is_none());

        let result = resp.result.unwrap();
        assert_eq!(result["outcome"], "NoEffect");
    }

    #[test]
    fn test_interact_type() {
        let engine = MockBrowserEngine::new();
        let mut dispatcher = NrpDispatcher::new(Box::new(engine));

        let req = make_request(
            "Interact.type",
            serde_json::json!({"node_id": "n2", "text": "hello"}),
        );
        let resp = dispatcher.dispatch(req);
        assert!(resp.error.is_none());

        let result = resp.result.unwrap();
        assert_eq!(result["outcome"], "DomChanged");
        assert_eq!(result["value_after"], "hello");
    }

    #[test]
    fn test_runtime_evaluate() {
        let engine = MockBrowserEngine::new();
        let mut dispatcher = NrpDispatcher::new(Box::new(engine));

        let req = make_request(
            "Runtime.evaluate",
            serde_json::json!({"expression": "1 + 1"}),
        );
        let resp = dispatcher.dispatch(req);
        assert!(resp.error.is_none());

        let result = resp.result.unwrap();
        assert_eq!(result["type"], "string");
    }

    #[test]
    fn test_wait_for_selector() {
        let engine = MockBrowserEngine::new();
        let mut dispatcher = NrpDispatcher::new(Box::new(engine));

        let req = make_request(
            "Wait.forSelector",
            serde_json::json!({"css": "button.submit", "timeout_ms": 3000}),
        );
        let resp = dispatcher.dispatch(req);
        assert!(resp.error.is_none());

        let result = resp.result.unwrap();
        assert_eq!(result["found"], true);
    }

    #[test]
    fn test_sequence_id_monotonic() {
        let engine = MockBrowserEngine::new();
        let mut dispatcher = NrpDispatcher::new(Box::new(engine));
        let id1 = dispatcher.next_sequence_id();
        let id2 = dispatcher.next_sequence_id();
        let id3 = dispatcher.next_sequence_id();
        assert!(id1 < id2);
        assert!(id2 < id3);
    }
}
