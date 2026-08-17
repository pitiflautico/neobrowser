//! The per-session state a tool needs between calls: snapshots, recordings, and logs.
//!
//! The stored snapshot is what makes stable references work across calls — `observe` records
//! it, and the next action resolves `role:name#nth` against it rather than against a node id
//! that a re-render has already invalidated.

//! Tier 2: the browser session — a lazily-launched (or attached) Chrome plus one or
//! more CDP tabs sharing it.
//!
//! The Chrome process is owned by the `Browser`, so multiple tabs share a single
//! browser instance. Tools operate on the *active* tab (`tab()`); `new_tab`,
//! `list_tabs`, `switch_tab`, and `close_tab` manage the set. In attach mode
//! (`NEOBROWSER_ATTACH_PORT`) we connect to a Chrome we do not own — no launch, no
//! stealth patching, no kill on shutdown.

use super::{Browser, Recording};

impl Browser {
    /// Read the stored baseline without consuming it.
    pub async fn take_snapshot(&self) -> Option<crate::observe::Snapshot> {
        self.last_snapshot.lock().await.clone()
    }

    pub async fn store_snapshot(&self, snap: crate::observe::Snapshot) {
        *self.last_snapshot.lock().await = Some(snap);
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

    /// The captured network entries as typed values, for the HAR exporter.
    ///
    /// `network_log` serialises to a JSON string for the tool output; HAR needs the
    /// structs, and re-parsing our own JSON to get them back would be silly.
    pub async fn network_entries(
        &self,
        pattern: Option<&str>,
        limit: usize,
    ) -> Vec<crate::capture::NetworkEntry> {
        match self.capture().await {
            Ok(cap) => cap.network_log(pattern, limit).await,
            // No live capture yet means no traffic recorded, which is an empty HAR
            // rather than an error.
            Err(_) => Vec::new(),
        }
    }
}
