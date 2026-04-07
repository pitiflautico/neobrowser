//! CDP — Chrome DevTools Protocol typed layer.
//!
//! Architecture:
//! - `CdpTransport` trait: abstraction over WebSocket/pipe/mock
//! - `transport.rs`: real WebSocket/pipe implementation (was cdp.rs)
//! - `mock.rs`: MockTransport for unit testing
//! - One module per CDP domain (page.rs, runtime.rs, dom.rs, etc.)

mod transport;
mod mock;
pub mod page;
pub mod runtime;
pub mod dom;
pub mod input;
pub mod network;
pub mod target;
pub mod emulation;
pub mod browser_domain;
pub mod fetch;
pub mod css;
pub mod dom_storage;
pub mod accessibility;
pub mod log_domain;
pub mod performance;
pub mod security;
pub mod indexed_db;
pub mod service_worker;

// Re-export transport types (backward compat: crate::cdp::CdpSession still works)
pub use transport::{CdpSession, CdpError, EventCallback};
pub use mock::MockTransport;
// ScopedTransport is defined in this file and publicly available

use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Result type for CDP operations.
pub type CdpResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Callback for CDP events.
pub type TransportEventCallback = Arc<dyn Fn(Value) + Send + Sync>;

/// Abstraction over CDP connections (WebSocket, pipe, or mock).
///
/// All CDP domain modules use this trait, enabling unit tests
/// with MockTransport instead of a real Chrome.
pub trait CdpTransport: Send + Sync {
    /// Send a CDP command and wait for the response.
    fn send(
        &self,
        method: &str,
        params: Value,
    ) -> Pin<Box<dyn Future<Output = CdpResult<Value>> + Send + '_>>;

    /// Send a CDP command to a specific session (page target).
    fn send_to(
        &self,
        session_id: &str,
        method: &str,
        params: Value,
    ) -> Pin<Box<dyn Future<Output = CdpResult<Value>> + Send + '_>>;

    /// Subscribe to a CDP event.
    fn on_event(
        &self,
        event: &str,
        callback: TransportEventCallback,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;

    /// Wait for a specific CDP event (one-shot).
    fn wait_for_event(
        &self,
        event: &str,
        timeout_ms: u64,
    ) -> Pin<Box<dyn Future<Output = CdpResult<Value>> + Send + '_>>;

    /// Check if connection is alive.
    fn is_alive(&self) -> bool;
}

/// A transport scoped to a specific CDP session (page target).
///
/// When Chrome attaches to a target, all commands must include the sessionId.
/// ScopedTransport wraps a real transport + sessionId, so typed domain modules
/// can use `transport.send()` and it automatically routes via `send_to()`.
pub struct ScopedTransport<'a> {
    inner: &'a CdpSession,
    session_id: String,
}

impl<'a> ScopedTransport<'a> {
    pub fn new(transport: &'a CdpSession, session_id: &str) -> Self {
        Self {
            inner: transport,
            session_id: session_id.to_string(),
        }
    }
}

impl CdpTransport for ScopedTransport<'_> {
    fn send(
        &self,
        method: &str,
        params: Value,
    ) -> Pin<Box<dyn Future<Output = CdpResult<Value>> + Send + '_>> {
        let method = method.to_string();
        let session_id = self.session_id.clone();
        Box::pin(async move {
            let params_opt = if params.is_null() { None } else { Some(params) };
            self.inner.send_to_raw(&session_id, &method, params_opt)
                .await
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                    Box::new(CdpError(e.to_string()))
                })
        })
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
            let params_opt = if params.is_null() { None } else { Some(params) };
            self.inner.send_to_raw(&session_id, &method, params_opt)
                .await
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                    Box::new(CdpError(e.to_string()))
                })
        })
    }

    fn on_event(
        &self,
        event: &str,
        callback: TransportEventCallback,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        let event = event.to_string();
        Box::pin(async move {
            self.inner.on_raw(&event, callback).await;
        })
    }

    fn wait_for_event(
        &self,
        event: &str,
        timeout_ms: u64,
    ) -> Pin<Box<dyn Future<Output = CdpResult<Value>> + Send + '_>> {
        let event = event.to_string();
        Box::pin(async move {
            self.inner.wait_for_raw(&event, timeout_ms)
                .await
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                    Box::new(CdpError(e.to_string()))
                })
        })
    }

    fn is_alive(&self) -> bool {
        self.inner.is_alive()
    }
}

// Implement CdpTransport for CdpSession (real connection)
impl CdpTransport for CdpSession {
    fn send(
        &self,
        method: &str,
        params: Value,
    ) -> Pin<Box<dyn Future<Output = CdpResult<Value>> + Send + '_>> {
        let method = method.to_string();
        Box::pin(async move {
            let params_opt = if params.is_null() { None } else { Some(params) };
            self.send_raw(&method, params_opt)
                .await
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                    Box::new(CdpError(e.to_string()))
                })
        })
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
            let params_opt = if params.is_null() { None } else { Some(params) };
            self.send_to_raw(&session_id, &method, params_opt)
                .await
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                    Box::new(CdpError(e.to_string()))
                })
        })
    }

    fn on_event(
        &self,
        event: &str,
        callback: TransportEventCallback,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        let event = event.to_string();
        Box::pin(async move {
            self.on_raw(&event, callback).await;
        })
    }

    fn wait_for_event(
        &self,
        event: &str,
        timeout_ms: u64,
    ) -> Pin<Box<dyn Future<Output = CdpResult<Value>> + Send + '_>> {
        let event = event.to_string();
        Box::pin(async move {
            self.wait_for_raw(&event, timeout_ms)
                .await
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                    Box::new(CdpError(e.to_string()))
                })
        })
    }

    fn is_alive(&self) -> bool {
        self.is_alive()
    }
}
