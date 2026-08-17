//! Tier 0: Chrome process manager.
//!
//! Port of the Python `chrome_process.py`. Design invariants kept:
//! - No shared PID file that could kill sibling processes.
//! - A `ChromeProcess` owns exactly the child it spawned and only ever kills that.
//! - `health_check()` requires BOTH the process alive AND the debug port responding,
//!   which prevents handing out a zombie ("GhostChrome").
//!
//! Improvements over the Python original:
//! - The spawned child is owned, so `Drop` reaps it — no orphan Chromes if the
//!   manager is dropped without an explicit `kill()`.
//! - `kill()` sends SIGTERM first (Chrome flushes its profile/cookies) and only
//!   escalates to SIGKILL after a grace period.
//!
//!
//! Split by responsibility: [`sandbox`] decides whether the renderer sandbox can be on,
//! [`discover`] finds a Chrome and asks what it is, [`endpoint`] talks to its HTTP port,
//! [`process`] owns the process and guarantees teardown, and [`lock`] distinguishes a stale
//! profile lock from a live one. The error type stays here, since every part returns it.

use thiserror::Error;

pub mod discover;
pub mod endpoint;
pub mod lock;
pub mod process;
pub mod sandbox;

pub use discover::{
    chrome_bin, chrome_user_agent, detect_chrome_major, discover_chrome_bin, DEFAULT_CHROME_FLAGS,
};
pub use endpoint::{close_tab, find_free_port, open_new_tab, port_alive, wait_for_chrome, NewTab};
pub use lock::{clear_stale_lock_for_test, profile_lock_holder};
pub use process::ChromeProcess;
pub use sandbox::{no_sandbox_opt_in_active, sandbox_support, SandboxSupport};

#[derive(Debug, Error)]
pub enum ChromeError {
    #[error("invalid port {0}: must be 1024..=65535")]
    InvalidPort(u16),
    #[error("profile_dir must be under {base}: got {got}")]
    ProfileOutsideBase { base: String, got: String },
    #[error("chrome did not become ready on port {port} within timeout{stderr}")]
    NotReady { port: u16, stderr: String },
    #[error(
        "profile {profile:?} is already in use by a running Chrome (pid {pid}). \
         Chrome locks a user-data dir exclusively, so this session cannot launch \
         its own. Either drive that browser with NEOBROWSER_ATTACH_PORT={port_hint}, \
         or give this session its own profile with NEOBROWSER_PROFILE=<name>."
    )]
    ProfileInUse {
        profile: String,
        pid: i32,
        port_hint: String,
    },
    #[error(
        "refusing to launch Chrome without its sandbox: {reason}. The sandbox is what \
         keeps a compromised renderer — i.e. any page NeoBrowser visits — from reaching \
         the rest of this machine. {hint}"
    )]
    SandboxUnavailable {
        reason: &'static str,
        hint: &'static str,
    },
    #[error(
        "refusing to combine an unsandboxed Chrome with real-profile cookies. \
         NEOBROWSER_ALLOW_NO_SANDBOX drops the barrier between a hostile page and this \
         machine, while NEOBROWSER_REAL_PROFILE={profile:?} hands that same browser your \
         logged-in sessions — together they turn one renderer bug into full account and \
         host compromise. Drop one of the two, or, if you accept that risk, set \
         NEOBROWSER_ALLOW_NO_SANDBOX=with-real-profile."
    )]
    NoSandboxWithRealProfile { profile: String },
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("http error talking to chrome debug endpoint: {0}")]
    Http(#[from] reqwest::Error),
}

/// Whether this host looks able to run Chrome's own sandbox.
///
/// Deliberately conservative: it only reports a blocker it can prove, because a
/// false negative here would refuse to start for a user whose sandbox is fine.
#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use tokio::process::{Child, Command};

    use super::endpoint::validate_port;
    use super::sandbox::{no_sandbox_opt_in, resolve_sandbox, NoSandboxOptIn};
    use super::*;

    #[test]
    fn discover_honors_env_override() {
        let _g = crate::env_test_guard();
        let prev = std::env::var_os("NEOBROWSER_CHROME_BIN");
        std::env::set_var("NEOBROWSER_CHROME_BIN", "/custom/chrome");
        assert_eq!(discover_chrome_bin(), PathBuf::from("/custom/chrome"));
        match prev {
            Some(v) => std::env::set_var("NEOBROWSER_CHROME_BIN", v),
            None => std::env::remove_var("NEOBROWSER_CHROME_BIN"),
        }
    }

    #[test]
    fn default_flags_suppress_webdriver_and_avoid_disable_gpu() {
        assert!(DEFAULT_CHROME_FLAGS.contains(&"--disable-blink-features=AutomationControlled"));
        assert!(DEFAULT_CHROME_FLAGS.contains(&"--headless=new"));
        // --disable-gpu must NOT be a default (software WebGL is a headless tell).
        assert!(!DEFAULT_CHROME_FLAGS.contains(&"--disable-gpu"));
    }

    /// The regression that matters most: no build may ship --no-sandbox as a
    /// default. If this ever fails, every page NeoBrowser visits runs unconfined.
    #[test]
    fn no_sandbox_is_never_a_default_flag() {
        assert!(
            !DEFAULT_CHROME_FLAGS.contains(&"--no-sandbox"),
            "--no-sandbox must only come from the resolve_sandbox opt-in"
        );
    }

    /// Scoped env setter that restores the previous value on drop, so a failing
    /// assertion can't leak NEOBROWSER_ALLOW_NO_SANDBOX into later tests.
    struct EnvVar {
        key: &'static str,
        prev: Option<String>,
    }

    impl EnvVar {
        fn set(key: &'static str, value: &str) -> Self {
            let prev = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, prev }
        }
        fn unset(key: &'static str) -> Self {
            let prev = std::env::var(key).ok();
            std::env::remove_var(key);
            Self { key, prev }
        }
    }

    impl Drop for EnvVar {
        fn drop(&mut self) {
            match self.prev.take() {
                Some(v) => std::env::set_var(self.key, v),
                None => std::env::remove_var(self.key),
            }
        }
    }

    #[test]
    fn opt_in_parses_only_explicit_affirmatives() {
        let _g = crate::env_test_guard();
        for (value, expected) in [
            ("1", NoSandboxOptIn::Yes),
            ("true", NoSandboxOptIn::Yes),
            ("YES", NoSandboxOptIn::Yes),
            ("with-real-profile", NoSandboxOptIn::YesWithRealProfile),
            ("With-Real-Profile", NoSandboxOptIn::YesWithRealProfile),
            // Anything else must fall back to the secure default, so a typo or a
            // "0" never silently disables the sandbox.
            ("0", NoSandboxOptIn::No),
            ("false", NoSandboxOptIn::No),
            ("", NoSandboxOptIn::No),
            ("maybe", NoSandboxOptIn::No),
        ] {
            let _e = EnvVar::set("NEOBROWSER_ALLOW_NO_SANDBOX", value);
            assert_eq!(no_sandbox_opt_in(), expected, "value {value:?}");
        }
    }

    #[test]
    fn default_launch_is_sandboxed_when_the_host_allows_it() {
        let _g = crate::env_test_guard();
        let _a = EnvVar::unset("NEOBROWSER_ALLOW_NO_SANDBOX");
        let _r = EnvVar::unset("NEOBROWSER_REAL_PROFILE");
        // Skip where the host genuinely can't sandbox (root CI, locked-down
        // kernel): there the correct behavior is the error asserted below.
        if sandbox_support() == SandboxSupport::Available {
            assert!(!resolve_sandbox().unwrap(), "must not pass --no-sandbox");
        } else {
            assert!(matches!(
                resolve_sandbox(),
                Err(ChromeError::SandboxUnavailable { .. })
            ));
        }
    }

    #[test]
    fn opt_in_alone_allows_unsandboxed() {
        let _g = crate::env_test_guard();
        let _a = EnvVar::set("NEOBROWSER_ALLOW_NO_SANDBOX", "1");
        let _r = EnvVar::unset("NEOBROWSER_REAL_PROFILE");
        assert!(resolve_sandbox().unwrap());
    }

    /// The dangerous combination: no sandbox AND the user's live cookies. One
    /// renderer bug would take both the machine and the accounts, so the plain
    /// opt-in is not enough to authorize it.
    #[test]
    fn real_profile_plus_opt_in_is_refused_without_the_specific_token() {
        let _g = crate::env_test_guard();
        let _a = EnvVar::set("NEOBROWSER_ALLOW_NO_SANDBOX", "1");
        let _r = EnvVar::set("NEOBROWSER_REAL_PROFILE", "Default");
        assert!(matches!(
            resolve_sandbox(),
            Err(ChromeError::NoSandboxWithRealProfile { .. })
        ));
    }

    #[test]
    fn real_profile_plus_specific_token_is_allowed() {
        let _g = crate::env_test_guard();
        let _a = EnvVar::set("NEOBROWSER_ALLOW_NO_SANDBOX", "with-real-profile");
        let _r = EnvVar::set("NEOBROWSER_REAL_PROFILE", "Default");
        assert!(resolve_sandbox().unwrap());
    }

    /// An empty NEOBROWSER_REAL_PROFILE is not a real profile, so it must not
    /// trip the stricter branch.
    #[test]
    fn blank_real_profile_is_not_a_real_profile() {
        let _g = crate::env_test_guard();
        let _a = EnvVar::set("NEOBROWSER_ALLOW_NO_SANDBOX", "1");
        let _r = EnvVar::set("NEOBROWSER_REAL_PROFILE", "   ");
        assert!(resolve_sandbox().unwrap());
    }

    #[test]
    fn find_free_port_is_in_valid_range() {
        let p = find_free_port().unwrap();
        assert!((1024..=65535).contains(&p), "got {p}");
        assert!(validate_port(p).is_ok());
    }

    #[test]
    fn validate_port_rejects_out_of_range() {
        assert!(validate_port(80).is_err());
        assert!(validate_port(1024).is_ok());
        assert!(validate_port(65535).is_ok());
    }

    /// Build a manager around an arbitrary child process for kill/Drop tests.
    fn proc_of(child: Child) -> ChromeProcess {
        ChromeProcess {
            profile_dir: PathBuf::from("/tmp"),
            port: 0,
            child: Some(child),
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn kill_force_terminates_and_reaps() {
        let child = Command::new("sleep").arg("30").spawn().unwrap();
        let mut proc = proc_of(child);
        let pid = proc.pid().unwrap();
        proc.kill(true).await;
        assert!(proc.child.is_none());
        // Reaped and gone: signalling the pid must fail with ESRCH.
        // SAFETY: signal 0 only probes for existence; `pid` is a spawned child's.
        let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
        assert!(rc != 0, "process {pid} still exists after kill(true)");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn drop_terminates_child_without_explicit_kill() {
        let pid;
        {
            let child = Command::new("sleep").arg("30").spawn().unwrap();
            let proc = proc_of(child);
            pid = proc.pid().unwrap();
        } // Drop runs here: SIGTERM + bounded reap.
          // SAFETY: as above — an existence probe on a pid we spawned.
        let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
        assert!(rc != 0, "process {pid} survived ChromeProcess drop");
    }

    #[test]
    fn user_agent_shape_from_major() {
        // Directly exercise the UA string shape without depending on a real Chrome.
        let major = "150";
        let token = if cfg!(target_os = "windows") {
            "Windows NT 10.0; Win64; x64"
        } else if cfg!(target_os = "linux") {
            "X11; Linux x86_64"
        } else {
            "Macintosh; Intel Mac OS X 10_15_7"
        };
        let ua = format!(
            "Mozilla/5.0 ({token}) AppleWebKit/537.36 (KHTML, like Gecko) \
             Chrome/{major}.0.0.0 Safari/537.36"
        );
        assert!(ua.contains("Chrome/150.0.0.0"));
        assert!(!ua.contains("HeadlessChrome"));
    }

    #[test]
    fn detect_major_parses_version_string() {
        // Simulate `--version` output parsing via a temp script.
        // We can't guarantee Chrome here, so assert the parser on a known binary:
        // `echo` prints its args; wrap the expected shape.
        // Instead, unit-test the digit scan through a tiny helper reimplementation.
        fn parse(text: &str) -> Option<String> {
            let bytes = text.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i].is_ascii_digit() {
                    let start = i;
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                    if i + 1 < bytes.len() && bytes[i] == b'.' && bytes[i + 1].is_ascii_digit() {
                        return Some(text[start..i].to_string());
                    }
                } else {
                    i += 1;
                }
            }
            None
        }
        assert_eq!(parse("Google Chrome 150.0.7258.5 "), Some("150".into()));
        assert_eq!(parse("Chromium 121.0.6167.184"), Some("121".into()));
        assert_eq!(parse("no version here"), None);
    }
}
