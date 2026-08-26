//! Handoff file for the currently running Chrome session.
//!
//! Written when Chrome launches, deleted when it shuts down. Lets external tools
//! discover the debug port without parsing `ps` output.

use serde::{Deserialize, Serialize};

use crate::paths;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub pid: u32,
    pub cdp_port: u16,
    pub profile: String,
    pub started_at: String,
}

/// Write the handoff file for the current session.
pub fn write(pid: u32, cdp_port: u16, profile: &str) {
    let info = SessionInfo {
        pid,
        cdp_port,
        profile: profile.to_string(),
        started_at: chrono::Utc::now().to_rfc3339(),
    };
    let path = paths::session_file();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(&info) {
        let _ = std::fs::write(&path, json);
    }
}

/// Remove the handoff file, if it exists.
pub fn clear() {
    let _ = std::fs::remove_file(paths::session_file());
}

/// Read the current handoff file, if any.
pub fn read() -> Option<SessionInfo> {
    let data = std::fs::read_to_string(paths::session_file()).ok()?;
    serde_json::from_str(&data).ok()
}

/// Read the handoff file and verify the Chrome it points to is actually alive.
///
/// A stale file is ignored: the PID must exist and the CDP port must answer.
pub async fn live_session() -> Option<SessionInfo> {
    let info = read()?;
    // Verify the process is still running.
    #[cfg(unix)]
    {
        let alive = unsafe { libc::kill(info.pid as i32, 0) } == 0;
        if !alive {
            return None;
        }
    }
    #[cfg(windows)]
    {
        // On Windows we skip the PID check and rely on the port probe.
    }
    // Verify the CDP port answers.
    if !crate::chrome::port_alive(info.cdp_port).await {
        return None;
    }
    Some(info)
}
