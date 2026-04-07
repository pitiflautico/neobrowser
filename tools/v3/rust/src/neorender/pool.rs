//! Multi-session pool — manage multiple NeoSession instances by name.
//!
//! Each session has its own cookie jar, localStorage, and V8 runtime.
//! Sessions don't contaminate each other.

use std::collections::HashMap;

/// Pool of named browser sessions.
pub struct NeoSessionPool {
    sessions: HashMap<String, super::session::NeoSession>,
    max_sessions: usize,
}

impl NeoSessionPool {
    /// Create a new pool with a maximum number of concurrent sessions.
    pub fn new(max: usize) -> Self {
        Self {
            sessions: HashMap::new(),
            max_sessions: max,
        }
    }

    /// Get an existing session by name, or create a new one.
    /// Returns error if pool is at capacity and name doesn't exist.
    pub fn get_or_create(
        &mut self,
        name: &str,
        cookies_file: Option<&str>,
    ) -> Result<&mut super::session::NeoSession, String> {
        if !self.sessions.contains_key(name) {
            if self.sessions.len() >= self.max_sessions {
                return Err(format!(
                    "Session pool full ({}/{}). Close a session first.",
                    self.sessions.len(),
                    self.max_sessions
                ));
            }
            let session = super::session::NeoSession::new(cookies_file)?;
            self.sessions.insert(name.to_string(), session);
        }
        Ok(self.sessions.get_mut(name).unwrap())
    }

    /// Get an existing session by name.
    pub fn get(&mut self, name: &str) -> Option<&mut super::session::NeoSession> {
        self.sessions.get_mut(name)
    }

    /// Close and remove a session.
    pub fn close(&mut self, name: &str) {
        self.sessions.remove(name);
    }

    /// List all active session names.
    pub fn list(&self) -> Vec<String> {
        self.sessions.keys().cloned().collect()
    }

    /// Number of active sessions.
    pub fn count(&self) -> usize {
        self.sessions.len()
    }
}
