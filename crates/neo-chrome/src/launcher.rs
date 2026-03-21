//! Chrome process launcher — find, launch, and manage Chrome.
//!
//! Searches standard paths for Chrome/Chromium, launches with CDP enabled,
//! kills zombies from previous runs, and cleans up on Drop.

use crate::{ChromeError, Result};
use std::path::{Path, PathBuf};
use tokio::process::{Child, Command};

/// Standard Chrome/Chromium binary locations.
const CHROME_PATHS: &[&str] = &[
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "/usr/bin/chromium",
    "/usr/bin/chromium-browser",
    "/usr/bin/google-chrome",
    "/usr/bin/google-chrome-stable",
];

/// Find the Chrome binary on this system.
pub fn find_chrome() -> Result<PathBuf> {
    for p in CHROME_PATHS {
        let path = Path::new(p);
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

        kill_zombies(&profile).await;
        remove_singleton_lock(&profile);

        let port = find_free_port().await?;
        let mut args = vec![
            format!("--remote-debugging-port={port}"),
            format!("--user-data-dir={}", profile.display()),
            "--no-first-run".to_string(),
            "--no-default-browser-check".to_string(),
            "--disable-background-networking".to_string(),
        ];

        if headless {
            args.push("--window-position=-32000,-32000".to_string());
            args.push("--window-size=1920,1080".to_string());
        }

        let child = Command::new(&chrome_bin)
            .args(&args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;

        // Wait for DevToolsActivePort file to appear.
        wait_for_devtools(&profile, port).await?;

        Ok(Self {
            child,
            port,
            profile_dir: profile,
        })
    }

    /// Get the WebSocket URL for the browser target.
    pub async fn ws_url(&self) -> Result<String> {
        let url = format!("http://127.0.0.1:{}/json/version", self.port);
        for _ in 0..20 {
            if let Ok(resp) = reqwest_lite(&url).await {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&resp) {
                    if let Some(ws) = json.get("webSocketDebuggerUrl").and_then(|v| v.as_str()) {
                        return Ok(ws.to_string());
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        Err(ChromeError::ConnectionFailed(
            "Could not get WebSocket URL from /json/version".into(),
        ))
    }

    /// Get the first page target ID.
    pub async fn first_target_id(&self) -> Result<String> {
        let url = format!("http://127.0.0.1:{}/json/list", self.port);
        for _ in 0..20 {
            if let Ok(resp) = reqwest_lite(&url).await {
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
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        Err(ChromeError::ConnectionFailed("No page target found".into()))
    }

    /// Kill the Chrome process.
    pub async fn kill(&mut self) {
        let _ = self.child.kill().await;
    }
}

impl Drop for ChromeProcess {
    fn drop(&mut self) {
        // Best-effort kill — can't await in Drop, so use start_kill.
        let _ = self.child.start_kill();
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
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"));
    home.join(".neo-chrome")
}

/// Kill any Chrome processes using the same profile directory.
async fn kill_zombies(profile_dir: &Path) {
    let profile_str = profile_dir.to_string_lossy();
    // Best-effort: use pkill with user-data-dir match.
    let _ = Command::new("pkill")
        .args(["-f", &format!("--user-data-dir={profile_str}")])
        .output()
        .await;
    // Give processes time to die.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
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
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

/// Wait for Chrome's DevToolsActivePort file, indicating CDP is ready.
async fn wait_for_devtools(profile_dir: &Path, _port: u16) -> Result<()> {
    let devtools_file = profile_dir.join("DevToolsActivePort");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        if devtools_file.exists() {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    Err(ChromeError::Timeout(
        "DevToolsActivePort not created within 10s".into(),
    ))
}

/// Minimal HTTP GET without pulling in reqwest — uses raw TCP.
async fn reqwest_lite(url: &str) -> std::result::Result<String, String> {
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
