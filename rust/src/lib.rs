//! NeoBrowser — a fast, stealthy MCP browser-automation server that drives real
//! Chrome. Library crate exposing the modules the binary and integration tests use.

pub mod action;
pub mod bridge;
pub mod browser;
pub mod capture;
pub mod cdp;
pub mod chrome;
pub mod config;
pub mod cookies;
pub mod devtools;
pub mod frames;
pub mod http_transport;
pub mod js;
pub mod llm;
pub mod mcp;
pub mod observe;
pub mod ops;
pub mod page;
pub mod paths;
pub mod playbook;
pub mod policy;
pub mod reach;
pub mod search;
pub mod sessions;
pub mod stealth;
pub mod tool_impls;
pub mod tools;
pub mod trace;
pub mod untrusted;
pub mod vault;
pub mod walls;

/// Serializes tests that mutate process-global environment variables, which would
/// otherwise race under cargo's parallel test threads. Poison-tolerant: a panic in
/// one env test must not wedge the others.
#[cfg(test)]
pub(crate) fn env_test_guard() -> std::sync::MutexGuard<'static, ()> {
    use std::sync::Mutex;
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}
