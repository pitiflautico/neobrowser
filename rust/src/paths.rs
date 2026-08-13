//! Central runtime-data locations for NeoBrowser.
//!
//! Everything lives under `NEOBROWSER_HOME` (default `~/.neobrowser`), overridable
//! via the `NEOBROWSER_HOME` environment variable. Keeping every path in one place
//! means the on-disk layout — profiles, cookies, sessions, playbooks — is
//! defined exactly once. Mirrors the Python `neobrowser/paths.py`.

use std::path::PathBuf;

/// Resolve the NeoBrowser home directory, honoring `NEOBROWSER_HOME`.
pub fn home() -> PathBuf {
    match std::env::var_os("NEOBROWSER_HOME") {
        Some(v) if !v.is_empty() => expand_tilde(PathBuf::from(v)),
        _ => dirs_home().join(".neobrowser"),
    }
}

/// Ghost Chrome user-data dirs.
pub fn profiles_base() -> PathBuf {
    home().join("profiles")
}

/// Per-profile JSON cookie snapshots.
pub fn cookies_base() -> PathBuf {
    home().join("cookies")
}

/// Full session caches (cookies + storage).
pub fn sessions_base() -> PathBuf {
    home().join("sessions")
}

/// Recorded action playbooks.
pub fn playbooks_base() -> PathBuf {
    home().join("playbooks")
}

/// Chrome stderr logs, one per debug port. Chrome explains startup failures
/// (profile lock held, port in use, missing sandbox) on stderr; keeping it lets
/// the "did not become ready" error carry the real reason instead of guessing.
pub fn logs_base() -> PathBuf {
    home().join("logs")
}

/// Path of the stderr log for the Chrome instance on `port`.
pub fn chrome_log(port: u16) -> PathBuf {
    logs_base().join(format!("chrome-{port}.log"))
}

/// The user's real Chrome profile root (macOS layout).
#[cfg(target_os = "macos")]
pub fn real_chrome_profile() -> PathBuf {
    dirs_home()
        .join("Library")
        .join("Application Support")
        .join("Google")
        .join("Chrome")
}

#[cfg(target_os = "linux")]
pub fn real_chrome_profile() -> PathBuf {
    dirs_home().join(".config").join("google-chrome")
}

#[cfg(target_os = "windows")]
pub fn real_chrome_profile() -> PathBuf {
    // %LOCALAPPDATA%\Google\Chrome\User Data
    let local = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs_home().join("AppData").join("Local"));
    local.join("Google").join("Chrome").join("User Data")
}

/// Best-effort home directory without pulling in the `dirs` crate.
fn dirs_home() -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(p) = std::env::var_os("USERPROFILE") {
            return PathBuf::from(p);
        }
    }
    if let Some(p) = std::env::var_os("HOME") {
        return PathBuf::from(p);
    }
    PathBuf::from(".")
}

/// Expand a leading `~` to the home directory.
fn expand_tilde(p: PathBuf) -> PathBuf {
    if let Ok(s) = p.strip_prefix("~") {
        return dirs_home().join(s);
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_honors_env() {
        let _g = crate::env_test_guard();
        // Isolated because env is process-global: use a unique value and restore.
        let prev = std::env::var_os("NEOBROWSER_HOME");
        std::env::set_var("NEOBROWSER_HOME", "/tmp/nb-test-home");
        assert_eq!(home(), PathBuf::from("/tmp/nb-test-home"));
        assert_eq!(profiles_base(), PathBuf::from("/tmp/nb-test-home/profiles"));
        assert_eq!(cookies_base(), PathBuf::from("/tmp/nb-test-home/cookies"));
        match prev {
            Some(v) => std::env::set_var("NEOBROWSER_HOME", v),
            None => std::env::remove_var("NEOBROWSER_HOME"),
        }
    }

    #[test]
    fn default_home_under_dot_neobrowser() {
        let _g = crate::env_test_guard();
        let prev = std::env::var_os("NEOBROWSER_HOME");
        std::env::remove_var("NEOBROWSER_HOME");
        assert!(home().ends_with(".neobrowser"));
        if let Some(v) = prev {
            std::env::set_var("NEOBROWSER_HOME", v);
        }
    }
}
