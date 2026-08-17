//! The websocket loop, and what happens when it dies.
//!
//! When the connection drops, every pending request is failed with a reason. This is the
//! mechanism behind a property the whole tool depends on: a dead transport produces errors,
//! never empty successes. A default value returned here would reach a model as a page that
//! evaluated to nothing.

use super::{CdpError, CdpEvent, Outbound, Pending, ProtocolError};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::sync::{broadcast, mpsc};
use tokio_tungstenite::tungstenite::Message;

/// The single owner of the WebSocket. Multiplexes outbound commands and inbound
/// frames, routing responses to `pending` and events to `events_tx`.
pub(super) async fn connection_loop<S>(
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
pub(super) async fn route_message(
    txt: &str,
    pending: &Pending,
    events_tx: &broadcast::Sender<CdpEvent>,
) {
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

pub(super) async fn fail_pending(pending: &Pending, id: u64, err: CdpError) {
    if let Some(pr) = pending.lock().await.remove(&id) {
        let _ = pr.tx.send(Err(err));
    }
}

/// Fail every outstanding request when the connection dies, so callers get a typed
/// `Closed` error immediately instead of waiting out their timeouts.
pub(super) async fn drain_all(pending: &Pending, reason: &str) {
    let mut map = pending.lock().await;
    for (_, pr) in map.drain() {
        let _ = pr.tx.send(Err(CdpError::Closed(reason.to_string())));
    }
}
