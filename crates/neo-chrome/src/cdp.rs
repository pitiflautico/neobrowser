//! Raw CDP client — WebSocket JSON-RPC to Chrome.
//!
//! Sends CDP commands, matches responses by ID, dispatches events.
//! Minimal: no pipe mode, no event listeners — just send/receive.

use crate::{ChromeError, Result};
use futures::{SinkExt, StreamExt};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;

/// Pending command map: request ID -> response channel.
type PendingMap = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value>>>>>;

/// Raw CDP WebSocket client with command/response correlation.
pub struct CdpClient {
    tx: mpsc::UnboundedSender<String>,
    pending: PendingMap,
    next_id: Arc<AtomicU64>,
    alive: Arc<AtomicBool>,
    _recv_handle: JoinHandle<()>,
    _send_handle: JoinHandle<()>,
}

impl CdpClient {
    /// Connect to a CDP WebSocket endpoint.
    pub async fn connect(ws_url: &str) -> Result<Self> {
        let (ws_stream, _) = tokio_tungstenite::connect_async(ws_url)
            .await
            .map_err(|e| ChromeError::ConnectionFailed(e.to_string()))?;

        let (ws_write, ws_read) = ws_stream.split();
        let (tx, rx) = mpsc::unbounded_channel::<String>();
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let alive = Arc::new(AtomicBool::new(true));

        let send_handle = Self::spawn_send_loop(ws_write, rx);
        let recv_handle = Self::spawn_recv_loop(ws_read, pending.clone(), alive.clone());

        Ok(Self {
            tx,
            pending,
            next_id: Arc::new(AtomicU64::new(1)),
            alive,
            _recv_handle: recv_handle,
            _send_handle: send_handle,
        })
    }

    /// Send a CDP command and wait for the response.
    pub async fn send(&self, method: &str, params: Option<Value>) -> Result<Value> {
        self.send_internal(None, method, params).await
    }

    /// Send a CDP command to a specific session (page target).
    pub async fn send_to(
        &self,
        session_id: &str,
        method: &str,
        params: Option<Value>,
    ) -> Result<Value> {
        self.send_internal(Some(session_id), method, params).await
    }

    /// Check if the WebSocket connection is still alive.
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    // ─── Internal ───

    /// Core send logic shared by `send` and `send_to`.
    async fn send_internal(
        &self,
        session_id: Option<&str>,
        method: &str,
        params: Option<Value>,
    ) -> Result<Value> {
        if !self.is_alive() {
            return Err(ChromeError::ConnectionFailed("CDP connection dead".into()));
        }

        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let mut msg = serde_json::json!({ "id": id, "method": method });
        if let Some(p) = params {
            msg["params"] = p;
        }
        if let Some(sid) = session_id {
            msg["sessionId"] = serde_json::json!(sid);
        }

        let (tx, rx) = oneshot::channel();
        {
            self.pending.lock().await.insert(id, tx);
        }

        self.tx
            .send(msg.to_string())
            .map_err(|e| ChromeError::ConnectionFailed(e.to_string()))?;

        let result = tokio::time::timeout(std::time::Duration::from_secs(30), rx)
            .await
            .map_err(|_| ChromeError::Timeout(format!("{method} timed out after 30s")))?
            .map_err(|_| ChromeError::ConnectionFailed("response channel dropped".into()))?;

        result
    }

    /// Spawn the WebSocket send loop.
    fn spawn_send_loop<S>(ws_write: S, mut rx: mpsc::UnboundedReceiver<String>) -> JoinHandle<()>
    where
        S: futures::Sink<tokio_tungstenite::tungstenite::Message> + Unpin + Send + 'static,
        S::Error: std::fmt::Debug,
    {
        tokio::spawn(async move {
            let mut ws_write = ws_write;
            while let Some(msg) = rx.recv().await {
                let ws_msg = tokio_tungstenite::tungstenite::Message::Text(msg.into());
                if ws_write.send(ws_msg).await.is_err() {
                    break;
                }
            }
        })
    }

    /// Spawn the WebSocket receive loop — matches responses, marks dead on close.
    fn spawn_recv_loop<S>(ws_read: S, pending: PendingMap, alive: Arc<AtomicBool>) -> JoinHandle<()>
    where
        S: futures::Stream<
                Item = std::result::Result<
                    tokio_tungstenite::tungstenite::Message,
                    tokio_tungstenite::tungstenite::Error,
                >,
            > + Unpin
            + Send
            + 'static,
    {
        tokio::spawn(async move {
            let mut ws_read = ws_read;
            while let Some(Ok(msg)) = ws_read.next().await {
                let text = match msg {
                    tokio_tungstenite::tungstenite::Message::Text(t) => t.to_string(),
                    _ => continue,
                };
                let parsed: Value = match serde_json::from_str(&text) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                Self::dispatch_response(&pending, &parsed).await;
            }
            // WebSocket closed — fail all pending.
            alive.store(false, Ordering::SeqCst);
            let mut pending = pending.lock().await;
            for (_id, sender) in pending.drain() {
                let err: Result<Value> = Err(ChromeError::ConnectionFailed(
                    "WebSocket disconnected".into(),
                ));
                let _ = sender.send(err);
            }
        })
    }

    /// Match a response to its pending command by ID.
    async fn dispatch_response(pending: &PendingMap, parsed: &Value) {
        let id = match parsed.get("id").and_then(|v| v.as_u64()) {
            Some(id) => id,
            None => return, // Event, not a response — ignore in minimal client.
        };

        let mut pending = pending.lock().await;
        if let Some(sender) = pending.remove(&id) {
            if let Some(error) = parsed.get("error") {
                let msg = error
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("Unknown CDP error");
                let method = parsed
                    .get("method")
                    .and_then(|m| m.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let _ = sender.send(Err(ChromeError::CommandFailed {
                    method,
                    error: msg.to_string(),
                }));
            } else {
                let result = parsed.get("result").cloned().unwrap_or(Value::Null);
                let _ = sender.send(Ok(result));
            }
        }
    }
}
