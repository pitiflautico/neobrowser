//! Session-level capture of CDP console + network events.
//!
//! The Python `ChromeTab` accumulated console messages and network requests inside
//! its reader thread. Here a background task subscribes to the tab's broadcast event
//! stream and fills bounded ring buffers, so `console_logs` / `network_log` can read
//! recent activity. Bounded so a long-lived tab can't grow memory without limit.

use std::collections::VecDeque;
use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::cdp::CdpClient;

const MAX_CONSOLE: usize = 500;
const MAX_NETWORK: usize = 500;

#[derive(Debug, Clone, Serialize)]
pub struct ConsoleEntry {
    pub level: String,
    pub text: String,
    pub timestamp: f64,
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct NetworkEntry {
    pub request_id: String,
    pub url: String,
    pub method: String,
    pub status: Option<i64>,
    pub status_text: String,
    pub duration_ms: Option<f64>,
    pub encoded_data_length: Option<f64>,
    pub timestamp: f64,
}

/// Bounded buffers of recent console + network activity for one tab.
#[derive(Default)]
pub struct Capture {
    console: Mutex<VecDeque<ConsoleEntry>>,
    network: Mutex<VecDeque<NetworkEntry>>,
}

impl Capture {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Spawn a background task that drains the tab's event stream into the buffers.
    /// Ends when the tab (and its broadcast sender) is dropped.
    pub fn spawn_listener(self: &Arc<Self>, client: &CdpClient) {
        let mut rx = client.subscribe();
        let this = self.clone();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(ev) => this.handle(&ev.method, &ev.params).await,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break, // channel closed → tab gone
                }
            }
        });
    }

    async fn handle(&self, method: &str, params: &Value) {
        match method {
            "Runtime.consoleAPICalled" => {
                let level = params.get("type").and_then(|v| v.as_str()).unwrap_or("log");
                let mut parts = Vec::new();
                if let Some(args) = params.get("args").and_then(|a| a.as_array()) {
                    for arg in args {
                        if let Some(s) = arg.get("value").and_then(|v| v.as_str()) {
                            parts.push(s.to_string());
                        } else if let Some(d) = arg.get("description").and_then(|v| v.as_str()) {
                            parts.push(d.to_string());
                        } else if let Some(v) = arg.get("value") {
                            parts.push(v.to_string());
                        }
                    }
                }
                let source = params
                    .get("stackTrace")
                    .and_then(|s| s.get("callFrames"))
                    .and_then(|f| f.as_array())
                    .and_then(|f| f.first())
                    .and_then(|f| f.get("url"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                self.push_console(ConsoleEntry {
                    level: level.to_string(),
                    text: parts.join(" "),
                    timestamp: params
                        .get("timestamp")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0),
                    source,
                })
                .await;
            }
            "Runtime.exceptionThrown" => {
                let detail = params.get("exceptionDetails");
                let text = detail
                    .and_then(|d| d.get("exception"))
                    .and_then(|e| e.get("description"))
                    .and_then(|v| v.as_str())
                    .or_else(|| detail.and_then(|d| d.get("text")).and_then(|v| v.as_str()))
                    .unwrap_or("")
                    .to_string();
                self.push_console(ConsoleEntry {
                    level: "error".into(),
                    text,
                    timestamp: params
                        .get("timestamp")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0),
                    source: "exception".into(),
                })
                .await;
            }
            "Network.requestWillBeSent" => {
                let req_id = params
                    .get("requestId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if req_id.is_empty() {
                    return;
                }
                let request = params.get("request");
                let entry = NetworkEntry {
                    request_id: req_id.to_string(),
                    url: request
                        .and_then(|r| r.get("url"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    method: request
                        .and_then(|r| r.get("method"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("GET")
                        .to_string(),
                    status: None,
                    status_text: String::new(),
                    duration_ms: None,
                    encoded_data_length: None,
                    timestamp: params
                        .get("timestamp")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0),
                };
                let mut net = self.network.lock().await;
                if net.len() >= MAX_NETWORK {
                    net.pop_front();
                }
                net.push_back(entry);
            }
            "Network.responseReceived" => {
                let req_id = params
                    .get("requestId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let response = params.get("response");
                let resp_ts = params
                    .get("timestamp")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                let mut net = self.network.lock().await;
                if let Some(entry) = net.iter_mut().rev().find(|e| e.request_id == req_id) {
                    entry.status = response
                        .and_then(|r| r.get("status"))
                        .and_then(|v| v.as_i64());
                    entry.status_text = response
                        .and_then(|r| r.get("statusText"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    entry.encoded_data_length = response
                        .and_then(|r| r.get("encodedDataLength"))
                        .and_then(|v| v.as_f64());
                    if resp_ts > 0.0 && entry.timestamp > 0.0 {
                        entry.duration_ms = Some((resp_ts - entry.timestamp) * 1000.0);
                    }
                }
            }
            _ => {}
        }
    }

    async fn push_console(&self, entry: ConsoleEntry) {
        let mut c = self.console.lock().await;
        if c.len() >= MAX_CONSOLE {
            c.pop_front();
        }
        c.push_back(entry);
    }

    /// Recent console entries, optionally filtered by level, last `limit`.
    pub async fn console_logs(&self, level: Option<&str>, limit: usize) -> Vec<ConsoleEntry> {
        let c = self.console.lock().await;
        let filtered: Vec<ConsoleEntry> = c
            .iter()
            .filter(|e| level.is_none_or(|l| e.level == l))
            .cloned()
            .collect();
        let start = filtered.len().saturating_sub(limit);
        filtered[start..].to_vec()
    }

    /// Recent network entries, optionally filtered by URL substring, last `limit`.
    pub async fn network_log(&self, pattern: Option<&str>, limit: usize) -> Vec<NetworkEntry> {
        let n = self.network.lock().await;
        let filtered: Vec<NetworkEntry> = n
            .iter()
            .filter(|e| pattern.is_none_or(|p| e.url.contains(p)))
            .cloned()
            .collect();
        let start = filtered.len().saturating_sub(limit);
        filtered[start..].to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn console_capture_and_level_filter() {
        let cap = Capture::new();
        cap.handle(
            "Runtime.consoleAPICalled",
            &json!({ "type": "error", "args": [{"type":"string","value":"boom"}], "timestamp": 1.0 }),
        )
        .await;
        cap.handle(
            "Runtime.consoleAPICalled",
            &json!({ "type": "log", "args": [{"type":"string","value":"hi"}], "timestamp": 2.0 }),
        )
        .await;
        let all = cap.console_logs(None, 50).await;
        assert_eq!(all.len(), 2);
        let errs = cap.console_logs(Some("error"), 50).await;
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].text, "boom");
    }

    #[tokio::test]
    async fn network_request_then_response_merges() {
        let cap = Capture::new();
        cap.handle(
            "Network.requestWillBeSent",
            &json!({ "requestId": "1", "request": {"url":"https://x.com/a","method":"GET"}, "timestamp": 1.0 }),
        )
        .await;
        cap.handle(
            "Network.responseReceived",
            &json!({ "requestId": "1", "response": {"status":200,"statusText":"OK","encodedDataLength":123.0}, "timestamp": 1.5 }),
        )
        .await;
        let logs = cap.network_log(None, 50).await;
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].status, Some(200));
        assert_eq!(logs[0].duration_ms, Some(500.0));
    }

    #[tokio::test]
    async fn network_url_filter() {
        let cap = Capture::new();
        for (id, url) in [("1", "https://a.com/x"), ("2", "https://b.com/y")] {
            cap.handle(
                "Network.requestWillBeSent",
                &json!({ "requestId": id, "request": {"url": url, "method":"GET"}, "timestamp": 1.0 }),
            )
            .await;
        }
        let only_b = cap.network_log(Some("b.com"), 50).await;
        assert_eq!(only_b.len(), 1);
        assert_eq!(only_b[0].url, "https://b.com/y");
    }
}
