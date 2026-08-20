//! Tier 1: an isolated CDP (Chrome DevTools Protocol) connection to a single tab.
//!
//! Port of the concurrency core in the Python `chrome_tab.py`, redesigned around
//! tokio instead of a background reader thread + per-request queues.
//!
//! Model: one owned "connection task" holds the WebSocket and multiplexes with a
//! `select!` loop:
//!   - outbound commands arrive over an mpsc channel and are written to the socket;
//!   - inbound frames are parsed and routed — a frame carrying an `id` fulfills the
//!     matching request's `oneshot`; a frame carrying a `method` is an event and is
//!     published on a `broadcast` channel.
//!
//! This removes the fragile "who owns recv()" coordination of the threaded version:
//! there is exactly one reader, responses and events never race, and when the socket
//! dies every pending request is drained with a typed `Closed` error instead of
//! hanging until timeout.
//!
//! Split into [`client`] (sending a command and awaiting its reply) and [`transport`] (the
//! websocket loop, and failing every pending request when it dies).

use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;
use tokio::sync::{broadcast, mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;

pub mod client;
pub mod transport;

/// Default seconds to wait for a CDP response, matching the Python `_SEND_TIMEOUT`.
///
/// `NEOBROWSER_SEND_TIMEOUT` overrides this in seconds (e.g. `60` for slow CI
/// runners). It is capped so a typo cannot hang a caller forever.
pub fn default_send_timeout() -> Duration {
    const DEFAULT: Duration = Duration::from_secs(30);
    const MAX: Duration = Duration::from_secs(120);
    std::env::var("NEOBROWSER_SEND_TIMEOUT")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_secs)
        .filter(|d| *d <= MAX)
        .unwrap_or(DEFAULT)
}

#[derive(Debug, Error, Clone)]
pub enum CdpError {
    #[error("CDP command '{method}' timed out after {timeout:?}")]
    Timeout { method: String, timeout: Duration },
    #[error("CDP connection closed: {0}")]
    Closed(String),
    #[error("CDP error for '{method}': [{code}] {message}")]
    Protocol {
        method: String,
        code: i64,
        message: String,
    },
    #[error("websocket error: {0}")]
    WebSocket(String),
    #[error("serialization error: {0}")]
    Serde(String),
}

/// A CDP event (a protocol message with no `id`), e.g. `Page.frameNavigated`.
#[derive(Debug, Clone)]
pub struct CdpEvent {
    pub method: String,
    pub params: Value,
}

/// A request awaiting its response, with the method name kept so protocol
/// errors can say WHICH command failed instead of reporting method ''.
struct PendingRequest {
    method: String,
    tx: oneshot::Sender<Result<Value, CdpError>>,
}

/// A response frame we route back to the caller.
type Pending = Arc<Mutex<HashMap<u64, PendingRequest>>>;

/// An outbound command queued to the connection task.
struct Outbound {
    id: u64,
    text: String,
}

/// An isolated CDP connection to one tab.
pub struct CdpClient {
    cmd_tx: mpsc::UnboundedSender<Outbound>,
    pending: Pending,
    events_tx: broadcast::Sender<CdpEvent>,
    next_id: AtomicU64,
    conn_task: Mutex<Option<JoinHandle<()>>>,
}

#[derive(Deserialize)]
struct ProtocolError {
    code: i64,
    message: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use serde_json::json;
    use tokio::net::TcpListener;
    use tokio_tungstenite::accept_async;
    use tokio_tungstenite::tungstenite::Message;

    /// Spin up an in-process WebSocket server that speaks a tiny CDP-like protocol,
    /// so the client is exercised end-to-end without a real Chrome.
    async fn mock_cdp_server<F>(handler: F) -> String
    where
        F: Fn(Value) -> Vec<Value> + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((stream, _)) = listener.accept().await {
                let mut ws = accept_async(stream).await.unwrap();
                while let Some(Ok(msg)) = ws.next().await {
                    if let Message::Text(txt) = msg {
                        let req: Value = serde_json::from_str(&txt).unwrap();
                        for out in handler(req) {
                            ws.send(Message::Text(out.to_string())).await.unwrap();
                        }
                    }
                }
            }
        });
        format!("ws://{addr}")
    }

    #[tokio::test]
    async fn send_routes_response_by_id() {
        let url = mock_cdp_server(|req| {
            let id = req["id"].as_u64().unwrap();
            vec![json!({ "id": id, "result": { "echo": req["method"] } })]
        })
        .await;

        let client = CdpClient::connect(&url).await.unwrap();
        let res = client.send("Page.enable", json!({})).await.unwrap();
        assert_eq!(res["echo"], "Page.enable");
    }

    #[tokio::test]
    async fn concurrent_sends_match_their_own_ids() {
        // Server replies with the id embedded so we can prove no cross-talk.
        let url = mock_cdp_server(|req| {
            let id = req["id"].as_u64().unwrap();
            vec![json!({ "id": id, "result": { "id_seen": id } })]
        })
        .await;
        let client = CdpClient::connect(&url).await.unwrap();

        let mut handles = vec![];
        for _ in 0..20 {
            let c = client.clone();
            handles.push(tokio::spawn(async move {
                c.send("Runtime.evaluate", json!({})).await.unwrap()
            }));
        }
        let mut seen = std::collections::HashSet::new();
        for h in handles {
            let r = h.await.unwrap();
            let id = r["id_seen"].as_u64().unwrap();
            assert!(seen.insert(id), "duplicate id routed: {id}");
        }
        assert_eq!(seen.len(), 20);
    }

    #[tokio::test]
    async fn protocol_error_maps_to_typed_error() {
        let url = mock_cdp_server(|req| {
            let id = req["id"].as_u64().unwrap();
            vec![json!({ "id": id, "error": { "code": -32000, "message": "boom" } })]
        })
        .await;
        let client = CdpClient::connect(&url).await.unwrap();
        let err = client.send("DOM.getDocument", json!({})).await.unwrap_err();
        match err {
            CdpError::Protocol {
                method,
                code,
                message,
            } => {
                assert_eq!(method, "DOM.getDocument");
                assert_eq!(code, -32000);
                assert_eq!(message, "boom");
            }
            other => panic!("expected Protocol error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn events_are_delivered_to_subscribers() {
        // On any command, the server also emits an unsolicited event.
        let url = mock_cdp_server(|req| {
            let id = req["id"].as_u64().unwrap();
            vec![
                json!({ "method": "Page.frameNavigated", "params": { "url": "https://x.com" } }),
                json!({ "id": id, "result": {} }),
            ]
        })
        .await;
        let client = CdpClient::connect(&url).await.unwrap();
        let ev = tokio::join!(
            client.wait_for_event("Page.frameNavigated", Duration::from_secs(2)),
            client.send("Page.navigate", json!({ "url": "https://x.com" })),
        )
        .0
        .unwrap();
        assert_eq!(ev.params["url"], "https://x.com");
    }

    #[tokio::test]
    async fn timeout_is_typed_and_bounded() {
        // Server never replies.
        let url = mock_cdp_server(|_req| vec![]).await;
        let client = CdpClient::connect(&url).await.unwrap();
        let err = client
            .send_timeout("Never.replies", json!({}), Duration::from_millis(150))
            .await
            .unwrap_err();
        assert!(matches!(err, CdpError::Timeout { .. }), "got {err:?}");
    }

    #[tokio::test]
    async fn closed_connection_drains_pending() {
        // Server accepts then immediately drops the connection without replying.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((stream, _)) = listener.accept().await {
                let mut ws = accept_async(stream).await.unwrap();
                // Read one message, then close.
                let _ = ws.next().await;
                let _ = ws.close(None).await;
            }
        });
        let client = CdpClient::connect(&format!("ws://{addr}")).await.unwrap();
        let err = client
            .send_timeout("Page.enable", json!({}), Duration::from_secs(5))
            .await
            .unwrap_err();
        assert!(matches!(err, CdpError::Closed(_)), "got {err:?}");
    }
}
