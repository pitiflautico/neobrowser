//! Memory accounting, and refusing to continue rather than taking the machine down.
//!
//! A browser driven by an agent grows without an obvious ceiling: every tab, every heap
//! snapshot, every retained DOM. So memory is measured against a configurable cap and the
//! guard refuses further work when it is exceeded — an honest refusal is recoverable, an OOM
//! kill halfway through a task is not.

//! Tier 2: the browser session — a lazily-launched (or attached) Chrome plus one or
//! more CDP tabs sharing it.
//!
//! The Chrome process is owned by the `Browser`, so multiple tabs share a single
//! browser instance. Tools operate on the *active* tab (`tab()`); `new_tab`,
//! `list_tabs`, `switch_tab`, and `close_tab` manage the set. In attach mode
//! (`NEOBROWSER_ATTACH_PORT`) we connect to a Chrome we do not own — no launch, no
//! stealth patching, no kill on shutdown.

use serde_json::json;

use crate::chrome::{self};

use super::Browser;

impl Browser {
    /// Resident memory of the Chrome process tree, in MiB, or `None` when it cannot be
    /// measured on this platform.
    ///
    /// Measured from the OS rather than from Chrome's own metrics: the JS heap is only
    /// part of the cost, and what exhausts a machine is the sum of the renderer
    /// processes.
    pub async fn memory_mb(&self) -> Option<u64> {
        // In attach mode NeoBrowser does not own the process, so there is no tree of
        // ours to measure and no business limiting the user's own browser.
        let pid = { self.state.lock().await.proc.as_ref().and_then(|p| p.pid()) }?;
        #[cfg(unix)]
        {
            // One `ps` over the whole tree: summing per-process RSS is close enough for
            // a guard rail, and shared pages being double-counted errs on the safe side.
            let out = std::process::Command::new("ps")
                .args(["-Ao", "ppid,pid,rss"])
                .output()
                .ok()?;
            let text = String::from_utf8_lossy(&out.stdout);
            let mut total_kb: u64 = 0;
            for line in text.lines().skip(1) {
                let mut f = line.split_whitespace();
                let ppid: u32 = f.next()?.parse().unwrap_or(0);
                let this: u32 = f.next()?.parse().unwrap_or(0);
                let rss: u64 = f.next().unwrap_or("0").parse().unwrap_or(0);
                if this == pid || ppid == pid {
                    total_kb += rss;
                }
            }
            Some(total_kb / 1024)
        }
        #[cfg(not(unix))]
        {
            let _ = pid;
            None
        }
    }

    /// Memory ceiling for the browser tree, from `NEOBROWSER_MAX_MEMORY_MB`.
    ///
    /// Unset by default: guessing a limit would break large legitimate sessions, and an
    /// unattended agent is exactly the case where an operator should choose one.
    pub(super) fn max_memory_mb() -> Option<u64> {
        std::env::var("NEOBROWSER_MAX_MEMORY_MB")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .filter(|m| *m > 0)
    }

    /// Refuse to open more tabs once the configured ceiling is passed.
    ///
    /// Checked at `new_tab` rather than continuously: a background watchdog that killed
    /// the browser mid-action would lose the user's work, whereas declining to grow is
    /// recoverable and says why.
    pub(super) async fn memory_guard(&self) -> Result<(), crate::tools::ToolError> {
        let Some(limit) = Self::max_memory_mb() else {
            return Ok(());
        };
        let Some(used) = self.memory_mb().await else {
            return Ok(());
        };
        if used > limit {
            return Err(crate::tools::ToolError::Failed(format!(
                "the browser tree is using {used} MiB, over the {limit} MiB NEOBROWSER_MAX_MEMORY_MB ceiling. Close tabs with close_tab, or raise the limit. Refusing to open another renderer rather than pushing this host into swap"
            )));
        }
        Ok(())
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
}
