//! The client surface: sending a command and awaiting its reply.
//!
//! `Drop` is not incidental. A dropped client with pending requests must fail them rather
//! than leave their futures hanging forever, because a caller awaiting a reply that can never
//! arrive does not error — it stops, silently, and takes the task with it.

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

use super::transport::connection_loop;
use super::{
    CdpClient, CdpError, CdpEvent, Outbound, Pending, PendingRequest, DEFAULT_SEND_TIMEOUT,
};
use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc, oneshot, Mutex};

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
