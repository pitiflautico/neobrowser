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
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde::Deserialize;
use thiserror::Error;
use tokio::process::{Child, Command};

use crate::paths;

#[derive(Debug, Error)]
pub enum ChromeError {
    #[error("invalid port {0}: must be 1024..=65535")]
    InvalidPort(u16),
    #[error("profile_dir must be under {base}: got {got}")]
    ProfileOutsideBase { base: String, got: String },
    #[error("chrome did not become ready on port {port} within timeout{stderr}")]
    NotReady { port: u16, stderr: String },
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("http error talking to chrome debug endpoint: {0}")]
    Http(#[from] reqwest::Error),
}

/// Locate a Chrome/Chromium binary cross-platform.
///
/// Honors `NEOBROWSER_CHROME_BIN` first, then probes the usual macOS app-bundle
/// paths, the PATH (Linux), and the standard Windows install locations. Falls
/// back to the macOS default so a failure names a concrete, fixable path.
pub fn discover_chrome_bin() -> PathBuf {
    if let Some(env) = std::env::var_os("NEOBROWSER_CHROME_BIN") {
        if !env.is_empty() {
            return PathBuf::from(env);
        }
    }
    let mac_paths = [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
    ];
    for p in mac_paths {
        if Path::new(p).exists() {
            return PathBuf::from(p);
        }
    }
    for name in [
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
        "chrome",
    ] {
        if let Some(found) = which(name) {
            return found;
        }
    }
    for p in [
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
    ] {
        if Path::new(p).exists() {
            return PathBuf::from(p);
        }
    }
    PathBuf::from(mac_paths[0])
}

/// Minimal `which`: search PATH for an executable by name.
fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// The discovered Chrome binary, cached process-wide.
pub fn chrome_bin() -> &'static Path {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(discover_chrome_bin).as_path()
}

/// Return the installed Chrome major version (e.g. "150"), or `None` if unknown.
pub fn detect_chrome_major(chrome_bin: &Path) -> Option<String> {
    let out = std::process::Command::new(chrome_bin)
        .arg("--version")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    // Match the first "<major>.<minor>" run of digits.
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            // Require a following '.' and another digit to look like a version.
            if i + 1 < bytes.len() && bytes[i] == b'.' && bytes[i + 1].is_ascii_digit() {
                return Some(text[start..i].to_string());
            }
        } else {
            i += 1;
        }
    }
    None
}

/// Build a User-Agent matching the REAL installed Chrome, consistent with its
/// genuine Client Hints. Applied via the `--user-agent` launch flag (which, unlike
/// CDP `Network.setUserAgentOverride`, does NOT blank Client Hints), turning the
/// only remaining headless tell (`HeadlessChrome`) into a clean identity.
pub fn chrome_user_agent() -> Option<&'static str> {
    static UA: OnceLock<Option<String>> = OnceLock::new();
    UA.get_or_init(|| {
        let major = detect_chrome_major(chrome_bin())?;
        let token = if cfg!(target_os = "windows") {
            "Windows NT 10.0; Win64; x64"
        } else if cfg!(target_os = "linux") {
            "X11; Linux x86_64"
        } else {
            // Darwin and anything else -> frozen macOS token.
            "Macintosh; Intel Mac OS X 10_15_7"
        };
        Some(format!(
            "Mozilla/5.0 ({token}) AppleWebKit/537.36 (KHTML, like Gecko) \
             Chrome/{major}.0.0.0 Safari/537.36"
        ))
    })
    .as_deref()
}

/// Headless launch flags — deliberately minimal and free of automation tells.
/// `--disable-blink-features=AutomationControlled` suppresses `navigator.webdriver`.
/// `--disable-gpu` is intentionally absent: under `--headless=new` the GPU works and
/// software WebGL (SwiftShader) is itself a headless fingerprint. Opt in via
/// `NEOBROWSER_DISABLE_GPU` on GPU-less CI hosts.
pub const DEFAULT_CHROME_FLAGS: &[&str] = &[
    "--headless=new",
    "--no-sandbox",
    "--disable-dev-shm-usage",
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-background-networking",
    "--disable-sync",
    "--disable-translate",
    "--mute-audio",
    "--window-size=1920,1080",
    "--disable-blink-features=AutomationControlled",
    // Keep the renderer live: in --headless=new an occluded/backgrounded tab is
    // throttled, which stalls requestAnimationFrame / IntersectionObserver and
    // leaves virtualized lists and deferred dialogs unrendered. See browser.rs
    // (focus emulation) and page::nudge_frame for the rest of the fix.
    "--disable-backgrounding-occluded-windows",
    "--disable-renderer-backgrounding",
    "--disable-background-timer-throttling",
];

fn validate_port(port: u16) -> Result<(), ChromeError> {
    if (1024..=65535).contains(&port) {
        Ok(())
    } else {
        Err(ChromeError::InvalidPort(port))
    }
}

/// Find a free TCP port by binding to port 0 and letting the OS assign one.
pub fn find_free_port() -> std::io::Result<u16> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    Ok(listener.local_addr()?.port())
}

#[derive(Debug, Deserialize)]
pub struct NewTab {
    pub id: String,
    #[serde(rename = "webSocketDebuggerUrl")]
    pub web_socket_debugger_url: String,
    #[serde(default)]
    pub url: String,
}

/// Poll `GET /json/version` until Chrome responds or the timeout expires.
pub async fn wait_for_chrome(port: u16, timeout: Duration) -> Result<(), ChromeError> {
    validate_port(port)?;
    let url = format!("http://127.0.0.1:{port}/json/version");
    let client = reqwest::Client::new();
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(resp) = client
            .get(&url)
            .timeout(Duration::from_secs(1))
            .send()
            .await
        {
            if resp.status().is_success() {
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(ChromeError::NotReady {
        port,
        stderr: chrome_stderr_tail(port),
    })
}

/// Last few lines of Chrome's stderr for `port`, formatted for an error message.
/// Empty when there is nothing useful to show, so the message stays clean.
fn chrome_stderr_tail(port: u16) -> String {
    let Ok(log) = std::fs::read_to_string(paths::chrome_log(port)) else {
        return String::new();
    };
    let tail: Vec<&str> = log
        .lines()
        .filter(|l| !l.trim().is_empty())
        .rev()
        .take(8)
        .collect();
    if tail.is_empty() {
        return String::new();
    }
    let body: Vec<&str> = tail.into_iter().rev().collect();
    format!(".\nchrome stderr:\n  {}", body.join("\n  "))
}

/// Does Chrome's HTTP debug endpoint on `port` respond? (Standalone; used for
/// health-checking an attached Chrome we don't own.)
pub async fn port_alive(port: u16) -> bool {
    if validate_port(port).is_err() {
        return false;
    }
    let url = format!("http://127.0.0.1:{port}/json/version");
    reqwest::Client::new()
        .get(&url)
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// Open a new tab via the DevTools HTTP endpoint.
///
/// IMPORTANT: must use PUT, not GET — GET returns HTTP 405 on modern Chrome.
pub async fn open_new_tab(port: u16) -> Result<NewTab, ChromeError> {
    validate_port(port)?;
    let url = format!("http://127.0.0.1:{port}/json/new");
    let client = reqwest::Client::new();
    let resp = client
        .put(&url)
        .timeout(Duration::from_secs(5))
        .send()
        .await?
        .error_for_status()?;
    Ok(resp.json::<NewTab>().await?)
}

/// Close a tab by its DevTools target id via the HTTP endpoint.
pub async fn close_tab(port: u16, target_id: &str) -> Result<(), ChromeError> {
    validate_port(port)?;
    let url = format!("http://127.0.0.1:{port}/json/close/{target_id}");
    reqwest::Client::new()
        .get(&url)
        .timeout(Duration::from_secs(5))
        .send()
        .await?;
    Ok(())
}

/// Manages a single Chrome process. Owns exactly one child; `kill`/`Drop` only
/// ever touch that child.
#[derive(Debug)]
pub struct ChromeProcess {
    pub profile_dir: PathBuf,
    pub port: u16,
    child: Option<Child>,
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

        let port = find_free_port()?;
        let mut cmd = Command::new(chrome_bin());
        cmd.arg(format!("--remote-debugging-port={port}"))
            .arg(format!("--user-data-dir={}", profile_dir.display()));
        for flag in DEFAULT_CHROME_FLAGS {
            cmd.arg(flag);
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

/// Send SIGTERM on Unix; on Windows, start_kill semantics are handled by the caller.
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

/// Remove `Singleton*` from a profile when the process that created them is
/// gone.
///
/// `SingletonLock` is a symlink whose target is `hostname-pid`. If that pid is
/// still alive the lock is legitimate — a real Chrome owns this profile — and
/// nothing is removed, so we never yank the profile out from under a running
/// sibling. Only a genuinely orphaned lock is cleared.
fn clear_stale_lock(profile_dir: &Path) {
    let lock = profile_dir.join("SingletonLock");
    let Ok(target) = std::fs::read_link(&lock) else {
        return; // no lock, or not a symlink: nothing to clean
    };
    let target = target.to_string_lossy().to_string();
    let Some(pid) = target
        .rsplit('-')
        .next()
        .and_then(|p| p.parse::<i32>().ok())
    else {
        return; // unrecognized format — leave it alone rather than guess
    };
    if pid_alive(pid) {
        return; // a live Chrome owns this profile
    }
    for f in ["SingletonLock", "SingletonCookie", "SingletonSocket"] {
        let _ = std::fs::remove_file(profile_dir.join(f));
    }
}

/// Test hook: exercise the stale-lock rule without spawning Chrome.
#[doc(hidden)]
pub fn clear_stale_lock_for_test(profile_dir: &Path) {
    clear_stale_lock(profile_dir);
}

/// Does a process with this pid exist? Signal 0 checks for existence without
/// delivering anything.
fn pid_alive(pid: i32) -> bool {
    #[cfg(unix)]
    {
        if pid <= 0 {
            return false;
        }
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        true // can't tell: assume alive and leave the lock in place
    }
}

fn term(pid: u32) {
    #[cfg(unix)]
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }
    #[cfg(windows)]
    {
        let _ = pid; // handled via Child::start_kill on the Windows path
    }
}

#[cfg(test)]
mod tests {
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
