//! Multi-context browser pool.
//!
//! Manages multiple isolated browser sessions for parallel automation.
//! Each context has its own profile directory, cookies, and state.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextInfo {
    pub id: String,
    pub profile_dir: String,
    pub status: String,  // "idle", "busy", "dead"
    pub current_url: String,
    pub created_at: String,
    pub last_used: String,
}

pub struct BrowserPool {
    contexts: HashMap<String, ContextInfo>,
    max_contexts: usize,
    base_dir: std::path::PathBuf,
}

impl BrowserPool {
    pub fn new(max_contexts: usize) -> Self {
        let base_dir = dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(".neobrowser")
            .join("pool");
        std::fs::create_dir_all(&base_dir).ok();

        Self {
            contexts: HashMap::new(),
            max_contexts,
            base_dir,
        }
    }

    pub fn create_context(&mut self, id: Option<String>) -> Result<String, String> {
        if self.contexts.len() >= self.max_contexts {
            return Err(format!("Pool full ({}/{})", self.contexts.len(), self.max_contexts));
        }
        let ctx_id = id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()[..8].to_string());
        let profile_dir = self.base_dir.join(&ctx_id);
        std::fs::create_dir_all(&profile_dir).map_err(|e| format!("{e}"))?;

        let now = chrono::Utc::now().to_rfc3339();
        self.contexts.insert(ctx_id.clone(), ContextInfo {
            id: ctx_id.clone(),
            profile_dir: profile_dir.to_string_lossy().into(),
            status: "idle".into(),
            current_url: String::new(),
            created_at: now.clone(),
            last_used: now,
        });

        Ok(ctx_id)
    }

    pub fn list(&self) -> Vec<&ContextInfo> {
        self.contexts.values().collect()
    }

    pub fn get(&self, id: &str) -> Option<&ContextInfo> {
        self.contexts.get(id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut ContextInfo> {
        self.contexts.get_mut(id)
    }

    pub fn set_status(&mut self, id: &str, status: &str, url: &str) {
        if let Some(ctx) = self.contexts.get_mut(id) {
            ctx.status = status.into();
            ctx.current_url = url.into();
            ctx.last_used = chrono::Utc::now().to_rfc3339();
        }
    }

    pub fn destroy(&mut self, id: &str) -> Result<(), String> {
        if let Some(ctx) = self.contexts.remove(id) {
            // Clean up profile directory
            let _ = std::fs::remove_dir_all(&ctx.profile_dir);
            Ok(())
        } else {
            Err(format!("Context not found: {id}"))
        }
    }

    pub fn destroy_all(&mut self) {
        let ids: Vec<String> = self.contexts.keys().cloned().collect();
        for id in ids {
            let _ = self.destroy(&id);
        }
    }
}
