//! Tier 2: the browser session — a lazily-launched (or attached) Chrome plus one or
//! more CDP tabs sharing it.
//!
//! The Chrome process is owned by the `Browser`, so multiple tabs share a single
//! browser instance. Tools operate on the *active* tab (`tab()`); `new_tab`,
//! `list_tabs`, `switch_tab`, and `close_tab` manage the set. In attach mode
//! (`NEOBROWSER_ATTACH_PORT`) we connect to a Chrome we do not own — no launch, no
//! stealth patching, no kill on shutdown.

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::sync::Mutex;

use crate::capture::Capture;
use crate::cdp::CdpClient;
use crate::chrome::{self, ChromeProcess};
use crate::page;
use crate::paths;

/// One open tab: its DevTools target id, CDP client, and captured events.
struct TabHandle {
    id: String,
    client: Arc<CdpClient>,
    capture: Arc<Capture>,
}

/// The shared Chrome plus its tabs.
#[derive(Default)]
struct State {
    proc: Option<ChromeProcess>,
    port: u16,
    attached: bool,
    tabs: Vec<TabHandle>,
    active: usize,
}

/// An in-progress playbook recording.
struct Recording {
    domain: String,
    task: String,
    steps: Vec<serde_json::Value>,
}

/// Attach to an already-running Chrome on this debug port instead of launching one.
fn attach_port() -> Option<u16> {
    std::env::var("NEOBROWSER_ATTACH_PORT")
        .ok()?
        .trim()
        .parse::<u16>()
        .ok()
}

/// The browser handle shared across all tool calls.
pub struct Browser {
    state: Mutex<State>,
    recording: Mutex<Option<Recording>>,
}

impl Browser {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State::default()),
            recording: Mutex::new(None),
        }
    }

    // --- playbook recording ----------------------------------------------------

    pub async fn start_recording(&self, domain: &str, task: &str) {
        *self.recording.lock().await = Some(Recording {
            domain: domain.to_string(),
            task: task.to_string(),
            steps: Vec::new(),
        });
    }

    pub async fn record_step(&self, tool: &str, args: &serde_json::Value) {
        if let Some(rec) = self.recording.lock().await.as_mut() {
            rec.steps
                .push(serde_json::json!({ "tool": tool, "args": args }));
        }
    }

    pub async fn stop_recording(&self) -> usize {
        let Some(rec) = self.recording.lock().await.take() else {
            return 0;
        };
        let n = rec.steps.len();
        let _ = crate::playbook::save(&rec.domain, &rec.task, &rec.steps);
        n
    }

    pub async fn is_recording(&self) -> bool {
        self.recording.lock().await.is_some()
    }

    // --- tab access ------------------------------------------------------------

    /// Get the active tab, launching/attaching Chrome and opening a first tab on
    /// first use. Self-healing: if the whole Chrome died, everything is rebuilt.
    pub async fn tab(&self) -> Result<Arc<CdpClient>, crate::tools::ToolError> {
        let mut st = self.state.lock().await;
        self.ensure(&mut st).await?;
        let idx = st.active.min(st.tabs.len().saturating_sub(1));
        Ok(st.tabs[idx].client.clone())
    }

    /// Ensure `st` has a healthy Chrome with at least one tab.
    async fn ensure(&self, st: &mut State) -> Result<(), crate::tools::ToolError> {
        let healthy = !st.tabs.is_empty()
            && match st.proc.as_mut() {
                Some(p) => p.health_check().await,
                None if st.attached => chrome::port_alive(st.port).await,
                None => false,
            };
        if healthy {
            return Ok(());
        }
        // Rebuild from scratch.
        if let Some(mut p) = st.proc.take() {
            p.kill(true).await;
        }
        st.tabs.clear();
        st.active = 0;

        let (proc, port, attached) = match attach_port() {
            Some(port) => {
                chrome::wait_for_chrome(port, Duration::from_secs(5)).await?;
                (None, port, true)
            }
            None => {
                let proc = ChromeProcess::launch(paths::profiles_base().join("default")).await?;
                let port = proc.port;
                chrome::wait_for_chrome(port, Duration::from_secs(15)).await?;
                (Some(proc), port, false)
            }
        };
        let handle = Self::open_tab(port, !attached, true).await?;
        st.proc = proc;
        st.port = port;
        st.attached = attached;
        st.tabs.push(handle);
        st.active = 0;
        if attached {
            tracing::info!("attached to existing Chrome on port {port}");
        }
        Ok(())
    }

    /// Open one tab on `port`, wire up domains/capture, and (for owned tabs)
    /// install stealth. `inject_cookies` triggers real-profile auto-auth once.
    async fn open_tab(
        port: u16,
        owned: bool,
        inject_cookies: bool,
    ) -> Result<TabHandle, crate::tools::ToolError> {
        let new_tab = chrome::open_new_tab(port).await?;
        let client = CdpClient::connect(&new_tab.web_socket_debugger_url).await?;

        client.send("Page.enable", json!({})).await?;
        client.send("Runtime.enable", json!({})).await?;
        client.send("Network.enable", json!({})).await?;
        let capture = Capture::new();
        capture.spawn_listener(&client);
        // Keep the headless compositor ticking (see page::nudge_frame).
        let _ = client
            .send(
                "Emulation.setFocusEmulationEnabled",
                json!({ "enabled": true }),
            )
            .await;

        if owned {
            // Stealth patch on every new document of tabs we own (never on attached,
            // real Chrome — patching genuine values creates the mismatch anti-bot looks for).
            let _ = client
                .send(
                    "Page.addScriptToEvaluateOnNewDocument",
                    json!({ "source": crate::stealth::STEALTH_JS }),
                )
                .await;

            if inject_cookies && crate::cookies::real_profile_folder().is_some() {
                match crate::cookies::read_real_profile_cookies(None) {
                    Ok(cookies) if !cookies.is_empty() => {
                        let n = cookies.len();
                        if client
                            .send("Network.setCookies", json!({ "cookies": cookies }))
                            .await
                            .is_ok()
                        {
                            tracing::info!("real-session: injected {n} cookies from profile");
                        }
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!("real-session: cookie sync skipped: {e}"),
                }
            }
        }

        Ok(TabHandle {
            id: new_tab.id,
            client,
            capture,
        })
    }

    /// The active tab's capture buffers (ensuring a session exists).
    async fn capture(&self) -> Result<Arc<Capture>, crate::tools::ToolError> {
        let mut st = self.state.lock().await;
        self.ensure(&mut st).await?;
        let idx = st.active.min(st.tabs.len().saturating_sub(1));
        Ok(st.tabs[idx].capture.clone())
    }

    // --- multi-tab management --------------------------------------------------

    /// Open a new tab on the shared Chrome and make it active. Returns its index/id.
    pub async fn new_tab(&self) -> Result<String, crate::tools::ToolError> {
        let mut st = self.state.lock().await;
        self.ensure(&mut st).await?;
        let handle = Self::open_tab(st.port, !st.attached, false).await?;
        let id = handle.id.clone();
        st.tabs.push(handle);
        st.active = st.tabs.len() - 1;
        Ok(json!({ "ok": true, "index": st.active, "id": id, "tabs": st.tabs.len() }).to_string())
    }

    /// List open tabs with their index, url, title, and which is active.
    pub async fn list_tabs(&self) -> Result<String, crate::tools::ToolError> {
        let mut st = self.state.lock().await;
        self.ensure(&mut st).await?;
        let active = st.active;
        let mut out = Vec::new();
        for (i, t) in st.tabs.iter().enumerate() {
            let url = page::current_url(&t.client).await.unwrap_or_default();
            let title = page::js(&t.client, "return document.title")
                .await
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_default();
            out.push(json!({ "index": i, "id": t.id, "url": url, "title": title, "active": i == active }));
        }
        Ok(json!({ "tabs": out, "active": active }).to_string())
    }

    /// Switch the active tab by index and bring it to the front.
    pub async fn switch_tab(&self, index: usize) -> Result<String, crate::tools::ToolError> {
        let mut st = self.state.lock().await;
        self.ensure(&mut st).await?;
        if index >= st.tabs.len() {
            return Err(crate::tools::ToolError::Argument(format!(
                "switch_tab: index {index} out of range (0..{})",
                st.tabs.len()
            )));
        }
        st.active = index;
        let _ = st.tabs[index]
            .client
            .send("Page.bringToFront", json!({}))
            .await;
        Ok(json!({ "ok": true, "active": index, "id": st.tabs[index].id }).to_string())
    }

    /// Close a tab by index (never the last one). Adjusts the active index.
    pub async fn close_tab(&self, index: usize) -> Result<String, crate::tools::ToolError> {
        let mut st = self.state.lock().await;
        self.ensure(&mut st).await?;
        if st.tabs.len() <= 1 {
            return Err(crate::tools::ToolError::Failed(
                "close_tab: cannot close the last remaining tab".into(),
            ));
        }
        if index >= st.tabs.len() {
            return Err(crate::tools::ToolError::Argument(format!(
                "close_tab: index {index} out of range (0..{})",
                st.tabs.len()
            )));
        }
        let handle = st.tabs.remove(index);
        let port = st.port;
        let _ = chrome::close_tab(port, &handle.id).await;
        if st.active >= st.tabs.len() {
            st.active = st.tabs.len() - 1;
        } else if st.active > index {
            st.active -= 1;
        }
        Ok(
            json!({ "ok": true, "closed": index, "active": st.active, "tabs": st.tabs.len() })
                .to_string(),
        )
    }

    // --- diagnostics passthrough (active tab) ----------------------------------

    pub async fn console_logs(
        &self,
        level: Option<&str>,
        limit: usize,
    ) -> Result<String, crate::tools::ToolError> {
        let cap = self.capture().await?;
        let logs = cap.console_logs(level, limit).await;
        Ok(serde_json::to_string(&logs).unwrap_or_else(|_| "[]".into()))
    }

    pub async fn network_log(
        &self,
        pattern: Option<&str>,
        limit: usize,
    ) -> Result<String, crate::tools::ToolError> {
        let cap = self.capture().await?;
        let logs = cap.network_log(pattern, limit).await;
        Ok(serde_json::to_string(&logs).unwrap_or_else(|_| "[]".into()))
    }

    pub async fn metrics(&self, key: Option<&str>) -> Result<String, crate::tools::ToolError> {
        let tab = self.tab().await?;
        let _ = tab.send("Performance.enable", json!({})).await;
        let result = tab.send("Performance.getMetrics", json!({})).await?;
        let mut map = serde_json::Map::new();
        if let Some(metrics) = result.get("metrics").and_then(|m| m.as_array()) {
            for m in metrics {
                if let (Some(name), Some(value)) =
                    (m.get("name").and_then(|v| v.as_str()), m.get("value"))
                {
                    map.insert(name.to_string(), value.clone());
                }
            }
        }
        let out = match key {
            Some(k) => json!({ k: map.get(k).cloned().unwrap_or(serde_json::Value::Null) }),
            None => serde_json::Value::Object(map),
        };
        Ok(out.to_string())
    }

    /// A JSON status snapshot: chrome discovery, session state, tab count.
    pub async fn status(&self) -> serde_json::Value {
        let st = self.state.lock().await;
        let bin = chrome::chrome_bin();
        json!({
            "chrome_bin": bin.display().to_string(),
            "chrome_found": bin.exists(),
            "session_up": !st.tabs.is_empty(),
            "port": if st.port == 0 { serde_json::Value::Null } else { json!(st.port) },
            "attached": st.attached,
            "tabs": st.tabs.len(),
            "active_tab": st.active,
        })
    }

    /// Tear down the session. Only ever kills a Chrome we launched.
    pub async fn shutdown(&self) {
        let mut st = self.state.lock().await;
        st.tabs.clear();
        if let Some(mut proc) = st.proc.take() {
            proc.kill(true).await;
        }
    }
}

impl Default for Browser {
    fn default() -> Self {
        Self::new()
    }
}
