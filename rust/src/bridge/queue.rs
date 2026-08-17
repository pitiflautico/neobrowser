//! The command queue itself: what the extension picks up, and what it hands back.
//!
//! Command ids are random rather than sequential. A sequential id is guessable, and a guessable
//! id on a loopback endpoint means a page that cannot read a response can still fabricate one —
//! answering a command it never received and feeding the caller invented data.

//! The agent half of the Chrome bridge: a localhost queue the extension polls.
//!
//! See `extension/README.md` for the security model. In short: instead of copying the
//! user's cookies into a second browser, or opening `--remote-debugging-port` on their
//! everyday Chrome and exposing every tab, the user shares one tab at a time from a
//! popup and can revoke it.
//!
//! The transport is a poll rather than a socket, and that is forced by the platform: a
//! Manifest V3 service worker is terminated when idle, so a long-lived WebSocket from
//! the extension would be killed with it. The extension asks for work; this side hands
//! out queued CDP commands and collects the results.
//!
//! Bound to `127.0.0.1` and **authenticated with a per-session token**.
//!
//! Loopback alone is not a boundary here, and it is worth being precise about why. Any
//! web page the user visits can issue a request to `http://127.0.0.1:<port>` — a
//! `text/plain` POST is a "simple" request, so there is no preflight, and while the
//! browser blocks the page from *reading* the response, the side effect still happens.
//! Without authentication a hostile page could therefore:
//!
//! * POST a forged result to `/bridge/results` and poison an in-flight CDP command,
//!   feeding the agent a fabricated answer; or
//! * POST to `/bridge` and drain the queue, so the real extension never receives the
//!   commands and every action times out.
//!
//! Two things close that. Every request must carry `X-NeoBrowser-Token`, which is a
//! *custom* header — that alone makes the request non-simple, so a page cannot send it
//! without a preflight this server refuses. And command ids are random rather than
//! sequential, so a forged result cannot address an outstanding waiter even if the
//! token were known.
//!
//! The token lives in `~/.neobrowser/bridge.token` at 0600, so a same-uid process can
//! read it — the same trust the MCP stdio transport already implies — while nothing
//! else can. Per-tab consent is still enforced in the extension, where Chrome shows
//! its own "being debugged" banner.
//!
//! Split so the listener sits apart from the queue it serves: [`server`] accepts
//! connections and authenticates them, while the queue, its pending map and the token file
//! stay here.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use super::{token_path, COMMAND_TIMEOUT_SECS};
use serde_json::{json, Value};
use tokio::sync::{oneshot, Mutex};

/// One queued CDP command.
#[derive(Debug, Clone)]
struct Pending {
    id: u64,
    tab_id: i64,
    method: String,
    params: Value,
}

/// Shared bridge state.
pub struct Bridge {
    port: u16,
    inner: Mutex<Inner>,
    /// Retained only so a stale sequential-id assumption cannot creep back in; ids
    /// themselves are random. Kept as a monotonic counter mixed into the id so two
    /// commands minted in the same instant cannot collide.
    next_id: AtomicU64,
    token: String,
}

#[derive(Default)]
struct Inner {
    /// Commands waiting to be handed to the extension.
    queue: Vec<Pending>,
    /// Where to deliver each result once it arrives.
    waiting: HashMap<u64, oneshot::Sender<Result<Value, String>>>,
    /// Tab ids the user has shared, as last reported by the extension. Advisory on
    /// this side — the extension enforces it — but needed so the agent can list what
    /// it is actually allowed to drive.
    shared_tabs: Vec<i64>,
    /// Whether the extension has polled at least once.
    connected: bool,
}

impl Bridge {
    pub fn new(port: u16) -> Arc<Self> {
        Arc::new(Self {
            port,
            inner: Mutex::new(Inner::default()),
            next_id: AtomicU64::new(1),
            token: crate::vault::random_token_hex(),
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    /// Persist the token where the user (and only the user) can read it, so
    /// `neobrowser bridge token` can print it for pasting into the extension popup.
    pub fn write_token_file(&self) -> std::io::Result<std::path::PathBuf> {
        let path = token_path();
        crate::sessions::write_private(&path, &self.token)?;
        Ok(path)
    }

    /// Constant-time token comparison.
    ///
    /// Over loopback with a 256-bit token a timing oracle is not a realistic attack,
    /// but a variable-time compare on a secret is the kind of thing that becomes one
    /// after a refactor, and the constant-time version costs nothing.
    pub(super) fn token_matches(&self, presented: Option<&str>) -> bool {
        let Some(presented) = presented else {
            return false;
        };
        let expected = self.token.as_bytes();
        let got = presented.trim().as_bytes();
        if expected.len() != got.len() {
            return false;
        }
        let mut diff = 0u8;
        for (a, b) in expected.iter().zip(got.iter()) {
            diff |= a ^ b;
        }
        diff == 0
    }

    /// Has the extension ever polled? Reported by `doctor` and `bridge_status`, so
    /// "nothing happens" is distinguishable from "the extension is not installed".
    pub async fn is_connected(&self) -> bool {
        self.inner.lock().await.connected
    }

    pub async fn shared_tabs(&self) -> Vec<i64> {
        self.inner.lock().await.shared_tabs.clone()
    }

    /// Send a CDP command to a shared tab and await its result.
    pub async fn send(&self, tab_id: i64, method: &str, params: Value) -> Result<Value, String> {
        // Random, not sequential: a guessable id lets a forged result be addressed to
        // an outstanding command. The counter is mixed in so two ids minted in the same
        // instant cannot collide.
        let id = {
            let seq = self.next_id.fetch_add(1, Ordering::Relaxed);
            let rand =
                u64::from_str_radix(&crate::vault::random_token_hex()[..15], 16).unwrap_or(seq);
            rand.wrapping_mul(1 << 8).wrapping_add(seq & 0xFF)
        };
        let (tx, rx) = oneshot::channel();
        {
            let mut inner = self.inner.lock().await;
            if !inner.connected {
                return Err(
                    "the NeoBrowser Bridge extension has not connected. Load it from \
                     extension/ in chrome://extensions, then share a tab from its popup"
                        .into(),
                );
            }
            inner.queue.push(Pending {
                id,
                tab_id,
                method: method.to_string(),
                params,
            });
            inner.waiting.insert(id, tx);
        }

        match tokio::time::timeout(std::time::Duration::from_secs(COMMAND_TIMEOUT_SECS), rx).await {
            Ok(Ok(result)) => result,
            // The sender was dropped: the bridge was torn down mid-flight.
            Ok(Err(_)) => Err("bridge closed while the command was in flight".into()),
            Err(_) => {
                // Timed out. Remove the waiter so a late result cannot be delivered to
                // a caller that has already given up.
                let mut inner = self.inner.lock().await;
                inner.waiting.remove(&id);
                inner.queue.retain(|c| c.id != id);
                Err(format!(
                    "the extension did not answer within {COMMAND_TIMEOUT_SECS}s. Is the \
                     tab still shared? Chrome drops the attachment when a tab closes"
                ))
            }
        }
    }

    /// Handle the extension's poll: record what it shares, hand back queued work.
    pub(super) async fn take_work(&self, shared: Vec<i64>) -> Value {
        let mut inner = self.inner.lock().await;
        inner.connected = true;
        inner.shared_tabs = shared;
        let commands: Vec<Value> = inner
            .queue
            .drain(..)
            .map(|c| {
                json!({ "id": c.id, "tabId": c.tab_id, "method": c.method, "params": c.params })
            })
            .collect();
        json!({ "commands": commands })
    }

    /// Deliver results back to whoever is waiting for them.
    pub(super) async fn deliver(&self, results: &[Value]) -> usize {
        let mut inner = self.inner.lock().await;
        let mut delivered = 0;
        for r in results {
            let Some(id) = r.get("id").and_then(Value::as_u64) else {
                continue;
            };
            let Some(tx) = inner.waiting.remove(&id) else {
                // Nobody is waiting: the caller already timed out. Dropping it is
                // correct, and counting it would overstate what got through.
                continue;
            };
            let payload = match r.get("error").and_then(Value::as_str) {
                Some(e) => Err(e.to_string()),
                None => Ok(r.get("result").cloned().unwrap_or(Value::Null)),
            };
            if tx.send(payload).is_ok() {
                delivered += 1;
            }
        }
        delivered
    }
}
