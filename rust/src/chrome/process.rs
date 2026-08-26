//! Owning the Chrome process: spawn it, and make sure it dies.
//!
//! A browser automation tool that leaks processes is a tool that fills a machine with
//! orphaned Chromes, each holding a profile lock that breaks the next run. So teardown is
//! not best-effort: `Drop` terminates and reaps with a timeout, escalating from SIGTERM to
//! SIGKILL, because a graceful shutdown that hangs is worse than an abrupt one.

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

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tokio::process::{Child, Command};

use super::discover::{chrome_bin, chrome_user_agent, DEFAULT_CHROME_FLAGS};
use super::endpoint::{find_free_port, validate_port};
use super::lock::{clear_stale_lock, devtools_active_port, lock_holder_pid, term};
use super::sandbox::resolve_sandbox;
use super::ChromeError;
use crate::paths;

/// Manages a single Chrome process. Owns exactly one child; `kill`/`Drop` only
/// ever touch that child.
#[derive(Debug)]
pub struct ChromeProcess {
    pub profile_dir: PathBuf,
    pub port: u16,
    pub(super) child: Option<Child>,
}

impl ChromeProcess {
    /// Launch headless Chrome on a free port. `profile_dir` must be under the
    /// profiles base (`~/.neobrowser/profiles/`).
    pub async fn launch(profile_dir: impl AsRef<Path>) -> Result<Self, ChromeError> {
        let profile_dir = profile_dir.as_ref().to_path_buf();
        let base = paths::profiles_base();
        // Create both dirs before canonicalizing so symlinked prefixes (e.g. macOS
        // /tmp -> /private/tmp) resolve consistently for base and profile alike.
        std::fs::create_dir_all(&base)?;
        std::fs::create_dir_all(&profile_dir)?;
        let canon_base = base.canonicalize().unwrap_or(base.clone());
        let canon_profile = profile_dir.canonicalize().unwrap_or(profile_dir.clone());
        if !canon_profile.starts_with(&canon_base) {
            return Err(ChromeError::ProfileOutsideBase {
                base: canon_base.display().to_string(),
                got: canon_profile.display().to_string(),
            });
        }

        // A Chrome that died without cleaning up leaves SingletonLock behind.
        // Chrome then refuses to use the profile and exits immediately, so the
        // debug port never opens and every launch fails with an opaque timeout.
        clear_stale_lock(&profile_dir);

        // If the lock survived, a live Chrome owns this profile. Launching anyway
        // would just fail on Chrome's ProcessSingleton with a timeout that says
        // nothing about the cause — so fail here with the two ways out instead.
        if let Some(pid) = lock_holder_pid(&profile_dir) {
            return Err(ChromeError::ProfileInUse {
                profile: profile_dir
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| profile_dir.display().to_string()),
                pid,
                port_hint: devtools_active_port(&profile_dir)
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "<its debug port>".into()),
            });
        }

        // Resolved before spawning: an unsandboxed launch must be an explicit,
        // logged decision, never a silent default.
        let unsandboxed = resolve_sandbox()?;

        let port = find_free_port()?;
        let mut cmd = Command::new(chrome_bin());
        cmd.arg(format!("--remote-debugging-port={port}"))
            .arg(format!("--user-data-dir={}", profile_dir.display()));
        for flag in DEFAULT_CHROME_FLAGS {
            cmd.arg(flag);
        }
        if unsandboxed {
            cmd.arg("--no-sandbox");
        }
        if let Some(ua) = chrome_user_agent() {
            cmd.arg(format!("--user-agent={ua}"));
        }
        if let Some(proxy) = std::env::var_os("NEOBROWSER_PROXY") {
            if !proxy.is_empty() {
                cmd.arg(format!("--proxy-server={}", proxy.to_string_lossy()));
            }
        }
        if std::env::var_os("NEOBROWSER_DISABLE_GPU").is_some() {
            cmd.arg("--disable-gpu");
        }
        // Keep stderr: Chrome writes the reason for a failed start there, and
        // discarding it is what turns a lock/port/sandbox problem into a bare
        // "did not become ready" with nothing to go on.
        cmd.stdout(std::process::Stdio::null())
            .stderr(chrome_stderr_sink(port))
            .kill_on_drop(false); // we reap explicitly in Drop with a graceful term

        let child = cmd.spawn()?;
        Ok(Self {
            profile_dir,
            port,
            child: Some(child),
        })
    }

    /// The OS process id of the spawned child, if still owned.
    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().and_then(|c| c.id())
    }

    /// Is the process still running? (No signal sent; just a non-blocking wait.)
    pub fn is_alive(&mut self) -> bool {
        match self.child.as_mut() {
            Some(c) => matches!(c.try_wait(), Ok(None)),
            None => false,
        }
    }

    /// Does Chrome's HTTP debug endpoint respond?
    pub async fn port_alive(&self) -> bool {
        if validate_port(self.port).is_err() {
            return false;
        }
        let url = format!("http://127.0.0.1:{}/json/version", self.port);
        reqwest::Client::new()
            .get(&url)
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    /// True only if BOTH the process is alive AND the port responds. Prevents
    /// handing out a zombie GhostChrome.
    pub async fn health_check(&mut self) -> bool {
        self.is_alive() && self.port_alive().await
    }

    /// Detach from the process without killing it. The child handle is dropped,
    /// so `Drop` will not terminate Chrome — it keeps running for the next
    /// process to attach to. Used by persistent mode.
    pub fn detach(&mut self) {
        self.child = None;
    }

    /// Terminate the process. Always sends SIGTERM first (Chrome flushes its
    /// profile) and waits up to 3s for a graceful exit; only if `force` and it
    /// is still alive after the grace period does it escalate to SIGKILL.
    /// An exited child is always reaped (no zombies).
    pub async fn kill(&mut self, force: bool) {
        let Some(pid) = self.pid() else { return };
        term(pid);
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if !self.is_alive() {
                // try_wait already reaped it.
                self.child = None;
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        if force {
            if let Some(mut c) = self.child.take() {
                let _ = c.start_kill(); // SIGKILL
                let _ = c.wait().await; // reap: no zombie
            }
        }
        // !force: graceful mode — Chrome got SIGTERM and exits on its own
        // schedule; we deliberately do not SIGKILL it.
    }
}

impl Drop for ChromeProcess {
    fn drop(&mut self) {
        // Terminate AND reap so we never leak or zombie a headless Chrome, even
        // when the manager is dropped without an explicit kill().
        let Some(mut child) = self.child.take() else {
            return;
        };
        // Graceful first: SIGTERM, then a bounded poll (~1s) for exit.
        if let Some(pid) = child.id() {
            term(pid);
        }
        if reap_with_timeout(&mut child, Duration::from_secs(1)) {
            return;
        }
        // Still alive: SIGKILL and reap again (bounded).
        let _ = child.start_kill();
        let _ = reap_with_timeout(&mut child, Duration::from_secs(1));
    }
}

/// Poll `try_wait` (which reaps on exit) until the child exits or `timeout`
/// elapses. Returns true if the child was confirmed exited and reaped.
/// Blocking and sync — only used from `Drop`, where await is impossible.
fn reap_with_timeout(child: &mut Child, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => return true,
            Ok(None) => {
                if Instant::now() >= deadline {
                    return false;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

/// Open (truncating) the stderr log for `port`, falling back to /dev/null if the
/// log directory is unusable — logging must never block a launch.
fn chrome_stderr_sink(port: u16) -> std::process::Stdio {
    if std::fs::create_dir_all(paths::logs_base()).is_err() {
        return std::process::Stdio::null();
    }
    match std::fs::File::create(paths::chrome_log(port)) {
        Ok(f) => std::process::Stdio::from(f),
        Err(_) => std::process::Stdio::null(),
    }
}
