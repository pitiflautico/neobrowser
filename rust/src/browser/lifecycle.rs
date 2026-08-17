//! Launching Chrome on demand, attaching, and guaranteeing it goes away.
//!
//! `tab()` is the entry point every tool goes through, and it is lazy on purpose: nothing
//! launches a browser until something actually needs one. `ensure` is where self-healing
//! lives — if the process died since the last call, the next call relaunches rather than
//! handing back a client whose socket is gone, because a dead client does not error
//! immediately, it errors halfway through someone's task.

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

use crate::capture::Capture;
use crate::cdp::CdpClient;
use crate::chrome::{self, ChromeProcess};

use super::{attach_port, Browser, State, TabHandle};

impl Browser {
    /// Get the active tab, launching/attaching Chrome and opening a first tab on
    /// first use. Self-healing: if the whole Chrome died, everything is rebuilt.
    pub async fn tab(&self) -> Result<Arc<CdpClient>, crate::tools::ToolError> {
        let mut st = self.state.lock().await;
        self.ensure(&mut st).await?;
        let idx = st.active.min(st.tabs.len().saturating_sub(1));
        Ok(st.tabs[idx].client.clone())
    }

    /// Ensure `st` has a healthy Chrome with at least one tab.
    pub(super) async fn ensure(&self, st: &mut State) -> Result<(), crate::tools::ToolError> {
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
                let proc = ChromeProcess::launch(self.profile_dir()).await?;
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
    pub(super) async fn open_tab(
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
                    json!({ "source": crate::stealth::stealth_js() }),
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
    pub(super) async fn capture(&self) -> Result<Arc<Capture>, crate::tools::ToolError> {
        let mut st = self.state.lock().await;
        self.ensure(&mut st).await?;
        let idx = st.active.min(st.tabs.len().saturating_sub(1));
        Ok(st.tabs[idx].capture.clone())
    }

    // --- multi-tab management --------------------------------------------------

    /// Open a new tab on the shared Chrome and make it active. Returns its index/id.
    /// Maximum concurrent tabs, from `NEOBROWSER_MAX_TABS`.
    ///
    /// A limit has to exist: each tab is a renderer process with its own memory, and an
    /// agent in a loop calling `new_tab` will exhaust the machine long before it
    /// notices. Refusing with a clear message beats a host that starts swapping.
    pub(super) fn max_tabs() -> usize {
        std::env::var("NEOBROWSER_MAX_TABS")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .filter(|n| *n > 0)
            .unwrap_or(20)
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

impl Browser {
    /// Replace the active tab with a fresh one showing the same URL, and return its client.
    ///
    /// The recovery for a Chrome state that has no other cure: a tab whose input pipeline has
    /// stopped delivering events (see [`crate::page::input_is_alive`]). Reload, re-navigation,
    /// re-attaching and every `Emulation`/`Input` reset leave it dead; a different tab works
    /// immediately.
    ///
    /// Cookies and storage live in the profile rather than the tab, so a session survives this
    /// — an authenticated page comes back authenticated. What does NOT survive is anything the
    /// old document held only in memory: unsaved form input, scroll position, JS state. So this
    /// is a last resort, and the caller must report that it happened rather than pretend the
    /// action simply worked.
    pub async fn replace_active_tab(
        &self,
    ) -> Result<(Arc<CdpClient>, String), crate::tools::ToolError> {
        let url = {
            let st = self.state.lock().await;
            let idx = st.active.min(st.tabs.len().saturating_sub(1));
            match st.tabs.get(idx) {
                Some(t) => crate::page::current_url(&t.client)
                    .await
                    .unwrap_or_default(),
                None => String::new(),
            }
        };

        let mut st = self.state.lock().await;
        self.ensure(&mut st).await?;
        let port = st.port;
        // `attached` means we are driving the user's real Chrome, where we must not inject
        // the stealth patch or their cookies a second time.
        let owned = !st.attached;
        let fresh = Self::open_tab(port, owned, false).await?;
        let idx = st.active.min(st.tabs.len().saturating_sub(1));
        if let Some(old) = st.tabs.get(idx) {
            let (p, id) = (port, old.id.clone());
            // Best-effort: a tab we cannot close is a leak, not a failure of the recovery.
            tokio::spawn(async move {
                let _ = crate::chrome::close_tab(p, &id).await;
            });
        }
        let client = fresh.client.clone();
        if st.tabs.is_empty() {
            st.tabs.push(fresh);
            st.active = 0;
        } else {
            st.tabs[idx] = fresh;
        }
        drop(st);

        if !url.is_empty() && url != "about:blank" {
            let budget = crate::action::Budget::from_secs(20.0);
            let _ = crate::page::navigate_budgeted(&client, &url, &budget).await;
        }
        Ok((client, url))
    }
}
