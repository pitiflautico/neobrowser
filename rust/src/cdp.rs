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

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use thiserror::Error;
use tokio::sync::{broadcast, mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;

/// Default seconds to wait for a CDP response, matching the Python `_SEND_TIMEOUT`.
pub const DEFAULT_SEND_TIMEOUT: Duration = Duration::from_secs(30);

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

impl CdpClient {
    /// Connect to a tab's `webSocketDebuggerUrl`.
    pub async fn connect(ws_url: &str) -> Result<Arc<Self>, CdpError> {
        let (ws_stream, _resp) = tokio_tungstenite::connect_async(ws_url)
            .await
            .map_err(|e| CdpError::WebSocket(e.to_string()))?;

        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<Outbound>();
        let (events_tx, _) = broadcast::channel::<CdpEvent>(1024);
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));

        let client = Arc::new(Self {
            cmd_tx,
            pending: pending.clone(),
            events_tx: events_tx.clone(),
            next_id: AtomicU64::new(1),
            conn_task: Mutex::new(None),
        });

        let handle = tokio::spawn(connection_loop(ws_stream, cmd_rx, pending, events_tx));
        *client.conn_task.lock().await = Some(handle);
        Ok(client)
    }

    /// Subscribe to CDP events. Returns a receiver; lagging receivers drop old events.
    pub fn subscribe(&self) -> broadcast::Receiver<CdpEvent> {
        self.events_tx.subscribe()
    }

    /// Send a CDP command with the default timeout and await its result object.
    pub async fn send(&self, method: &str, params: Value) -> Result<Value, CdpError> {
        self.send_timeout(method, params, DEFAULT_SEND_TIMEOUT)
            .await
    }

    /// Send a CDP command and await its `result` object, bounded by `timeout`.
    pub async fn send_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, CdpError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let payload = json!({ "id": id, "method": method, "params": params });
        let text = serde_json::to_string(&payload).map_err(|e| CdpError::Serde(e.to_string()))?;

        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(
            id,
            PendingRequest {
                method: method.to_string(),
                tx,
            },
        );

        if self.cmd_tx.send(Outbound { id, text }).is_err() {
            self.pending.lock().await.remove(&id);
            return Err(CdpError::Closed("connection task gone".into()));
        }

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(CdpError::Closed("response channel dropped".into())),
            Err(_) => {
                // Timed out — stop tracking so a late reply doesn't leak.
                self.pending.lock().await.remove(&id);
                Err(CdpError::Timeout {
                    method: method.to_string(),
                    timeout,
                })
            }
        }
    }

    /// Convenience: `Runtime.evaluate` returning the deep JSON value of the result.
    pub async fn eval(&self, expression: &str) -> Result<Value, CdpError> {
        let result = self
            .send(
                "Runtime.evaluate",
                json!({
                    "expression": expression,
                    "returnByValue": true,
                    "awaitPromise": false,
                }),
            )
            .await?;
        Ok(result
            .get("result")
            .and_then(|r| r.get("value"))
            .cloned()
            .unwrap_or(Value::Null))
    }

    /// Wait for the first event whose `method` matches `method_name`, bounded by `timeout`.
    pub async fn wait_for_event(
        &self,
        method_name: &str,
        timeout: Duration,
    ) -> Result<CdpEvent, CdpError> {
        let mut rx = self.subscribe();
        let fut = async {
            loop {
                match rx.recv().await {
                    Ok(ev) if ev.method == method_name => return Ok(ev),
                    Ok(_) => continue,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => {
                        return Err(CdpError::Closed("event stream closed".into()))
                    }
                }
            }
        };
        match tokio::time::timeout(timeout, fut).await {
            Ok(res) => res,
            Err(_) => Err(CdpError::Timeout {
                method: method_name.to_string(),
                timeout,
            }),
        }
    }
}

impl Drop for CdpClient {
    fn drop(&mut self) {
        // Abort the connection task if it's still running; the socket closes with it.
        if let Ok(mut guard) = self.conn_task.try_lock() {
            if let Some(handle) = guard.take() {
                handle.abort();
            }
        }
    }
}

/// The single owner of the WebSocket. Multiplexes outbound commands and inbound
/// frames, routing responses to `pending` and events to `events_tx`.
async fn connection_loop<S>(
    mut ws: S,
    mut cmd_rx: mpsc::UnboundedReceiver<Outbound>,
    pending: Pending,
    events_tx: broadcast::Sender<CdpEvent>,
) where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error>
        + Unpin,
{
    let close_reason;
    loop {
        tokio::select! {
            outbound = cmd_rx.recv() => {
                match outbound {
                    Some(out) => {
                        if let Err(e) = ws.send(Message::Text(out.text)).await {
                            // Write failed: fail this request and tear down.
                            fail_pending(&pending, out.id, CdpError::WebSocket(e.to_string())).await;
                            close_reason = format!("write failed: {e}");
                            break;
                        }
                    }
                    None => {
                        // All client handles dropped.
                        close_reason = "all senders dropped".into();
                        break;
                    }
                }
            }
            frame = ws.next() => {
                match frame {
                    Some(Ok(Message::Text(txt))) => {
                        route_message(&txt, &pending, &events_tx).await;
                    }
                    Some(Ok(Message::Binary(bin))) => {
                        if let Ok(txt) = String::from_utf8(bin) {
                            route_message(&txt, &pending, &events_tx).await;
                        }
                    }
                    Some(Ok(Message::Close(_))) => {
                        close_reason = "peer closed".into();
                        break;
                    }
                    Some(Ok(Message::Ping(p))) => {
                        let _ = ws.send(Message::Pong(p)).await;
                    }
                    Some(Ok(_)) => { /* Pong/Frame: ignore */ }
                    Some(Err(e)) => {
                        close_reason = format!("read error: {e}");
                        break;
                    }
                    None => {
                        close_reason = "stream ended".into();
                        break;
                    }
                }
            }
        }
    }
    drain_all(&pending, &close_reason).await;
}

/// Parse and route a single protocol message.
async fn route_message(txt: &str, pending: &Pending, events_tx: &broadcast::Sender<CdpEvent>) {
    let msg: Value = match serde_json::from_str(txt) {
        Ok(v) => v,
        Err(_) => return, // malformed frame: drop it
    };

    if let Some(id) = msg.get("id").and_then(|v| v.as_u64()) {
        let sender = pending.lock().await.remove(&id);
        if let Some(pr) = sender {
            if let Some(err) = msg.get("error") {
                let pe: ProtocolError =
                    serde_json::from_value(err.clone()).unwrap_or(ProtocolError {
                        code: -1,
                        message: err.to_string(),
                    });
                let _ = pr.tx.send(Err(CdpError::Protocol {
                    method: pr.method,
                    code: pe.code,
                    message: pe.message,
                }));
            } else {
                let result = msg.get("result").cloned().unwrap_or(Value::Null);
                let _ = pr.tx.send(Ok(result));
            }
        }
        return;
    }

    if let Some(method) = msg.get("method").and_then(|v| v.as_str()) {
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        // Ignore send errors: no subscribers is fine.
        let _ = events_tx.send(CdpEvent {
            method: method.to_string(),
            params,
        });
    }
}

async fn fail_pending(pending: &Pending, id: u64, err: CdpError) {
    if let Some(pr) = pending.lock().await.remove(&id) {
        let _ = pr.tx.send(Err(err));
    }
}

/// Fail every outstanding request when the connection dies, so callers get a typed
/// `Closed` error immediately instead of waiting out their timeouts.
async fn drain_all(pending: &Pending, reason: &str) {
    let mut map = pending.lock().await;
    for (_, pr) in map.drain() {
        let _ = pr.tx.send(Err(CdpError::Closed(reason.to_string())));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;
    use tokio_tungstenite::accept_async;

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
