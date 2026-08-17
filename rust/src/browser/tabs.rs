//! Opening, listing, switching and closing tabs.
//!
//! Tabs are addressed by index rather than by CDP target id, because an index is what a model
//! can reason about after reading `list_tabs`. The cap on open tabs is deliberate: an agent in
//! a loop opening tabs will exhaust memory long before it notices, and the failure mode of a
//! swapping machine is far worse than a refused open.

use serde_json::json;

use crate::chrome::{self};
use crate::page;

use super::Browser;

impl Browser {
    pub async fn new_tab(&self) -> Result<String, crate::tools::ToolError> {
        let mut st = self.state.lock().await;
        self.ensure(&mut st).await?;
        // Checked before opening, so the limit is a refusal rather than a machine that
        // has already started swapping.
        let cap = Self::max_tabs();
        // Released before the async memory probe so the guard does not hold the state
        // lock across a subprocess call.
        drop(st);
        self.memory_guard().await?;
        let mut st = self.state.lock().await;
        if st.tabs.len() >= cap {
            return Err(crate::tools::ToolError::Failed(format!(
                "tab limit reached ({cap} open). Close a tab with close_tab, or raise \
                 NEOBROWSER_MAX_TABS. Each tab is a renderer process, so this cap is \
                 what keeps a loop calling new_tab from exhausting the host"
            )));
        }
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
            let title = page::eval_body(&t.client, "return document.title")
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
}
