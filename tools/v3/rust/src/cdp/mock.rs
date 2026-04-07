//! Mock CDP transport for unit testing.
//!
//! Allows programming expected CDP responses without a real Chrome.
//! Tracks all calls for assertion.

use super::{CdpTransport, CdpResult, TransportEventCallback};
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

/// A single expected CDP call with its response.
#[derive(Clone)]
struct Expectation {
    response: Result<Value, String>,
}

/// Record of a CDP call made through the mock.
#[derive(Debug, Clone)]
pub struct CallRecord {
    pub method: String,
    pub params: Value,
    pub session_id: Option<String>,
}

/// Mock transport for testing CDP domain modules without Chrome.
pub struct MockTransport {
    /// Method -> queue of responses (FIFO)
    expectations: Arc<Mutex<HashMap<String, Vec<Expectation>>>>,
    /// All calls made, in order
    calls: Arc<Mutex<Vec<CallRecord>>>,
    /// Event listeners registered
    listeners: Arc<Mutex<HashMap<String, Vec<TransportEventCallback>>>>,
    /// Simulated alive state
    alive: AtomicBool,
    /// Default response for unexpected calls (None = error)
    default_response: Arc<Mutex<Option<Value>>>,
}

impl MockTransport {
    /// Create a new mock transport (alive by default).
    pub fn new() -> Self {
        Self {
            expectations: Arc::new(Mutex::new(HashMap::new())),
            calls: Arc::new(Mutex::new(Vec::new())),
            listeners: Arc::new(Mutex::new(HashMap::new())),
            alive: AtomicBool::new(true),
            default_response: Arc::new(Mutex::new(None)),
        }
    }

    /// Program an expected response for a CDP method.
    /// Multiple calls to expect() for the same method queue responses (FIFO).
    pub async fn expect(&self, method: &str, response: Value) {
        let mut expectations = self.expectations.lock().await;
        expectations
            .entry(method.to_string())
            .or_default()
            .push(Expectation {
                response: Ok(response),
            });
    }

    /// Program an error response for a CDP method.
    pub async fn expect_error(&self, method: &str, error_message: &str) {
        let mut expectations = self.expectations.lock().await;
        expectations
            .entry(method.to_string())
            .or_default()
            .push(Expectation {
                response: Err(error_message.to_string()),
            });
    }

    /// Set a default response for any method without explicit expectations.
    pub async fn set_default_response(&self, response: Value) {
        let mut default = self.default_response.lock().await;
        *default = Some(response);
    }

    /// Set alive state (simulate connection drop).
    pub fn set_alive(&self, alive: bool) {
        self.alive.store(alive, Ordering::SeqCst);
    }

    /// Get all recorded calls.
    pub async fn calls(&self) -> Vec<CallRecord> {
        self.calls.lock().await.clone()
    }

    /// Count how many times a method was called.
    pub async fn call_count(&self, method: &str) -> usize {
        self.calls
            .lock()
            .await
            .iter()
            .filter(|c| c.method == method)
            .count()
    }

    /// Assert a method was called exactly N times.
    pub async fn assert_called(&self, method: &str, times: usize) {
        let count = self.call_count(method).await;
        assert_eq!(
            count, times,
            "Expected {method} to be called {times} times, but was called {count} times"
        );
    }

    /// Assert a method was called at least once.
    pub async fn assert_called_once(&self, method: &str) {
        let count = self.call_count(method).await;
        assert!(
            count >= 1,
            "Expected {method} to be called at least once, but was called {count} times"
        );
    }

    /// Get the params of the Nth call to a method (0-indexed).
    pub async fn call_params(&self, method: &str, index: usize) -> Option<Value> {
        self.calls
            .lock()
            .await
            .iter()
            .filter(|c| c.method == method)
            .nth(index)
            .map(|c| c.params.clone())
    }

    /// Fire a simulated CDP event to all registered listeners.
    pub async fn fire_event(&self, event: &str, params: Value) {
        let listeners = self.listeners.lock().await;
        if let Some(cbs) = listeners.get(event) {
            for cb in cbs {
                cb(params.clone());
            }
        }
    }

    /// Clear all expectations and call records.
    pub async fn reset(&self) {
        self.expectations.lock().await.clear();
        self.calls.lock().await.clear();
    }

    /// Internal: resolve a call against expectations.
    async fn resolve(&self, method: &str, params: Value, session_id: Option<String>) -> CdpResult<Value> {
        // Record the call
        self.calls.lock().await.push(CallRecord {
            method: method.to_string(),
            params: params.clone(),
            session_id,
        });

        // Check if dead
        if !self.alive.load(Ordering::SeqCst) {
            return Err("CDP connection dead".into());
        }

        // Try to find an expectation
        let mut expectations = self.expectations.lock().await;
        if let Some(queue) = expectations.get_mut(method) {
            if !queue.is_empty() {
                let exp = queue.remove(0);
                return match exp.response {
                    Ok(v) => Ok(v),
                    Err(msg) => Err(msg.into()),
                };
            }
        }

        // Fallback to default
        let default = self.default_response.lock().await;
        if let Some(ref v) = *default {
            return Ok(v.clone());
        }

        Err(format!("MockTransport: unexpected call to {method}").into())
    }
}

impl CdpTransport for MockTransport {
    fn send(
        &self,
        method: &str,
        params: Value,
    ) -> Pin<Box<dyn Future<Output = CdpResult<Value>> + Send + '_>> {
        let method = method.to_string();
        Box::pin(async move { self.resolve(&method, params, None).await })
    }

    fn send_to(
        &self,
        session_id: &str,
        method: &str,
        params: Value,
    ) -> Pin<Box<dyn Future<Output = CdpResult<Value>> + Send + '_>> {
        let method = method.to_string();
        let session_id = session_id.to_string();
        Box::pin(async move {
            self.resolve(&method, params, Some(session_id)).await
        })
    }

    fn on_event(
        &self,
        event: &str,
        callback: TransportEventCallback,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        let event = event.to_string();
        Box::pin(async move {
            let mut listeners = self.listeners.lock().await;
            listeners.entry(event).or_default().push(callback);
        })
    }

    fn wait_for_event(
        &self,
        event: &str,
        timeout_ms: u64,
    ) -> Pin<Box<dyn Future<Output = CdpResult<Value>> + Send + '_>> {
        let event = event.to_string();
        Box::pin(async move {
            // Create a oneshot channel
            let (tx, rx) = tokio::sync::oneshot::channel::<Value>();
            let tx = Arc::new(Mutex::new(Some(tx)));

            // Register listener
            let tx_clone = tx.clone();
            let callback: TransportEventCallback = Arc::new(move |params| {
                let tx = tx_clone.clone();
                tokio::spawn(async move {
                    if let Some(sender) = tx.lock().await.take() {
                        let _ = sender.send(params);
                    }
                });
            });

            {
                let mut listeners = self.listeners.lock().await;
                listeners.entry(event).or_default().push(callback);
            }

            // Wait with timeout
            let result = tokio::time::timeout(
                std::time::Duration::from_millis(timeout_ms),
                rx,
            )
            .await
            .map_err(|_| -> Box<dyn std::error::Error + Send + Sync> { "Timeout waiting for event".into() })?
            .map_err(|_| -> Box<dyn std::error::Error + Send + Sync> { "Event channel closed".into() })?;

            Ok(result)
        })
    }

    fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn mock_responds_to_expected_method() {
        let mock = MockTransport::new();
        mock.expect("Page.navigate", json!({"frameId": "abc123"})).await;

        let result = mock
            .send("Page.navigate", json!({"url": "https://example.com"}))
            .await
            .unwrap();

        assert_eq!(result["frameId"], "abc123");
    }

    #[tokio::test]
    async fn mock_fails_on_unexpected_method() {
        let mock = MockTransport::new();

        let result = mock.send("Page.navigate", json!({})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn mock_error_response() {
        let mock = MockTransport::new();
        mock.expect_error("Page.navigate", "Page not found").await;

        let result = mock.send("Page.navigate", json!({})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Page not found"));
    }

    #[tokio::test]
    async fn mock_records_calls() {
        let mock = MockTransport::new();
        mock.set_default_response(json!({})).await;

        mock.send("Page.enable", json!(null)).await.unwrap();
        mock.send("Runtime.enable", json!(null)).await.unwrap();
        mock.send("Page.navigate", json!({"url": "https://x.com"})).await.unwrap();

        mock.assert_called("Page.enable", 1).await;
        mock.assert_called("Runtime.enable", 1).await;
        mock.assert_called("Page.navigate", 1).await;
        assert_eq!(mock.call_count("DOM.enable").await, 0);
    }

    #[tokio::test]
    async fn mock_call_params_accessible() {
        let mock = MockTransport::new();
        mock.expect("Page.navigate", json!({"frameId": "f1"})).await;

        mock.send("Page.navigate", json!({"url": "https://test.com"})).await.unwrap();

        let params = mock.call_params("Page.navigate", 0).await.unwrap();
        assert_eq!(params["url"], "https://test.com");
    }

    #[tokio::test]
    async fn mock_fifo_expectations() {
        let mock = MockTransport::new();
        mock.expect("Runtime.evaluate", json!({"result": {"value": 1}})).await;
        mock.expect("Runtime.evaluate", json!({"result": {"value": 2}})).await;

        let r1 = mock.send("Runtime.evaluate", json!({})).await.unwrap();
        let r2 = mock.send("Runtime.evaluate", json!({})).await.unwrap();

        assert_eq!(r1["result"]["value"], 1);
        assert_eq!(r2["result"]["value"], 2);
    }

    #[tokio::test]
    async fn mock_dead_connection() {
        let mock = MockTransport::new();
        mock.set_alive(false);

        let result = mock.send("Page.enable", json!(null)).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("dead"));
    }

    #[tokio::test]
    async fn mock_fires_events() {
        let mock = MockTransport::new();

        let received = Arc::new(Mutex::new(None));
        let received_clone = received.clone();

        mock.on_event(
            "Page.loadEventFired",
            Arc::new(move |params| {
                let received = received_clone.clone();
                tokio::spawn(async move {
                    *received.lock().await = Some(params);
                });
            }),
        )
        .await;

        mock.fire_event("Page.loadEventFired", json!({"timestamp": 12345.0})).await;

        // Give the spawned task a moment
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let val = received.lock().await;
        assert!(val.is_some());
        assert_eq!(val.as_ref().unwrap()["timestamp"], 12345.0);
    }

    #[tokio::test]
    async fn mock_send_to_with_session() {
        let mock = MockTransport::new();
        mock.expect("Page.navigate", json!({"frameId": "f1"})).await;

        let result = mock
            .send_to("session-abc", "Page.navigate", json!({"url": "https://x.com"}))
            .await
            .unwrap();

        assert_eq!(result["frameId"], "f1");

        let calls = mock.calls().await;
        assert_eq!(calls[0].session_id, Some("session-abc".to_string()));
    }

    #[tokio::test]
    async fn mock_default_response() {
        let mock = MockTransport::new();
        mock.set_default_response(json!({})).await;

        // Any method works with default
        let r = mock.send("SomeRandom.method", json!({})).await;
        assert!(r.is_ok());
    }

    #[tokio::test]
    async fn mock_reset_clears_state() {
        let mock = MockTransport::new();
        mock.set_default_response(json!({})).await;
        mock.send("Page.enable", json!(null)).await.unwrap();

        mock.reset().await;

        assert_eq!(mock.call_count("Page.enable").await, 0);
        // After reset, no expectations left
        let r = mock.send("Page.enable", json!(null)).await;
        // default_response is NOT cleared by reset, only expectations and calls
        assert!(r.is_ok());
    }

    #[tokio::test]
    async fn mock_wait_for_event() {
        let mock = MockTransport::new();

        // Spawn event fire after a short delay
        let mock_ref = &mock;
        let wait_handle = tokio::spawn({
            let listeners = mock.listeners.clone();
            async move {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                let listeners = listeners.lock().await;
                if let Some(cbs) = listeners.get("Page.loadEventFired") {
                    for cb in cbs {
                        cb(json!({"timestamp": 999.0}));
                    }
                }
            }
        });

        let result = mock_ref
            .wait_for_event("Page.loadEventFired", 1000)
            .await
            .unwrap();

        assert_eq!(result["timestamp"], 999.0);
        wait_handle.await.unwrap();
    }
}
