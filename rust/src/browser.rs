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
fn attach_port() -> Option<u16> {
    std::env::var("NEOBROWSER_ATTACH_PORT")
        .ok()?
        .trim()
        .parse::<u16>()
        .ok()
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
