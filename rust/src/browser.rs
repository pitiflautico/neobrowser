//! Tier 2: the browser session — a lazily-launched (or attached) Chrome plus one or
//! more CDP tabs sharing it.
//!
//! The Chrome process is owned by the `Browser`, so multiple tabs share a single
//! browser instance. Tools operate on the *active* tab (`tab()`); `new_tab`,
//! `list_tabs`, `switch_tab`, and `close_tab` manage the set. In attach mode
//! (`NEOBROWSER_ATTACH_PORT`) we connect to a Chrome we do not own — no launch, no
//! stealth patching, no kill on shutdown.
//!
//! `Browser` is one type with one job — own the session — so its methods are split across
//! several `impl` blocks rather than several types: [`lifecycle`] launches and guarantees
//! teardown, [`tabs`] manages tabs, [`state`] holds what a tool needs between calls, and
//! [`limits`] refuses work rather than taking the machine down.

use std::sync::Arc;

use tokio::sync::Mutex;

use crate::capture::Capture;
use crate::cdp::CdpClient;
use crate::chrome::ChromeProcess;
use crate::paths;

pub mod lifecycle;
pub mod limits;
pub mod session_info;
pub mod state;
pub mod tabs;

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
///
/// `NEOBROWSER_ATTACH_PORT=<port>` pins the port. `NEOBROWSER_ATTACH_PORT=auto`
/// discovers it: scan the process table for `--remote-debugging-port=N` and
/// probe each candidate's `/json/version` until a real Chrome answers (#13).
fn attach_port() -> Option<u16> {
    let v = std::env::var("NEOBROWSER_ATTACH_PORT").ok()?;
    let v = v.trim();
    if v.eq_ignore_ascii_case("auto") {
        return discover_debug_port();
    }
    v.parse::<u16>().ok()
}

/// Ports mentioned in `--remote-debugging-port=N` flags in a process listing.
fn debug_port_candidates(ps_output: &str) -> Vec<u16> {
    let mut out = Vec::new();
    for chunk in ps_output.split("--remote-debugging-port=").skip(1) {
        let digits: String = chunk.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(port) = digits.parse::<u16>() {
            if port > 0 && !out.contains(&port) {
                out.push(port);
            }
        }
    }
    out
}

/// Probe `http://127.0.0.1:<port>/json/version` with plain std I/O and require
/// a Chrome-ish answer. Any open port is not enough — it must be a CDP endpoint.
fn probe_cdp_port(port: u16) -> bool {
    use std::io::{Read, Write};
    let Ok(mut stream) = std::net::TcpStream::connect(("127.0.0.1", port)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(800)));
    let _ = stream.set_write_timeout(Some(std::time::Duration::from_millis(800)));
    if stream
        .write_all(b"GET /json/version HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n")
        .is_err()
    {
        return false;
    }
    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf).unwrap_or(0);
    let text = String::from_utf8_lossy(&buf[..n]);
    text.contains("\"Browser\"") && text.contains("Chrome")
}

fn discover_debug_port() -> Option<u16> {
    let out = std::process::Command::new("ps")
        .args(["-axo", "command"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    debug_port_candidates(&text)
        .into_iter()
        .find(|&p| probe_cdp_port(p))
}

/// The browser handle shared across all tool calls.
pub struct Browser {
    /// Profile directory override. `None` means the process-wide
    /// `NEOBROWSER_PROFILE`, which is the stdio case. The HTTP transport sets one per
    /// session, because Chrome takes an exclusive lock on a user-data dir — two
    /// sessions sharing one could not both run, and sharing one deliberately would
    /// leak cookies between callers.
    profile_override: Option<std::path::PathBuf>,
    state: Mutex<State>,
    recording: Mutex<Option<Recording>>,
    /// The last accessibility snapshot, so `observe(diff=true)` has a baseline to
    /// compare against. Lives on the browser rather than in the tool because it must
    /// outlive a single call, and is per-session so two clients cannot see each
    /// other's baseline.
    last_snapshot: Mutex<Option<crate::observe::Snapshot>>,
}

impl Browser {
    pub fn new() -> Self {
        Self {
            profile_override: None,
            state: Mutex::new(State::default()),
            recording: Mutex::new(None),
            last_snapshot: Mutex::new(None),
        }
    }

    /// A browser pinned to its own profile directory, for session isolation.
    pub fn with_profile(name: &str) -> Self {
        let mut b = Self::new();
        // Validated through the same whitelist as NEOBROWSER_PROFILE: a session id
        // arrives over the network, and an unvalidated one would point Chrome's
        // user-data dir at an arbitrary path.
        let safe = crate::paths::sanitize_profile_name(name);
        b.profile_override = Some(crate::paths::profiles_base().join(safe));
        b
    }

    fn profile_dir(&self) -> std::path::PathBuf {
        self.profile_override
            .clone()
            .unwrap_or_else(paths::profile_dir)
    }

    // --- snapshot baseline for incremental observation --------------------------
}

impl Default for Browser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_port_candidates_extracts_unique_ports() {
        let ps = "/usr/bin/chrome --remote-debugging-port=9222 --flag\n\
                  helper --type=renderer --remote-debugging-port=9222\n\
                  other --remote-debugging-port=49905 x\n\
                  not-a-port --remote-debugging-port=abc\n";
        assert_eq!(debug_port_candidates(ps), vec![9222, 49905]);
        assert!(debug_port_candidates("nothing here").is_empty());
    }

    #[test]
    fn attach_port_parses_values() {
        let _g = crate::env_test_guard();
        std::env::remove_var("NEOBROWSER_ATTACH_PORT");
        assert_eq!(attach_port(), None);
        std::env::set_var("NEOBROWSER_ATTACH_PORT", "9222");
        assert_eq!(attach_port(), Some(9222));
        std::env::set_var("NEOBROWSER_ATTACH_PORT", "nope");
        assert_eq!(attach_port(), None);
        std::env::remove_var("NEOBROWSER_ATTACH_PORT");
    }
}
