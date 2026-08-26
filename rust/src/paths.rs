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

/// Which **Ghost** profile this process uses, from `NEOBROWSER_PROFILE`
/// (default `"default"`).
///
/// Chrome takes an exclusive lock on a user-data dir, so two NeoBrowser
/// processes sharing one profile cannot both run: the second dies on startup
/// with an opaque timeout. Naming the profile per session (or per agent) keeps
/// concurrent sessions from colliding, and keeps their browsing isolated.
///
/// Not to be confused with two neighbours:
/// - `NEOBROWSER_REAL_PROFILE` names the *user's own* Chrome profile, the
///   source real cookies are imported FROM.
/// - `sessions::profile_name()` names on-disk cookie/session snapshots, and is
///   still keyed on `NEOBROWSER_REAL_PROFILE` — so two Ghost profiles sharing
///   one real profile share their snapshot files.
///
/// The name is validated so it can never escape `profiles_base()`: an
/// unvalidated `NEOBROWSER_PROFILE=../../.ssh` would point Chrome's user-data
/// dir at an arbitrary directory. Anything invalid falls back to `"default"`
/// rather than erroring, so a typo degrades instead of breaking the session.
pub fn ghost_profile_name() -> String {
    std::env::var("NEOBROWSER_PROFILE")
        .ok()
        .filter(|n| is_safe_profile_name(n))
        .unwrap_or_else(|| "default".to_string())
}

/// Reduce an arbitrary name to a safe profile directory component.
///
/// Used for HTTP session ids, which arrive over the network. An unvalidated id would
/// point Chrome's `--user-data-dir` at a caller-chosen path; `is_safe_profile_name`
/// already encodes what is acceptable, so this maps anything else onto a safe form
/// rather than inventing a second rule.
pub fn sanitize_profile_name(name: &str) -> String {
    if is_safe_profile_name(name) {
        return name.to_string();
    }
    let mapped: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
        .take(64)
        .collect();
    // Must start alphanumeric and be non-empty, per `is_safe_profile_name`.
    let mapped = mapped.trim_start_matches(['_', '-']).to_string();
    if mapped.is_empty() {
        "session".to_string()
    } else {
        format!("s{mapped}")
    }
}

/// Alphanumeric start, then alphanumerics/space/underscore/hyphen, max 64.
/// Rejects anything with a path separator, `..`, or a leading dot.
fn is_safe_profile_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '_' | '-'))
}

/// The Ghost Chrome user-data dir for this process's profile.
pub fn profile_dir() -> PathBuf {
    profiles_base().join(ghost_profile_name())
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

/// Handoff file describing the currently running Chrome session.
///
/// Written when Chrome launches, deleted when it shuts down. Lets external
/// tools discover the debug port without parsing `ps` output.
pub fn session_file() -> PathBuf {
    home().join("session.json")
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

    /// The profile name lands in a filesystem path, so a traversal attempt must
    /// never reach it — an unvalidated `../../.ssh` would point Chrome's
    /// user-data dir at an arbitrary directory. Invalid names degrade to
    /// "default" rather than erroring, so a typo doesn't break the session.
    #[test]
    fn ghost_profile_name_rejects_traversal_and_falls_back() {
        let _g = crate::env_test_guard();
        let prev = std::env::var_os("NEOBROWSER_PROFILE");

        std::env::remove_var("NEOBROWSER_PROFILE");
        assert_eq!(ghost_profile_name(), "default", "unset");

        for good in ["alpha", "Session 2", "agent-7", "a_b-c 9"] {
            std::env::set_var("NEOBROWSER_PROFILE", good);
            assert_eq!(ghost_profile_name(), good, "{good:?} should be accepted");
        }
        for bad in [
            "../evil",
            "..",
            "/etc/passwd",
            ".ssh",
            "a/b",
            "",
            "-leading-dash",
            "x..y/../z",
        ] {
            std::env::set_var("NEOBROWSER_PROFILE", bad);
            assert_eq!(ghost_profile_name(), "default", "{bad:?} must be rejected");
        }

        // And the resulting dir always stays under profiles_base().
        std::env::set_var("NEOBROWSER_PROFILE", "../evil");
        assert!(profile_dir().starts_with(profiles_base()));

        match prev {
            Some(v) => std::env::set_var("NEOBROWSER_PROFILE", v),
            None => std::env::remove_var("NEOBROWSER_PROFILE"),
        }
    }

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
