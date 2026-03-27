//! Chrome process launcher — find, launch, and manage Chrome.
//!
//! Searches standard paths for Chrome/Chromium, launches with CDP enabled,
//! kills zombies from previous runs, and cleans up on Drop.

use crate::{ChromeError, Result};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

/// Standard Chrome/Chromium binary locations.
/// Standard Chrome/Chromium binary locations (checked in order).
/// Playwright's bundled Chromium is preferred to avoid conflicts with running Chrome.
fn chrome_paths() -> Vec<String> {
    vec![
        // System Chrome (works with --headless=new and temp profiles)
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".into(),
        "/Applications/Chromium.app/Contents/MacOS/Chromium".into(),
        "/usr/bin/chromium".into(),
        "/usr/bin/chromium-browser".into(),
        "/usr/bin/google-chrome".into(),
        "/usr/bin/google-chrome-stable".into(),
    ]
}
const CHROME_PATHS: &[&str] = &[];

/// Find the Chrome binary on this system.
pub fn find_chrome() -> Result<PathBuf> {
    for p in chrome_paths() {
        let path = Path::new(&p);
        if path.exists() {
            return Ok(path.to_path_buf());
        }
    }
    Err(ChromeError::NotFound)
}

/// A running Chrome process with CDP debugging enabled.
/// Kills the process and cleans up on Drop.
pub struct ChromeProcess {
    child: Child,
    /// The CDP debugging port.
    pub port: u16,
    /// The user-data-dir used for this instance.
    pub profile_dir: PathBuf,
}

impl ChromeProcess {
    /// Launch Chrome with remote debugging on a free port.
    ///
    /// If `headless` is true, positions the window offscreen instead of
    /// using `--headless=new` (preserves real browser fingerprint).
    pub async fn launch(profile_dir: Option<&str>, headless: bool) -> Result<Self> {
        let chrome_bin = find_chrome()?;
        let profile = resolve_profile_dir(profile_dir);

        kill_zombies_sync(&profile);
        remove_singleton_lock(&profile);

        let port = find_free_port_sync()?;
        let mut args = vec![
            format!("--remote-debugging-port={port}"),
            format!("--user-data-dir={}", profile.display()),
            "--no-first-run".to_string(),
            "--no-default-browser-check".to_string(),
            "--disable-background-networking".to_string(),
        ];

        // NEOMODE: anti-detection flags (always, not just headless)
        args.push("--disable-blink-features=AutomationControlled".to_string());
        args.push("--no-sandbox".to_string());
        args.push("--disable-dev-shm-usage".to_string());

        if headless {
            args.push("--headless=new".to_string());
            // Override UA to remove "HeadlessChrome"
            args.push("--user-agent=Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.0.0.0 Safari/537.36".to_string());
        }

        eprintln!("[neo-chrome] Binary: {}", chrome_bin.display());
        eprintln!("[neo-chrome] Profile: {}", profile.display());
        eprintln!("[neo-chrome] Port: {port}");
        eprintln!("[neo-chrome] Args: {:?}", args);

        eprintln!("[neo-chrome] Spawning...");
        let child = Command::new(&chrome_bin)
            .args(&args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;
        eprintln!("[neo-chrome] Spawned PID={}", child.id());

        // Wait for CDP to be ready
        eprintln!("[neo-chrome] Waiting for CDP on port {port}...");
        wait_for_devtools_sync(port)?;
        eprintln!("[neo-chrome] CDP ready!");

        Ok(Self {
            child,
            port,
            profile_dir: profile,
        })
    }

    /// Get the WebSocket URL for the browser target.
    /// Get WS URL. NOT async — safe to call from any context.
    pub fn ws_url_sync(&self) -> Result<String> {
        for _ in 0..20 {
            if let Some(resp) = sync_http_get(self.port, "/json/version") {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&resp) {
                    if let Some(ws) = json.get("webSocketDebuggerUrl").and_then(|v| v.as_str()) {
                        return Ok(ws.to_string());
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        Err(ChromeError::ConnectionFailed("Could not get WebSocket URL".into()))
    }

    /// Async wrapper for compatibility
    pub async fn ws_url(&self) -> Result<String> {
        self.ws_url_sync()
    }

    /// Get first page target ID. NOT async.
    pub fn first_target_id_sync(&self) -> Result<String> {
        for _ in 0..20 {
            if let Some(resp) = sync_http_get(self.port, "/json/list") {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&resp) {
                    if let Some(arr) = json.as_array() {
                        for target in arr {
                            if target.get("type").and_then(|v| v.as_str()) == Some("page") {
                                if let Some(id) = target.get("id").and_then(|v| v.as_str()) {
                                    return Ok(id.to_string());
                                }
                            }
                        }
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        Err(ChromeError::ConnectionFailed("No page target found".into()))
    }

    pub async fn first_target_id(&self) -> Result<String> {
        self.first_target_id_sync()
    }

    /// Kill the Chrome process.
    pub async fn kill(&mut self) {
        let _ = self.child.kill();
    }
}

impl Drop for ChromeProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

// ─── Internal helpers ───

/// Resolve the profile directory, defaulting to a temp-like location.
fn resolve_profile_dir(profile: Option<&str>) -> PathBuf {
    if let Some(p) = profile {
        return PathBuf::from(p);
    }
    if let Ok(p) = std::env::var("NEOCHROME_PROFILE") {
        return PathBuf::from(p);
    }
    // Use a fresh temp dir each time to avoid SingletonLock conflicts
    let dir = std::env::temp_dir().join(format!("neo-chrome-{}", std::process::id()));
    std::fs::create_dir_all(&dir).ok();
    dir
}

/// Kill any Chrome processes using the same profile directory.
async fn kill_zombies(profile_dir: &Path) {
    let profile_str = profile_dir.to_string_lossy();
    let _ = Command::new("pkill")
        .args(["-f", &format!("--user-data-dir={profile_str}")])
        .output();
    std::thread::sleep(std::time::Duration::from_millis(200));
}

/// Remove stale SingletonLock that prevents Chrome from launching.
fn remove_singleton_lock(profile_dir: &Path) {
    let lock = profile_dir.join("SingletonLock");
    if lock.exists() {
        let _ = std::fs::remove_file(&lock);
    }
}

/// Find a free TCP port by binding to :0.
async fn find_free_port() -> Result<u16> {
    // Use std TCP to avoid tokio I/O driver issues
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

/// Wait for Chrome's CDP to be ready.
async fn wait_for_devtools(_profile_dir: &Path, port: u16) -> Result<()> {
    tokio::task::spawn_blocking(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while std::time::Instant::now() < deadline {
            if check_cdp_ready(port) {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
        Err(ChromeError::Timeout("Chrome CDP not ready within 15s".into()))
    }).await.unwrap_or_else(|_| Err(ChromeError::Timeout("spawn_blocking failed".into())))
}

/// Sync HTTP GET to localhost CDP.
fn sync_http_get(port: u16, path: &str) -> Option<String> {
    use std::io::{Read, Write};
    let addr: std::net::SocketAddr = format!("127.0.0.1:{port}").parse().ok()?;
    let mut stream = std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(1000)).ok()?;
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(2000)));
    let req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).ok()?;
    // Read in chunks — don't use read_to_end (hangs if keep-alive)
    let mut buf = vec![0u8; 65536];
    let mut total = Vec::new();
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => total.extend_from_slice(&buf[..n]),
            Err(_) => break,
        }
        // Chrome CDP HTTP response is small (~500 bytes). Break once we have headers + body.
        if total.windows(4).any(|w| w == b"\r\n\r\n") && total.len() > 200 {
            break;
        }
    }
    let text = String::from_utf8_lossy(&total).to_string();
    if let Some(idx) = text.find("\r\n\r\n") {
        Some(text[idx + 4..].to_string())
    } else {
        Some(text)
    }
}

fn kill_zombies_sync(profile_dir: &Path) {
    let profile_str = profile_dir.to_string_lossy();
    let _ = Command::new("pkill")
        .args(["-f", &format!("--user-data-dir={profile_str}")])
        .output();
    std::thread::sleep(std::time::Duration::from_millis(200));
}

fn find_free_port_sync() -> Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

fn wait_for_devtools_sync(port: u16) -> Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while std::time::Instant::now() < deadline {
        if check_cdp_ready(port) {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
    Err(ChromeError::Timeout("Chrome CDP not ready within 15s".into()))
}

/// Sync check if CDP port is ready.
fn check_cdp_ready(port: u16) -> bool {
    use std::io::{Read, Write};
    let addr: std::net::SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let Ok(mut stream) = std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(500)) else {
        return false;
    };
    let req = format!("GET /json/version HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    if stream.write_all(req.as_bytes()).is_err() { return false; }
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(1000)));
    let mut buf = vec![0u8; 4096];
    match stream.read(&mut buf) {
        Ok(n) if n > 0 => {
            let text = String::from_utf8_lossy(&buf[..n]);
            text.contains("Browser")
        }
        _ => false,
    }
}

/// Minimal HTTP GET without pulling in reqwest — uses raw TCP.
pub(crate) async fn reqwest_lite(url: &str) -> std::result::Result<String, String> {
    let parsed: url::Url = url.parse().map_err(|e: url::ParseError| e.to_string())?;
    let host = parsed.host_str().ok_or("no host")?;
    let port = parsed.port().unwrap_or(80);
    let path = parsed.path();

    let addr = format!("{host}:{port}");
    let stream = tokio::net::TcpStream::connect(&addr)
        .await
        .map_err(|e| e.to_string())?;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = stream;
    let req = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    stream
        .write_all(req.as_bytes())
        .await
        .map_err(|e| e.to_string())?;

    let mut buf = Vec::new();
    stream
        .read_to_end(&mut buf)
        .await
        .map_err(|e| e.to_string())?;

    let text = String::from_utf8_lossy(&buf).to_string();
    // Extract body after \r\n\r\n
    if let Some(idx) = text.find("\r\n\r\n") {
        Ok(text[idx + 4..].to_string())
    } else {
        Err("No HTTP body found".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_chrome() {
        // Skip on CI or systems without Chrome.
        match find_chrome() {
            Ok(path) => assert!(path.exists()),
            Err(ChromeError::NotFound) => {
                eprintln!("Chrome not installed — skipping");
            }
            Err(e) => panic!("Unexpected error: {e}"),
        }
    }
}
