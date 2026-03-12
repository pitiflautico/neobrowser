//! Raw CDP client. WebSocket connection to Chrome.
//!
//! Zero abstractions. Zero frameworks. Just JSON over WebSocket.
//! Each command gets a unique ID, responses are matched by ID,
//! events are dispatched to listeners.

use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;

pub type EventCallback = Arc<dyn Fn(Value) + Send + Sync>;

#[derive(Debug)]
pub struct CdpError(pub String);

impl std::fmt::Display for CdpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CDP: {}", self.0)
    }
}

impl std::error::Error for CdpError {}

/// Raw CDP session — WebSocket + JSON, command/response matching.
pub struct CdpSession {
    tx: mpsc::UnboundedSender<String>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, CdpError>>>>>,
    listeners: Arc<Mutex<HashMap<String, Vec<EventCallback>>>>,
    next_id: AtomicU64,
    _recv_handle: JoinHandle<()>,
    _send_handle: JoinHandle<()>,
}

impl CdpSession {
    /// Connect to a CDP WebSocket endpoint.
    pub async fn connect(ws_url: &str) -> Result<Self, Box<dyn std::error::Error>> {
        use tokio_tungstenite::connect_async;

        let (ws_stream, _) = connect_async(ws_url).await?;
        let (ws_write, ws_read) = futures::StreamExt::split(ws_stream);

        // Channel for outgoing messages
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();

        // Send loop — forwards channel messages to WebSocket
        use futures::SinkExt;
        let send_handle = tokio::spawn(async move {
            let mut ws_write = ws_write;
            while let Some(msg) = rx.recv().await {
                if ws_write
                    .send(tokio_tungstenite::tungstenite::Message::Text(msg.into()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });

        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, CdpError>>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let listeners: Arc<Mutex<HashMap<String, Vec<EventCallback>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        // Recv loop — dispatches responses and events
        let pending_clone = pending.clone();
        let listeners_clone = listeners.clone();
        let recv_handle = tokio::spawn(async move {
            use futures::StreamExt;
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

                // Response to a command
                if let Some(id) = parsed.get("id").and_then(|v| v.as_u64()) {
                    let mut pending = pending_clone.lock().await;
                    if let Some(sender) = pending.remove(&id) {
                        if let Some(error) = parsed.get("error") {
                            let msg = error
                                .get("message")
                                .and_then(|m| m.as_str())
                                .unwrap_or("Unknown CDP error");
                            let _ = sender.send(Err(CdpError(msg.to_string())));
                        } else {
                            let result = parsed.get("result").cloned().unwrap_or(Value::Null);
                            let _ = sender.send(Ok(result));
                        }
                    }
                }
                // Event notification
                else if let Some(method) = parsed.get("method").and_then(|m| m.as_str()) {
                    let params = parsed.get("params").cloned().unwrap_or(Value::Null);
                    let listeners = listeners_clone.lock().await;
                    if let Some(cbs) = listeners.get(method) {
                        for cb in cbs {
                            cb(params.clone());
                        }
                    }
                }
            }
        });

        eprintln!("[CDP] Connected: {}...{}", &ws_url[..20.min(ws_url.len())],
            if ws_url.len() > 40 { &ws_url[ws_url.len()-20..] } else { "" });

        Ok(Self {
            tx,
            pending,
            listeners,
            next_id: AtomicU64::new(1),
            _recv_handle: recv_handle,
            _send_handle: send_handle,
        })
    }

    /// Send a CDP command and wait for the response.
    pub async fn send(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<Value, Box<dyn std::error::Error>> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);

        let mut msg = serde_json::json!({
            "id": id,
            "method": method,
        });
        if let Some(p) = params {
            msg["params"] = p;
        }

        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(id, tx);
        }

        self.tx.send(msg.to_string())?;

        // 30s timeout
        let result = tokio::time::timeout(std::time::Duration::from_secs(30), rx).await??;
        result.map_err(|e| e.into())
    }

    /// Send a CDP command to a specific session (page target).
    /// This adds the "sessionId" field so Chrome routes it to that target.
    pub async fn send_to(
        &self,
        session_id: &str,
        method: &str,
        params: Option<Value>,
    ) -> Result<Value, Box<dyn std::error::Error>> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);

        let mut msg = serde_json::json!({
            "id": id,
            "method": method,
            "sessionId": session_id,
        });
        if let Some(p) = params {
            msg["params"] = p;
        }

        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(id, tx);
        }

        self.tx.send(msg.to_string())?;

        let result = tokio::time::timeout(std::time::Duration::from_secs(30), rx).await??;
        result.map_err(|e| e.into())
    }

    /// Subscribe to a CDP event.
    pub async fn on(&self, event: &str, callback: EventCallback) {
        let mut listeners = self.listeners.lock().await;
        listeners
            .entry(event.to_string())
            .or_default()
            .push(callback);
    }

    /// Wait for a specific CDP event (one-shot). Returns event params.
    pub async fn wait_for(
        &self,
        event: &str,
        timeout_ms: u64,
    ) -> Result<Value, Box<dyn std::error::Error>> {
        let (tx, rx) = oneshot::channel::<Value>();
        let tx = Arc::new(Mutex::new(Some(tx)));

        let tx_clone = tx.clone();
        let callback: EventCallback = Arc::new(move |params| {
            let tx = tx_clone.clone();
            tokio::spawn(async move {
                if let Some(sender) = tx.lock().await.take() {
                    let _ = sender.send(params);
                }
            });
        });

        self.on(event, callback).await;

        let result = tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            rx,
        )
        .await??;

        // Note: listener stays registered but the oneshot is consumed.
        // For a production system, we'd clean it up.
        Ok(result)
    }
}
