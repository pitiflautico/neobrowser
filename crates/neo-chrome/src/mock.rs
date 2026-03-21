//! Mock Chrome session — no real Chrome needed.
//!
//! Returns configurable page results for testing engine fallback logic.
//! Used by neo-engine tests that need to verify Chrome fallback
//! without launching a browser.

use crate::{ChromeSessionTrait, Result};
use neo_types::{PageResult, PageState};
use std::collections::HashMap;

/// Mock Chrome session that returns pre-configured results.
pub struct MockChromeSession {
    /// URL -> PageResult mapping for navigate calls.
    pages: HashMap<String, PageResult>,
    /// URL -> JS result mapping for eval calls.
    eval_results: HashMap<String, String>,
    /// Default page result when URL not in the map.
    default_result: PageResult,
}

impl MockChromeSession {
    /// Create a new mock session with empty configuration.
    pub fn new() -> Self {
        Self {
            pages: HashMap::new(),
            eval_results: HashMap::new(),
            default_result: Self::blank_result("about:blank"),
        }
    }

    /// Register a page result for a specific URL.
    pub fn add_page(&mut self, url: &str, result: PageResult) {
        self.pages.insert(url.to_string(), result);
    }

    /// Register a JS eval result for a specific expression.
    pub fn add_eval(&mut self, js: &str, result: &str) {
        self.eval_results.insert(js.to_string(), result.to_string());
    }

    /// Set the default page result for unregistered URLs.
    pub fn set_default(&mut self, result: PageResult) {
        self.default_result = result;
    }

    /// Navigate mock — returns registered result or default.
    pub async fn navigate(&mut self, url: &str) -> Result<PageResult> {
        if let Some(result) = self.pages.get(url) {
            Ok(result.clone())
        } else {
            let mut result = self.default_result.clone();
            result.url = url.to_string();
            Ok(result)
        }
    }

    /// Eval mock — returns registered result or the expression itself.
    pub async fn eval(&self, js: &str) -> Result<String> {
        if let Some(result) = self.eval_results.get(js) {
            Ok(result.clone())
        } else {
            Ok(format!("mock:{js}"))
        }
    }

    /// Create a blank PageResult for a URL.
    pub fn blank_result(url: &str) -> PageResult {
        PageResult {
            url: url.to_string(),
            title: String::new(),
            state: PageState::Complete,
            render_ms: 0,
            links: 0,
            forms: 0,
            inputs: 0,
            buttons: 0,
            scripts: 0,
            errors: vec![],
            redirect_chain: vec![],
            page_id: 0,
        }
    }

    /// Create a PageResult with custom element counts.
    pub fn page_result(url: &str, title: &str, links: usize, forms: usize) -> PageResult {
        PageResult {
            url: url.to_string(),
            title: title.to_string(),
            state: PageState::Complete,
            render_ms: 0,
            links,
            forms,
            inputs: 0,
            buttons: 0,
            scripts: 0,
            errors: vec![],
            redirect_chain: vec![],
            page_id: 0,
        }
    }
}

impl Default for MockChromeSession {
    fn default() -> Self {
        Self::new()
    }
}

impl ChromeSessionTrait for MockChromeSession {
    fn navigate(
        &mut self,
        url: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<PageResult>> + Send + '_>> {
        let url = url.to_string();
        Box::pin(async move { self.navigate(&url).await })
    }

    fn eval(
        &self,
        js: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send + '_>> {
        let js = js.to_string();
        Box::pin(async move { self.eval(&js).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_default_result() {
        let mut mock = MockChromeSession::new();
        let result = mock.navigate("https://example.com").await.unwrap();
        assert_eq!(result.url, "https://example.com");
        assert_eq!(result.state, PageState::Complete);
    }

    #[tokio::test]
    async fn test_mock_configured_page() {
        let mut mock = MockChromeSession::new();
        mock.add_page(
            "https://test.com",
            MockChromeSession::page_result("https://test.com", "Test Page", 5, 2),
        );

        let result = mock.navigate("https://test.com").await.unwrap();
        assert_eq!(result.title, "Test Page");
        assert_eq!(result.links, 5);
        assert_eq!(result.forms, 2);
    }

    #[tokio::test]
    async fn test_mock_eval() {
        let mut mock = MockChromeSession::new();
        mock.add_eval("document.title", "Mock Title");

        let result = mock.eval("document.title").await.unwrap();
        assert_eq!(result, "Mock Title");

        // Unknown JS returns mock prefix.
        let fallback = mock.eval("unknown()").await.unwrap();
        assert!(fallback.starts_with("mock:"));
    }
}
