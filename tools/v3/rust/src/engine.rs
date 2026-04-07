//! Browser engine — raw CDP, no chromiumoxide.
//!
//! Launches Chrome, connects via WebSocket, drives everything
//! through direct CDP commands. No lifecycle waits, no abstractions
//! that block. Just send command → get result.

use crate::cdp::CdpSession;
use crate::cdp::ScopedTransport;
use crate::cdp::{page, runtime, dom, input, network, emulation, target, fetch, browser_domain};
use crate::semantic;
use crate::vision;
use crate::wom;
use html5ever::parse_document;
use html5ever::tendril::TendrilSink;
use markup5ever_rcdom::RcDom;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex as TokioMutex;

// ─── Helpers ───

/// Convert CdpResult errors to Box<dyn Error> (engine's error type).
/// CdpResult uses Send+Sync bounds, engine doesn't. This bridges them.
fn cdp_err(e: Box<dyn std::error::Error + Send + Sync>) -> Box<dyn std::error::Error> {
    e
}

/// ISO 8601 timestamp without chrono dependency.
fn chrono_now() -> String {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = d.as_secs();
    // Simple UTC timestamp: seconds since epoch as string
    // For human-readable, the consumer can convert.
    format!("{secs}")
}

// ─── Chrome binary discovery ───

fn find_chrome() -> Result<&'static str, &'static str> {
    let paths = [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
        "/usr/bin/google-chrome",
    ];
    for p in paths {
        if std::path::Path::new(p).exists() {
            return Ok(p);
        }
    }
    Err("Chrome/Chromium not found")
}

/// Find Chrome's DevToolsActivePort file to get the debug port.
fn find_debug_port() -> Option<(u16, String)> {
    let home = dirs::home_dir()?;
    // Standard macOS Chrome location
    let paths = [
        home.join("Library/Application Support/Google/Chrome/DevToolsActivePort"),
        home.join(".config/chromium/DevToolsActivePort"),
    ];
    for path in &paths {
        if let Ok(content) = std::fs::read_to_string(path) {
            let lines: Vec<&str> = content.lines().collect();
            if let Some(port_str) = lines.first() {
                if let Ok(port) = port_str.parse::<u16>() {
                    let ws_path = lines.get(1).unwrap_or(&"/devtools/browser");
                    return Some((port, ws_path.to_string()));
                }
            }
        }
    }
    None
}

// ─── CDP Network Types ───

/// A captured network request/response pair from CDP events.
#[derive(Clone, Debug)]
struct CdpNetworkEntry {
    request_id: String,
    method: String,
    url: String,
    request_headers: Value,
    post_data: Option<String>,
    resource_type: String,
    timestamp: f64,
    response_status: Option<i64>,
    response_headers: Option<Value>,
    response_mime: Option<String>,
    frame_id: String,
}

impl CdpNetworkEntry {
    fn to_json(&self) -> Value {
        let mut obj = json!({
            "requestId": self.request_id,
            "method": self.method,
            "url": self.url,
            "requestHeaders": self.request_headers,
            "resourceType": self.resource_type,
            "timestamp": self.timestamp,
            "frameId": self.frame_id,
        });
        if let Some(ref pd) = self.post_data {
            obj["postData"] = json!(pd);
        }
        if let Some(status) = self.response_status {
            obj["status"] = json!(status);
        }
        if let Some(ref h) = self.response_headers {
            obj["responseHeaders"] = h.clone();
        }
        if let Some(ref m) = self.response_mime {
            obj["mimeType"] = json!(m);
        }
        obj
    }
}

/// Rule for intercepting requests and responding with custom data.
#[derive(Clone, Debug)]
struct InterceptRule {
    url_pattern: String,
    response_body: String,
    status_code: u16,
    content_type: String,
}

// ─── Workflow Mapper ───

/// A single step recorded during workflow mapping.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkflowStep {
    pub step_number: usize,
    pub action: String,         // "observe", "click", "type", "select", "navigate", etc.
    pub target: Option<String>, // CSS selector or text label
    pub value: Option<String>,  // typed text, selected value, etc.
    pub url: String,
    pub observation: Value,     // rich page state captured after this step
    pub network_requests: Vec<Value>, // API calls made during this step
    pub timestamp: String,
    pub notes: String,
}

/// A complete workflow recording — becomes a reusable playbook.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Workflow {
    pub name: String,
    pub start_url: String,
    pub steps: Vec<WorkflowStep>,
    pub vue_model_schema: Value,          // accumulated Vue/React model structure
    pub api_endpoints_discovered: Vec<Value>, // all API endpoints seen
    pub field_map: Value,                 // label → selector mapping for replay
}

impl Workflow {
    fn new(name: &str, url: &str) -> Self {
        Self {
            name: name.to_string(),
            start_url: url.to_string(),
            steps: Vec::new(),
            vue_model_schema: json!({}),
            api_endpoints_discovered: Vec::new(),
            field_map: json!({}),
        }
    }
}

// ─── Session ───

pub struct Session {
    cdp: CdpSession,
    /// CDP target ID for the active page
    target_id: String,
    /// Page-level CDP session (attached to a target)
    page_session_id: String,
    pub last_url: String,
    chrome_process: Option<tokio::process::Child>,
    connected_mode: bool,
    /// CDP session ID for the active cross-origin iframe (if any).
    /// When set, eval/click/type/focus commands route through this session.
    active_frame_session_id: Option<String>,
    /// Frame ID of the active iframe (from Page.getFrameTree).
    active_frame_id: Option<String>,
    /// CDP-level captured network entries (survives navigation, captures cross-origin iframes)
    cdp_network_entries: Arc<TokioMutex<Vec<CdpNetworkEntry>>>,
    /// Whether CDP network capture is active
    cdp_network_active: Arc<std::sync::atomic::AtomicBool>,
    /// Rules for Fetch.requestPaused interception
    intercept_rules: Arc<TokioMutex<Vec<InterceptRule>>>,
    /// Active workflow being recorded (workflow mapper)
    pub active_workflow: Option<Workflow>,
}

/// Default persistent profile directory for the AI browser.
/// Override with NEOBROWSER_PROFILE env var to run multiple instances.
pub fn default_profile_dir() -> std::path::PathBuf {
    if let Ok(custom) = std::env::var("NEOBROWSER_PROFILE") {
        return std::path::PathBuf::from(custom);
    }
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".neobrowser")
        .join("profile")
}

/// Write cookies directly to the Chrome profile's SQLite database.
/// Must be called BEFORE Chrome launches (Chrome locks the file).
pub fn persist_cookies_to_profile(
    profile_dir: &std::path::Path,
    cookie_file: &str,
) -> Result<usize, Box<dyn std::error::Error>> {
    let content = std::fs::read_to_string(cookie_file)?;
    let data: Value = serde_json::from_str(&content)?;
    let cookies = if let Some(arr) = data.as_array() {
        arr.clone()
    } else if let Some(arr) = data.get("cookies").and_then(|c| c.as_array()) {
        arr.clone()
    } else {
        return Err("Invalid cookie format".into());
    };

    let db_path = profile_dir.join("Default").join("Cookies");
    std::fs::create_dir_all(profile_dir.join("Default"))?;

    let conn = rusqlite::Connection::open(&db_path)?;

    // Create table if first run
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS cookies (
            creation_utc INTEGER NOT NULL,
            host_key TEXT NOT NULL DEFAULT '',
            top_frame_site_key TEXT NOT NULL DEFAULT '',
            name TEXT NOT NULL DEFAULT '',
            value TEXT NOT NULL DEFAULT '',
            encrypted_value BLOB NOT NULL DEFAULT X'',
            path TEXT NOT NULL DEFAULT '/',
            expires_utc INTEGER NOT NULL DEFAULT 0,
            is_secure INTEGER NOT NULL DEFAULT 0,
            is_httponly INTEGER NOT NULL DEFAULT 0,
            last_access_utc INTEGER NOT NULL DEFAULT 0,
            has_expires INTEGER NOT NULL DEFAULT 1,
            is_persistent INTEGER NOT NULL DEFAULT 1,
            priority INTEGER NOT NULL DEFAULT 1,
            samesite INTEGER NOT NULL DEFAULT -1,
            source_scheme INTEGER NOT NULL DEFAULT 0,
            source_port INTEGER NOT NULL DEFAULT -1,
            last_update_utc INTEGER NOT NULL DEFAULT 0,
            source_type INTEGER NOT NULL DEFAULT 0,
            has_cross_site_ancestor INTEGER NOT NULL DEFAULT 0,
            UNIQUE (host_key, top_frame_site_key, name, path, source_scheme, source_port, has_cross_site_ancestor)
        );"
    )?;

    // Chrome epoch: microseconds since 1601-01-01
    // Unix epoch offset in microseconds: 11644473600 * 1_000_000
    let chrome_epoch_offset: i64 = 11_644_473_600 * 1_000_000;
    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_micros() as i64;
    let now_chrome = now_unix + chrome_epoch_offset;
    let expires_30d = now_chrome + 30 * 86400 * 1_000_000;

    let mut count = 0;
    for c in &cookies {
        let name = match c.get("name").and_then(|v| v.as_str()) {
            Some(n) => n,
            None => continue,
        };
        let value = c.get("value").and_then(|v| v.as_str()).unwrap_or("");
        let domain = c.get("domain").and_then(|v| v.as_str()).unwrap_or("");
        let path = c.get("path").and_then(|v| v.as_str()).unwrap_or("/");
        let secure = c.get("secure").and_then(|v| v.as_bool()).unwrap_or(false) as i32;
        let http_only = c.get("httpOnly").and_then(|v| v.as_bool()).unwrap_or(false) as i32;

        // Convert expires from unix seconds to chrome microseconds
        let expires_unix = c.get("expires").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let expires_chrome = if expires_unix > 1.0 {
            (expires_unix as i64) * 1_000_000 + chrome_epoch_offset
        } else {
            expires_30d
        };

        let source_scheme = if secure == 1 { 2 } else { 1 };

        // REPLACE — update if exists, insert if not
        conn.execute(
            "INSERT OR REPLACE INTO cookies (
                creation_utc, host_key, name, value, encrypted_value,
                path, expires_utc, is_secure, is_httponly,
                last_access_utc, has_expires, is_persistent,
                priority, samesite, source_scheme, source_port,
                last_update_utc, top_frame_site_key, source_type, has_cross_site_ancestor
            ) VALUES (?1,?2,?3,?4,X'',?5,?6,?7,?8,?9,1,1,1,-1,?10,-1,?9,'',0,0)",
            params![
                now_chrome, domain, name, value,
                path, expires_chrome, secure, http_only,
                now_chrome, source_scheme,
            ],
        )?;
        count += 1;
    }

    eprintln!("[ENGINE] Persisted {count} cookies to profile SQLite");
    Ok(count)
}

/// Save CDP-format cookies to the Chrome profile's SQLite database.
/// Called during close() to persist session cookies that Chrome doesn't save itself.
pub fn save_cookies_to_profile(
    profile_dir: &std::path::Path,
    cookies: &[Value],
) -> Result<usize, Box<dyn std::error::Error>> {
    let db_path = profile_dir.join("Default").join("Cookies");
    std::fs::create_dir_all(profile_dir.join("Default"))?;

    let conn = rusqlite::Connection::open(&db_path)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS cookies (
            creation_utc INTEGER NOT NULL,
            host_key TEXT NOT NULL DEFAULT '',
            top_frame_site_key TEXT NOT NULL DEFAULT '',
            name TEXT NOT NULL DEFAULT '',
            value TEXT NOT NULL DEFAULT '',
            encrypted_value BLOB NOT NULL DEFAULT X'',
            path TEXT NOT NULL DEFAULT '/',
            expires_utc INTEGER NOT NULL DEFAULT 0,
            is_secure INTEGER NOT NULL DEFAULT 0,
            is_httponly INTEGER NOT NULL DEFAULT 0,
            last_access_utc INTEGER NOT NULL DEFAULT 0,
            has_expires INTEGER NOT NULL DEFAULT 1,
            is_persistent INTEGER NOT NULL DEFAULT 1,
            priority INTEGER NOT NULL DEFAULT 1,
            samesite INTEGER NOT NULL DEFAULT -1,
            source_scheme INTEGER NOT NULL DEFAULT 0,
            source_port INTEGER NOT NULL DEFAULT -1,
            last_update_utc INTEGER NOT NULL DEFAULT 0,
            source_type INTEGER NOT NULL DEFAULT 0,
            has_cross_site_ancestor INTEGER NOT NULL DEFAULT 0,
            UNIQUE (host_key, top_frame_site_key, name, path, source_scheme, source_port, has_cross_site_ancestor)
        );"
    )?;

    let chrome_epoch_offset: i64 = 11_644_473_600 * 1_000_000;
    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_micros() as i64;
    let now_chrome = now_unix + chrome_epoch_offset;
    let expires_30d = now_chrome + 30 * 86400 * 1_000_000;

    // Merge: INSERT OR REPLACE preserves cookies from pre-persistence
    // while updating any that Chrome modified during the session.

    let mut count = 0;
    for c in cookies {
        // CDP cookie format: {name, value, domain, path, expires, size, httpOnly, secure, ...}
        let name = match c.get("name").and_then(|v| v.as_str()) {
            Some(n) => n,
            None => continue,
        };
        let value = c.get("value").and_then(|v| v.as_str()).unwrap_or("");
        let domain = c.get("domain").and_then(|v| v.as_str()).unwrap_or("");
        let path = c.get("path").and_then(|v| v.as_str()).unwrap_or("/");
        let secure = c.get("secure").and_then(|v| v.as_bool()).unwrap_or(false) as i32;
        let http_only = c.get("httpOnly").and_then(|v| v.as_bool()).unwrap_or(false) as i32;

        // CDP expires is unix timestamp (seconds)
        let expires_unix = c.get("expires").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let expires_chrome = if expires_unix > 1.0 {
            (expires_unix as i64) * 1_000_000 + chrome_epoch_offset
        } else {
            expires_30d
        };

        // Skip session cookies (expires = -1 or 0) — they shouldn't persist
        let session_cookie = c.get("session").and_then(|v| v.as_bool()).unwrap_or(false);
        if session_cookie {
            continue;
        }

        let source_scheme = if secure == 1 { 2 } else { 1 };

        conn.execute(
            "INSERT OR REPLACE INTO cookies (
                creation_utc, host_key, name, value, encrypted_value,
                path, expires_utc, is_secure, is_httponly,
                last_access_utc, has_expires, is_persistent,
                priority, samesite, source_scheme, source_port,
                last_update_utc, top_frame_site_key, source_type, has_cross_site_ancestor
            ) VALUES (?1,?2,?3,?4,X'',?5,?6,?7,?8,?9,1,1,1,-1,?10,-1,?9,'',0,0)",
            params![
                now_chrome, domain, name, value,
                path, expires_chrome, secure, http_only,
                now_chrome, source_scheme,
            ],
        )?;
        count += 1;
    }

    eprintln!("[ENGINE] Saved {count} cookies to profile SQLite");
    Ok(count)
}

impl Session {
    /// Kill Chrome processes that use a specific user-data-dir.
    /// Only kills neobrowser-owned profiles, never the user's personal Chrome.
    fn kill_zombies(profile_str: &str) {
        use std::process::Command;
        let output = Command::new("pkill")
            .args(["-f", &format!("user-data-dir={profile_str}")])
            .output();
        if let Ok(ref out) = output {
            if out.status.success() {
                eprintln!("[ENGINE] Killed zombie Chrome processes for {profile_str}");
                // Wait for processes to die and release locks
                std::thread::sleep(std::time::Duration::from_millis(1000));
            }
        }
    }

    /// Launch a new Chrome with a random debug port, connect via CDP.
    /// Always uses a persistent profile directory so sessions survive restarts.
    /// - `user_data_dir = Some(path)` → use that specific profile
    /// - `user_data_dir = None` → use ~/.neobrowser/profile/ (default)
    /// - `headless = true` → headless Chrome (default, fast but Cloudflare detects it)
    /// - `headless = false` → headed Chrome (visible window, passes Cloudflare/captchas)
    pub async fn launch(
        user_data_dir: Option<&str>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Self::launch_ex(user_data_dir, true).await
    }

    /// Launch Chrome with pipe-based CDP (no TCP port = undetectable by Cloudflare).
    pub async fn launch_stealth(
        user_data_dir: Option<&str>,
        headless: bool,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let chrome = find_chrome()?;
        let profile_dir = match user_data_dir {
            Some(dir) => std::path::PathBuf::from(dir),
            None => default_profile_dir(),
        };
        std::fs::create_dir_all(&profile_dir)?;
        let profile_str = profile_dir.to_string_lossy().to_string();
        Self::kill_zombies(&profile_str);
        let lock_file = profile_dir.join("SingletonLock");
        if lock_file.exists() { let _ = std::fs::remove_file(&lock_file); }

        let mut args = vec![
            "--remote-debugging-pipe".to_string(), // PIPE not PORT
            format!("--user-data-dir={profile_str}"),
            "--no-first-run".to_string(),
            "--no-default-browser-check".to_string(),
            "--disable-dev-shm-usage".to_string(),
            "--window-size=1440,900".to_string(),
        ];
        if headless {
            args.push("--headless=new".to_string());
        }

        // Chrome --remote-debugging-pipe reads from fd 3 and writes to fd 4.
        // We create two pipes and set up the file descriptors before spawning.
        // os_pipe::pipe() returns (PipeReader, PipeWriter).
        use std::os::unix::io::{FromRawFd, IntoRawFd};
        use std::process::Stdio;

        // Pipe 1: we write → Chrome reads (Chrome's fd 3)
        // PipeReader goes to Chrome (fd 3), PipeWriter stays with us
        let (chrome_reads_from, we_write_to) = os_pipe::pipe()?;
        // Pipe 2: Chrome writes → we read (Chrome's fd 4)
        // PipeReader stays with us, PipeWriter goes to Chrome (fd 4)
        let (we_read_from, chrome_writes_to) = os_pipe::pipe()?;

        let chrome_read_fd = chrome_reads_from.into_raw_fd();  // PipeReader → fd 3
        let chrome_write_fd = chrome_writes_to.into_raw_fd();  // PipeWriter → fd 4

        let mut child = unsafe {
            tokio::process::Command::new(chrome)
                .args(&args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .pre_exec(move || {
                    // Duplicate our pipe ends to fd 3 (Chrome reads) and fd 4 (Chrome writes)
                    if libc::dup2(chrome_read_fd, 3) == -1 { return Err(std::io::Error::last_os_error()); }
                    if libc::dup2(chrome_write_fd, 4) == -1 { return Err(std::io::Error::last_os_error()); }
                    // Close the originals
                    libc::close(chrome_read_fd);
                    libc::close(chrome_write_fd);
                    Ok(())
                })
                .spawn()?
        };

        // Give Chrome time to start
        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

        // Convert our pipe ends to tokio async files
        // we_read_from = PipeReader (Chrome's output to us)
        // we_write_to = PipeWriter (our input to Chrome)
        let chrome_stdout = unsafe {
            tokio::fs::File::from_raw_fd(we_read_from.into_raw_fd())
        };
        let chrome_stdin = unsafe {
            tokio::fs::File::from_raw_fd(we_write_to.into_raw_fd())
        };

        eprintln!("[ENGINE] Chrome launched with pipe (no TCP port, stealth mode)");

        let cdp = CdpSession::connect_pipe(chrome_stdout, chrome_stdin)?;

        // Get the first page target
        let targets = target::get_targets(&cdp, None).await.map_err(cdp_err)?;
        let page_target = targets.iter()
            .find(|t| t.type_ == "page")
            .ok_or("No page target")?;
        let target_id = page_target.target_id.clone();

        let session_id = target::attach_to_target(&cdp, &target_id, Some(true)).await.map_err(cdp_err)?;

        let scoped = ScopedTransport::new(&cdp, &session_id);
        page::enable(&scoped).await.map_err(cdp_err)?;
        runtime::enable(&scoped).await.map_err(cdp_err)?;

        eprintln!("[ENGINE] Stealth ready — target={}, session={}", &target_id[..8], &session_id[..8]);

        Ok(Self {
            cdp,
            target_id,
            page_session_id: session_id,
            last_url: String::new(),
            chrome_process: Some(child),
            connected_mode: false,
            active_frame_session_id: None,
            active_frame_id: None,
            cdp_network_entries: Arc::new(TokioMutex::new(Vec::new())),
            cdp_network_active: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            intercept_rules: Arc::new(TokioMutex::new(Vec::new())),
            active_workflow: None,
        })
    }

    pub async fn launch_ex(
        user_data_dir: Option<&str>,
        headless: bool,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let chrome = find_chrome()?;

        // Persistent profile — always
        let profile_dir = match user_data_dir {
            Some(dir) => std::path::PathBuf::from(dir),
            None => default_profile_dir(),
        };
        std::fs::create_dir_all(&profile_dir)?;
        let profile_str = profile_dir.to_string_lossy().to_string();

        // Kill zombie Chrome processes using the same profile dir.
        // Without this, Chrome refuses to start ("profile already in use").
        Self::kill_zombies(&profile_str);

        // Remove stale SingletonLock left by crashed Chrome
        let lock_file = profile_dir.join("SingletonLock");
        if lock_file.exists() {
            eprintln!("[ENGINE] Removing stale SingletonLock");
            let _ = std::fs::remove_file(&lock_file);
        }

        // Find a free port
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        drop(listener);

        // Minimal flags — every extra flag is a detection signal.
        // DO NOT add --disable-blink-features=AutomationControlled
        // (it's detectable because normal Chrome doesn't have it).
        let mut args = vec![
            format!("--remote-debugging-port={port}"),
            format!("--user-data-dir={profile_str}"),
            "--disable-dev-shm-usage".to_string(),
            "--no-first-run".to_string(),
            "--no-default-browser-check".to_string(),
            "--window-size=1440,900".to_string(),
        ];
        if headless {
            args.push("--headless=new".to_string());
        }

        let child = tokio::process::Command::new(chrome)
            .args(&args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        eprintln!("[ENGINE] Chrome launching on port {port}...");

        // Wait for Chrome to be ready (poll /json/version)
        let client = crate::http_client::local(2)?;

        let mut ws_url = String::new();
        for _ in 0..30 {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let url = format!("http://127.0.0.1:{port}/json/version");
            if let Ok(resp) = client.get(&url).send().await {
                if let Ok(data) = resp.json::<Value>().await {
                    if let Some(ws) = data["webSocketDebuggerUrl"].as_str() {
                        ws_url = ws.to_string();
                        break;
                    }
                }
            }
        }
        if ws_url.is_empty() {
            return Err("Chrome didn't start in time".into());
        }

        let cdp = CdpSession::connect(&ws_url).await?;

        // Create a new page (target)
        let target_id = target::create_target(&cdp, "about:blank", None, None, None, None, None, None)
            .await.map_err(cdp_err)?;

        // Attach to the target to get a session
        let session_id = target::attach_to_target(&cdp, &target_id, Some(true))
            .await.map_err(cdp_err)?;

        // Enable Page and Runtime domains
        let scoped = ScopedTransport::new(&cdp, &session_id);
        page::enable(&scoped).await.map_err(cdp_err)?;
        runtime::enable(&scoped).await.map_err(cdp_err)?;

        // STEALTH STRATEGY: Do NOT inject anything during launch.
        // Cloudflare Turnstile detects early CDP modifications.
        // Stealth is applied AFTER the first navigation via apply_stealth().
        eprintln!("[ENGINE] Ready (clean, no stealth yet) — target={}, session={}", &target_id[..8], &session_id[..8]);

        Ok(Self {
            cdp,
            target_id,
            page_session_id: session_id,
            last_url: String::new(),
            chrome_process: Some(child),
            connected_mode: false,
            active_frame_session_id: None,
            active_frame_id: None,
            cdp_network_entries: Arc::new(TokioMutex::new(Vec::new())),
            cdp_network_active: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            intercept_rules: Arc::new(TokioMutex::new(Vec::new())),
            active_workflow: None,
        })
    }

    /// Connect to an already-running Chrome via its debug port.
    pub async fn connect_port(port: u16) -> Result<Self, Box<dyn std::error::Error>> {
        let client = crate::http_client::local(2)?;

        let url = format!("http://127.0.0.1:{port}/json/version");
        let resp = client.get(&url).send().await?;
        let data: Value = resp.json().await?;
        let ws_url = data["webSocketDebuggerUrl"]
            .as_str()
            .ok_or("No webSocketDebuggerUrl")?;

        Self::connect(ws_url).await
    }

    /// Connect to Chrome's WebSocket directly.
    pub async fn connect(ws_url: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let cdp = CdpSession::connect(ws_url).await?;

        // Get existing pages
        let targets = target::get_targets(&cdp, None).await.map_err(cdp_err)?;

        // Find the first page target
        let page_target = targets
            .iter()
            .find(|t| t.type_ == "page")
            .ok_or("No page target found")?;

        let target_id = page_target.target_id.clone();
        let current_url = page_target.url.clone();

        // Attach
        let session_id = target::attach_to_target(&cdp, &target_id, Some(true))
            .await.map_err(cdp_err)?;

        let scoped = ScopedTransport::new(&cdp, &session_id);
        page::enable(&scoped).await.map_err(cdp_err)?;
        runtime::enable(&scoped).await.map_err(cdp_err)?;

        eprintln!("[ENGINE] Connected to existing Chrome — {current_url}");

        Ok(Self {
            cdp,
            target_id,
            page_session_id: session_id,
            last_url: current_url,
            chrome_process: None,
            connected_mode: true,
            active_frame_session_id: None,
            active_frame_id: None,
            cdp_network_entries: Arc::new(TokioMutex::new(Vec::new())),
            cdp_network_active: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            intercept_rules: Arc::new(TokioMutex::new(Vec::new())),
            active_workflow: None,
        })
    }

    /// Connect to user's running Chrome (reads DevToolsActivePort).
    pub async fn connect_running() -> Result<Self, Box<dyn std::error::Error>> {
        let (port, _ws_path) =
            find_debug_port().ok_or("Chrome not running with --remote-debugging-port")?;
        Self::connect_port(port).await
    }

    /// Check if the CDP connection is still alive.
    /// Apply stealth after Cloudflare/Turnstile has passed.
    /// Call this AFTER the first navigation, not during launch.
    pub async fn apply_stealth(&self) -> Result<(), Box<dyn std::error::Error>> {
        let identity = crate::identity::BrowserIdentity::random();
        eprintln!("[ENGINE] Applying stealth: {identity}");

        // Override UA
        network::set_user_agent_override(
            &self.scoped_main(),
            &identity.user_agent,
            Some(&identity.accept_language),
            Some(identity.platform_str()),
        ).await.map_err(cdp_err)?;

        // Inject stealth for future navigations
        page::add_script_to_evaluate_on_new_document(
            &self.scoped_main(),
            &identity.to_stealth_js(),
            None,
        ).await.map_err(cdp_err)?;

        // Apply to current page too
        runtime::evaluate(
            &self.scoped_main(),
            runtime::EvaluateParams {
                expression: identity.to_stealth_js(),
                return_by_value: Some(true),
                ..runtime::EvaluateParams::new("")
            },
        ).await.ok();

        // Pre-inject cookie banner killer for all future navigations
        page::add_script_to_evaluate_on_new_document(
            &self.scoped_main(),
            r#"
                // Set cookie consent BEFORE CookieYes SDK loads
                document.cookie='cookieyes-consent=consentid:neo,consent:yes,action:yes,necessary:yes,analytics:yes,advertisement:yes,other:yes;path=/;max-age=31536000';
                // MutationObserver: remove cookie banners as they appear
                new MutationObserver((muts) => {
                    for (const m of muts) for (const n of m.addedNodes) {
                        if (n.nodeType === 1) {
                            const cl = (typeof n.className === 'string') ? n.className : '';
                            const id = n.id || '';
                            if (cl.includes('cky-') || cl.includes('cc-banner') || cl.includes('cookie-consent') ||
                                id.includes('cky-') || n.getAttribute?.('data-cky-tag')) {
                                n.remove();
                                document.body?.classList.remove('cky-modal-open');
                                if (document.body) document.body.style.overflow = '';
                                if (document.documentElement) document.documentElement.style.overflow = '';
                            }
                        }
                    }
                }).observe(document.documentElement, {childList: true, subtree: true});
            "#,
            None,
        ).await.ok();

        // Block known cookie consent scripts at network level
        network::set_blocked_urls(
            &self.scoped_main(),
            &[
                "*cookieyes.com*",
                "*cookie-law*",
                "*cookie-consent*",
                "*cookiebot.com*",
                "*onetrust.com*",
                "*termly.io/api/v1/snippets*",
                "*cdn.iubenda.com/cs*",
            ],
        ).await.ok();

        eprintln!("[ENGINE] Stealth applied");
        Ok(())
    }

    // ─── Auto-dismiss cookie banners ───
    pub async fn dismiss_cookie_banners(&self) -> Result<bool, Box<dyn std::error::Error>> {
        let js = r#"
            (() => {
                // Strategy 1: Click accept/agree buttons by text
                const acceptTexts = [
                    'accept all', 'aceptar todo', 'aceptar todas', 'accept cookies',
                    'acepto', 'aceptar', 'accept', 'agree', 'ok', 'got it', 'entendido',
                    'allow all', 'permitir todo', 'consent', 'continuar', 'continue',
                    'j\'accepte', 'tout accepter', 'akzeptieren', 'alle akzeptieren',
                    'accetta tutto', 'aceitar tudo'
                ];
                const buttons = Array.from(document.querySelectorAll('button, a, [role="button"], span[onclick], div[onclick]'));
                for (const btn of buttons) {
                    const text = (btn.textContent || '').trim().toLowerCase();
                    if (text.length > 50) continue;
                    for (const accept of acceptTexts) {
                        if (text === accept || text.includes(accept)) {
                            btn.click();
                            // Strategy 2: Also remove overlay containers
                            setTimeout(() => {
                                document.querySelectorAll('[class*="cky"], [class*="cookie-banner"], [class*="consent-banner"], [id*="cookie"], [id*="consent"], [class*="CookieConsent"], [class*="cc-banner"]').forEach(el => el.remove());
                                document.body.style.overflow = '';
                                document.documentElement.style.overflow = '';
                            }, 500);
                            return 'dismissed: ' + text;
                        }
                    }
                }
                // Strategy 3: No button found, try to remove overlays directly
                let removed = 0;
                document.querySelectorAll('[class*="cky"], [class*="cookie"], [class*="consent"], [id*="cookie"], [id*="consent"], [class*="CookieConsent"], [class*="cc-banner"], [class*="gdpr"]').forEach(el => {
                    if (el.textContent && el.textContent.toLowerCase().includes('cookie')) {
                        el.remove();
                        removed++;
                    }
                });
                if (removed > 0) {
                    document.body.style.overflow = '';
                    document.documentElement.style.overflow = '';
                    return 'removed: ' + removed + ' elements';
                }
                return 'none';
            })()
        "#;
        let result = self.eval_string(js).await.unwrap_or_default();
        if result != "none" {
            eprintln!("[ENGINE] Cookie banner: {result}");
            // Wait for any animations/transitions
            tokio::time::sleep(std::time::Duration::from_millis(600)).await;
            // Second pass: clean up any remaining overlays
            self.eval_string(r#"
                document.querySelectorAll('[class*="cky"], [class*="cookie"], [class*="consent"], [id*="cookie"]').forEach(el => {
                    if (el.textContent && el.textContent.toLowerCase().includes('cookie')) el.remove();
                });
                document.body.style.overflow = '';
                document.documentElement.style.overflow = '';
            "#).await.ok();
            Ok(true)
        } else {
            Ok(false)
        }
    }

    // ─── PDF ───
    pub async fn pdf(&self, path: Option<&str>) -> Result<String, Box<dyn std::error::Error>> {
        let data = page::print_to_pdf(&self.scoped_main(), page::PrintToPdfParams {
            print_background: Some(true),
            ..Default::default()
        }).await.map_err(cdp_err)?;
        if let Some(path) = path {
            use base64::Engine;
            let bytes = base64::engine::general_purpose::STANDARD.decode(&data)?;
            std::fs::write(path, &bytes)?;
            Ok(format!("PDF saved to {path} ({} bytes)", bytes.len()))
        } else {
            Ok(format!("PDF generated ({} chars base64)", data.len()))
        }
    }

    // ─── Device emulation ───
    pub async fn set_device(&self, width: u32, height: u32, scale: f64, mobile: bool, ua: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
        let t = self.scoped_main();
        emulation::set_device_metrics_override(&t, width as i32, height as i32, scale, mobile, None, None).await.map_err(|e| e as Box<dyn std::error::Error>)?;
        if let Some(ua) = ua {
            emulation::set_user_agent_override(&t, ua, None, None).await.map_err(|e| e as Box<dyn std::error::Error>)?;
        }
        eprintln!("[ENGINE] Device: {width}x{height} @{scale}x mobile={mobile}");
        Ok(())
    }

    // ─── Geolocation ───
    pub async fn set_geolocation(&self, lat: f64, lng: f64, accuracy: Option<f64>) -> Result<(), Box<dyn std::error::Error>> {
        let t = self.scoped_main();
        emulation::set_geolocation_override(&t, Some(lat), Some(lng), Some(accuracy.unwrap_or(100.0))).await.map_err(|e| e as Box<dyn std::error::Error>)?;
        eprintln!("[ENGINE] Geolocation: {lat}, {lng}");
        Ok(())
    }

    // ─── Offline mode ───
    pub async fn set_offline(&self, offline: bool) -> Result<(), Box<dyn std::error::Error>> {
        let t = self.scoped_main();
        let conditions = if offline {
            network::NetworkConditions::offline()
        } else {
            network::NetworkConditions::no_throttle()
        };
        network::emulate_network_conditions(&t, conditions).await.map_err(|e| e as Box<dyn std::error::Error>)?;
        eprintln!("[ENGINE] Offline: {offline}");
        Ok(())
    }

    // ─── Color scheme ───
    pub async fn set_color_scheme(&self, scheme: &str) -> Result<(), Box<dyn std::error::Error>> {
        let t = self.scoped_main();
        let features = vec![emulation::MediaFeature {
            name: "prefers-color-scheme".to_string(),
            value: scheme.to_string(),
        }];
        emulation::set_emulated_media(&t, None, Some(features)).await.map_err(|e| e as Box<dyn std::error::Error>)?;
        eprintln!("[ENGINE] Color scheme: {scheme}");
        Ok(())
    }

    // ─── Drag and drop ───
    pub async fn drag(&self, from_x: f64, from_y: f64, to_x: f64, to_y: f64) -> Result<(), Box<dyn std::error::Error>> {
        let t = self.scoped();
        input::drag(&t, from_x, from_y, to_x, to_y, Some(10)).await.map_err(cdp_err)?;
        eprintln!("[ENGINE] Drag ({from_x},{from_y}) → ({to_x},{to_y})");
        Ok(())
    }

    // ─── Upload file ───
    pub async fn upload_file(&self, selector: &str, paths: &[String]) -> Result<(), Box<dyn std::error::Error>> {
        let t = self.scoped();
        // Find the file input element via DOM
        let doc_node = dom::get_document(&t, None, None).await.map_err(cdp_err)?;
        let node_id = dom::query_selector(&t, doc_node.node_id, selector).await.map_err(cdp_err)?;
        if node_id == 0 {
            return Err(format!("File input not found: {selector}").into());
        }
        // Set files
        let path_strs: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
        dom::set_file_input_files(&t, &path_strs, Some(node_id), None, None).await.map_err(cdp_err)?;
        eprintln!("[ENGINE] Upload {} files to {selector}", paths.len());
        Ok(())
    }

    // ─── Clipboard ───
    pub async fn clipboard_read(&self) -> Result<String, Box<dyn std::error::Error>> {
        let result = self.eval_string("navigator.clipboard.readText()").await?;
        Ok(result)
    }

    pub async fn clipboard_write(&self, text: &str) -> Result<(), Box<dyn std::error::Error>> {
        let escaped = text.replace('\\', "\\\\").replace('\'', "\\'").replace('\n', "\\n");
        self.eval_string(&format!("navigator.clipboard.writeText('{escaped}')")).await?;
        Ok(())
    }

    // ─── Mouse fine control ───
    pub async fn mouse_move(&self, x: f64, y: f64) -> Result<(), Box<dyn std::error::Error>> {
        let t = self.scoped();
        input::mouse_move(&t, x, y).await.map_err(cdp_err)?;
        Ok(())
    }

    pub async fn mouse_down(&self, x: f64, y: f64, button: &str) -> Result<(), Box<dyn std::error::Error>> {
        let t = self.scoped();
        input::dispatch_mouse_event(&t, input::DispatchMouseEventParams {
            type_: "mousePressed".to_string(), x, y,
            button: Some(button.to_string()), click_count: Some(1),
            ..Default::default()
        }).await.map_err(cdp_err)?;
        Ok(())
    }

    pub async fn mouse_up(&self, x: f64, y: f64, button: &str) -> Result<(), Box<dyn std::error::Error>> {
        let t = self.scoped();
        input::dispatch_mouse_event(&t, input::DispatchMouseEventParams {
            type_: "mouseReleased".to_string(), x, y,
            button: Some(button.to_string()), click_count: Some(1),
            ..Default::default()
        }).await.map_err(cdp_err)?;
        Ok(())
    }

    pub async fn mouse_wheel(&self, x: f64, y: f64, delta_x: f64, delta_y: f64) -> Result<(), Box<dyn std::error::Error>> {
        let t = self.scoped();
        input::scroll(&t, x, y, delta_x, delta_y).await.map_err(cdp_err)?;
        Ok(())
    }

    // ─── Highlight element ───
    pub async fn highlight(&self, selector: &str) -> Result<(), Box<dyn std::error::Error>> {
        let js = format!(
            r#"(() => {{
                const el = document.querySelector('{}');
                if (!el) return 'not_found';
                el.style.outline = '3px solid red';
                el.style.outlineOffset = '2px';
                return 'highlighted';
            }})()"#,
            selector.replace('\'', "\\'")
        );
        self.eval_string(&js).await?;
        Ok(())
    }

    // ─── Get element info ───
    pub async fn get_element_info(&self, selector: &str, what: &str) -> Result<String, Box<dyn std::error::Error>> {
        let js = match what {
            "text" => format!("document.querySelector('{}')?.innerText || ''", selector.replace('\'', "\\'")),
            "html" => format!("document.querySelector('{}')?.innerHTML || ''", selector.replace('\'', "\\'")),
            "value" => format!("document.querySelector('{}')?.value || ''", selector.replace('\'', "\\'")),
            "box" => format!(
                "JSON.stringify(document.querySelector('{}')?.getBoundingClientRect())",
                selector.replace('\'', "\\'")
            ),
            "styles" => format!(
                "JSON.stringify(Object.fromEntries([...getComputedStyle(document.querySelector('{}'))].map(p => [p, getComputedStyle(document.querySelector('{}')).getPropertyValue(p)]).slice(0, 20)))",
                selector.replace('\'', "\\'"), selector.replace('\'', "\\'")
            ),
            "count" => format!(
                "document.querySelectorAll('{}').length.toString()",
                selector.replace('\'', "\\'")
            ),
            _ => format!("document.querySelector('{}')?.getAttribute('{}') || ''",
                selector.replace('\'', "\\'"), what.replace('\'', "\\'")),
        };
        self.eval_string(&js).await
    }

    // ─── Screenshot annotated ───
    pub async fn screenshot_annotated(&self) -> Result<String, Box<dyn std::error::Error>> {
        // Inject numbered labels on interactive elements, take screenshot, remove labels
        let js = r#"
            (() => {
                const els = document.querySelectorAll('a,button,input,select,textarea,[role="button"],[role="link"],[role="tab"],[onclick]');
                const labels = [];
                let i = 1;
                els.forEach(el => {
                    const rect = el.getBoundingClientRect();
                    if (rect.width === 0 || rect.height === 0) return;
                    if (rect.top < 0 || rect.left < 0) return;
                    const label = document.createElement('div');
                    label.className = '_neo_label_';
                    label.textContent = i;
                    label.style.cssText = `position:fixed;top:${rect.top-10}px;left:${rect.left-10}px;z-index:999999;background:red;color:white;font-size:10px;font-weight:bold;padding:1px 4px;border-radius:8px;pointer-events:none;`;
                    document.body.appendChild(label);
                    labels.push({i, tag: el.tagName, text: (el.textContent||'').trim().substring(0,30), x: Math.round(rect.x), y: Math.round(rect.y)});
                    i++;
                });
                return JSON.stringify({count: i-1, labels});
            })()
        "#;
        let legend = self.eval_string(js).await?;

        // Take screenshot
        let screenshot = page::capture_screenshot(
            &self.scoped_main(),
            page::ScreenshotParams {
                format: Some("jpeg".into()),
                quality: Some(60),
                ..Default::default()
            },
        ).await.map_err(cdp_err)?;
        let b64 = &screenshot.data;

        // Remove labels
        self.eval_string("document.querySelectorAll('._neo_label_').forEach(el => el.remove())").await.ok();

        Ok(serde_json::json!({
            "screenshot_base64": b64,
            "legend": serde_json::from_str::<serde_json::Value>(&legend).unwrap_or_default(),
        }).to_string())
    }

    pub fn is_alive(&self) -> bool {
        self.cdp.is_alive()
    }

    /// Get a ScopedTransport bound to the active page session.
    /// This routes all CDP calls through the correct session_id.
    fn scoped(&self) -> ScopedTransport<'_> {
        let sid = self.active_frame_session_id.as_deref()
            .unwrap_or(&self.page_session_id);
        ScopedTransport::new(&self.cdp, sid)
    }

    /// Get a ScopedTransport bound to the main page session (ignoring frame override).
    fn scoped_main(&self) -> ScopedTransport<'_> {
        ScopedTransport::new(&self.cdp, &self.page_session_id)
    }

    // ─── Frame-aware helpers ───

    /// JS snippet that resolves the "active document" — the frame with real content.
    /// In a frameset, the top-level document is just <frameset> with no interactive elements.
    /// This finds the largest same-origin frame and returns its document.
    /// Outside framesets, returns the normal document.
    const ACTIVE_DOC_JS: &'static str = r#"
        (function() {
            // Quick check: if top document has interactive content, use it
            var topCount = document.querySelectorAll('a,button,input,select,textarea').length;
            if (topCount > 5 || !document.querySelector('frameset, iframe')) return document;

            // Frameset detected with sparse top content — find the best frame
            var best = null, bestScore = 0;
            var frames = document.querySelectorAll('frame, iframe');
            for (var i = 0; i < frames.length; i++) {
                try {
                    var doc = frames[i].contentDocument;
                    if (!doc || !doc.body) continue;
                    var score = doc.querySelectorAll('a,button,input,select,textarea').length;
                    // Also consider text length for content-heavy frames
                    score += Math.min(doc.body.innerText.length / 100, 50);
                    if (score > bestScore) { bestScore = score; best = doc; }
                } catch(e) { /* cross-origin frame, skip */ }
            }
            return best || document;
        })()
    "#;

    /// Wraps a JS expression to run against the active document (frame-aware).
    /// The expression receives `_doc` as the resolved document.
    #[allow(dead_code)]
    fn frame_aware_js(js_using_doc: &str) -> String {
        format!(
            r#"(() => {{ const _doc = {active_doc}; {body} }})()"#,
            active_doc = Self::ACTIVE_DOC_JS,
            body = js_using_doc,
        )
    }

    // ─── CDP helpers ───

    /// Evaluate JS in the page (or active frame), return the result as string.
    /// When a frame is active, automatically routes to that frame's context.
    pub async fn eval_string(&self, expression: &str) -> Result<String, Box<dyn std::error::Error>> {
        // If a frame is active, delegate to frame-aware eval
        if self.active_frame_id.is_some() {
            return self.eval_in_active_frame(expression).await;
        }

        let result = self
            .cdp
            .send_to(
                &self.page_session_id,
                "Runtime.evaluate",
                Some(json!({
                    "expression": expression,
                    "returnByValue": true,
                    "awaitPromise": true,
                })),
            )
            .await?;

        if let Some(exception) = result.get("exceptionDetails") {
            let text = exception
                .get("text")
                .and_then(|t| t.as_str())
                .unwrap_or("JS exception");
            return Err(text.to_string().into());
        }

        let value = &result["result"]["value"];
        match value {
            Value::String(s) => Ok(s.clone()),
            Value::Null => Ok(String::new()),
            other => Ok(other.to_string()),
        }
    }

    /// Get the current page URL.
    async fn current_url(&self) -> Result<String, Box<dyn std::error::Error>> {
        self.eval_string("window.location.href").await
    }

    /// Get the current page title.
    async fn current_title(&self) -> Result<String, Box<dyn std::error::Error>> {
        self.eval_string("document.title || ''").await
    }

    // ─── Cookies ───

    pub async fn load_cookies(&self, path: &str) -> Result<usize, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        let data: Value = serde_json::from_str(&content)?;

        let cookies = if let Some(arr) = data.as_array() {
            arr.clone()
        } else if let Some(arr) = data.get("cookies").and_then(|c| c.as_array()) {
            arr.clone()
        } else {
            return Err("Invalid cookie format".into());
        };

        let count = cookies.len();

        // 30 days from now — ensures Chrome persists cookies to disk
        let expires_default = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64()
            + 30.0 * 86400.0;

        // Build CDP cookies array
        let cdp_cookies: Vec<Value> = cookies
            .iter()
            .filter_map(|c| {
                let name = c.get("name")?.as_str()?;
                let value = c.get("value")?.as_str()?;
                let domain = c.get("domain").and_then(|v| v.as_str()).unwrap_or("");
                let path = c.get("path").and_then(|v| v.as_str()).unwrap_or("/");
                let secure = c.get("secure").and_then(|v| v.as_bool()).unwrap_or(false);
                let http_only = c.get("httpOnly").and_then(|v| v.as_bool()).unwrap_or(false);

                // Use provided expiry, or force 30 days so Chrome persists them
                let expires = c.get("expires")
                    .and_then(|v| v.as_f64())
                    .filter(|&e| e > 1.0)
                    .unwrap_or(expires_default);

                let scheme = if secure { "https" } else { "http" };
                let clean = domain.trim_start_matches('.');
                let url = format!("{scheme}://{clean}/");

                Some(json!({
                    "name": name,
                    "value": value,
                    "domain": domain,
                    "path": path,
                    "secure": secure,
                    "httpOnly": http_only,
                    "url": url,
                    "expires": expires,
                }))
            })
            .collect();

        // Network.setCookies for the current session — convert to typed CookieParam
        let typed_cookies: Vec<network::CookieParam> = cdp_cookies
            .iter()
            .filter_map(|c| serde_json::from_value(c.clone()).ok())
            .collect();
        network::set_cookies(&self.scoped_main(), typed_cookies).await.map_err(cdp_err)?;

        eprintln!("[ENGINE] Injected {count} cookies");
        Ok(count)
    }

    // ─── Navigation ───

    pub async fn goto(&mut self, url: &str) -> Result<(), Box<dyn std::error::Error>> {
        let t0 = Instant::now();

        // Page.navigate — just sends the command, does NOT wait for lifecycle.
        // This is the key difference from chromiumoxide: no hanging.
        // Note: page::navigate already checks errorText and returns Err if present.
        page::navigate(&self.scoped_main(), page::NavigateParams {
            url: url.to_string(),
            ..Default::default()
        }).await.map_err(cdp_err)?;

        // Wait for content to be usable — poll until DOM has interactive elements.
        // This replaces both the lifecycle wait AND the SPA wait.
        self.wait_for_content(5000).await;

        self.last_url = url.to_string();
        eprintln!("[ENGINE] goto {url} ({}ms)", t0.elapsed().as_millis());
        Ok(())
    }

    /// Wait until the page has enough interactive content to be useful.
    async fn wait_for_content(&self, max_ms: u64) {
        let js = r#"
            (() => {
                const count = document.querySelectorAll('a,button,input,select,textarea,[role]').length;
                const textLen = (document.body ? document.body.innerText.length : 0);
                return JSON.stringify({count, textLen});
            })()
        "#;

        let mut prev_count = -1i64;
        let mut stable = 0;
        let mut elapsed = 0u64;
        let interval = 300u64;

        // Initial wait for frameworks to bootstrap
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        elapsed += 500;

        while elapsed < max_ms {
            if let Ok(result) = self.eval_string(js).await {
                if let Ok(data) = serde_json::from_str::<Value>(&result) {
                    let count = data["count"].as_i64().unwrap_or(0);
                    let text_len = data["textLen"].as_i64().unwrap_or(0);

                    if count > 3 && count == prev_count && text_len > 30 {
                        return; // Content stabilized
                    }
                    if stable >= 2 {
                        return; // Stable enough
                    }
                    if count == prev_count {
                        stable += 1;
                    } else {
                        stable = 0;
                    }
                    prev_count = count;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(interval)).await;
            elapsed += interval;
        }
    }

    pub async fn back(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.eval_string("window.history.back()").await?;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        eprintln!("[ENGINE] back");
        Ok(())
    }

    pub async fn forward(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.eval_string("window.history.forward()").await?;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        eprintln!("[ENGINE] forward");
        Ok(())
    }

    pub async fn reload(&self) -> Result<(), Box<dyn std::error::Error>> {
        page::reload(&self.scoped_main(), None, None).await.map_err(cdp_err)?;
        self.wait_for_content(5000).await;
        eprintln!("[ENGINE] reload");
        Ok(())
    }

    // ─── Perception ───

    /// Tag interactive DOM elements with data-wom-id so act() can find them
    /// by WOM ID directly. This runs after each observe/WOM build.
    /// Tag DOM elements using the exact WOM node IDs.
    /// Walks the DOM in the same order as the WOM builder, assigning matching IDs.
    async fn tag_dom_elements_from_wom(&self, doc: &wom::WomDocument) -> Result<u64, Box<dyn std::error::Error>> {
        // Strategy: clear old tags, then use the same traversal order as WOM builder.
        // Since WOM builder walks document order (skipping nav/footer for links),
        // we replicate that in JS with the same prefix counters.
        let js = r#"
            (() => {
                // Remove old tags
                document.querySelectorAll('[data-wom-id]').forEach(el => el.removeAttribute('data-wom-id'));

                let hC = 0, lnkC = 0, btnC = 0, fldC = 0, selC = 0, imgC = 0, pC = 0;
                const pad = (n) => String(n).padStart(3, '0');

                function isInNavOrFooter(el) {
                    let p = el.parentElement;
                    while (p) {
                        const tag = p.tagName?.toLowerCase();
                        if (tag === 'nav' || tag === 'footer') return true;
                        p = p.parentElement;
                    }
                    return false;
                }

                // Walk DOM in document order (same as WOM builder)
                const walker = document.createTreeWalker(
                    document.documentElement,
                    NodeFilter.SHOW_ELEMENT,
                    null
                );

                let el;
                while (el = walker.nextNode()) {
                    const tag = el.tagName?.toLowerCase();
                    if (!tag) continue;

                    // Headings
                    if (/^h[1-6]$/.test(tag)) {
                        const t = (el.textContent || '').trim();
                        if (t) el.setAttribute('data-wom-id', 'h_' + pad(++hC));
                        continue;
                    }

                    // Links (skip nav/footer — matches WOM builder)
                    if (tag === 'a') {
                        const t = (el.textContent || '').trim();
                        if (t && !isInNavOrFooter(el)) {
                            el.setAttribute('data-wom-id', 'lnk_' + pad(++lnkC));
                        }
                        continue;
                    }

                    // Buttons, summary, role=button
                    if (tag === 'button' || tag === 'summary' || el.getAttribute('role') === 'button') {
                        const t = (el.textContent || el.value || '').trim();
                        if (t && t.length < 120) el.setAttribute('data-wom-id', 'btn_' + pad(++btnC));
                        continue;
                    }

                    // Contenteditable or role=textbox (BEFORE input check)
                    if (el.getAttribute('contenteditable') === 'true' || el.getAttribute('role') === 'textbox') {
                        el.setAttribute('data-wom-id', 'fld_' + pad(++fldC));
                        continue;
                    }

                    // Inputs and textareas
                    if (tag === 'input' || tag === 'textarea') {
                        const itype = (el.type || 'text').toLowerCase();
                        if (itype === 'hidden') continue;
                        if (itype === 'submit') {
                            el.setAttribute('data-wom-id', 'btn_' + pad(++btnC));
                        } else {
                            el.setAttribute('data-wom-id', 'fld_' + pad(++fldC));
                        }
                        continue;
                    }

                    // Selects
                    if (tag === 'select') {
                        el.setAttribute('data-wom-id', 'sel_' + pad(++selC));
                        continue;
                    }

                    // Forms
                    if (tag === 'form') {
                        // Don't skip children — let walker continue
                    }
                }

                return String(document.querySelectorAll('[data-wom-id]').length);
            })()
        "#;
        let result = self.eval_string(js).await?;
        let count = result.parse::<u64>().unwrap_or(0);
        Ok(count)
    }

    #[allow(dead_code)]
    async fn tag_dom_elements(&self) -> Result<u64, Box<dyn std::error::Error>> {
        let js = format!(
            r#"
            (() => {{
                const _doc = {active_doc};

                // Remove old tags
                _doc.querySelectorAll('[data-wom-id]').forEach(el => el.removeAttribute('data-wom-id'));

                let btnC = 0, lnkC = 0, fldC = 0, hC = 0, selC = 0, imgC = 0;
                const pad = (n) => String(n).padStart(3, '0');

                // Tag headings
                _doc.querySelectorAll('h1,h2,h3,h4,h5,h6').forEach(el => {{
                    const t = (el.textContent || '').trim();
                    if (t) el.setAttribute('data-wom-id', 'h_' + pad(++hC));
                }});

                // Tag links (skip nav/footer to match WOM builder)
                _doc.querySelectorAll('a').forEach(el => {{
                    const t = (el.textContent || '').trim();
                    if (t) el.setAttribute('data-wom-id', 'lnk_' + pad(++lnkC));
                }});

                // Tag buttons
                _doc.querySelectorAll('button, summary, [role="button"], input[type="submit"]').forEach(el => {{
                    const t = (el.textContent || el.value || '').trim();
                    if (t && t.length < 120) el.setAttribute('data-wom-id', 'btn_' + pad(++btnC));
                }});

                // Tag inputs
                _doc.querySelectorAll('input:not([type="hidden"]):not([type="submit"]), textarea, [contenteditable="true"], [role="textbox"], [role="combobox"], [role="searchbox"]').forEach(el => {{
                    el.setAttribute('data-wom-id', 'fld_' + pad(++fldC));
                }});

                // Tag selects
                _doc.querySelectorAll('select').forEach(el => {{
                    el.setAttribute('data-wom-id', 'sel_' + pad(++selC));
                }});

                // Tag checkboxes/radios
                _doc.querySelectorAll('input[type="checkbox"], input[type="radio"]').forEach(el => {{
                    el.setAttribute('data-wom-id', 'fld_' + pad(++fldC));
                }});

                const total = btnC + lnkC + fldC + hC + selC;
                return String(total);
            }})()
            "#,
            active_doc = Self::ACTIVE_DOC_JS,
        );
        let result = self.eval_string(&js).await?;
        let count = result.parse::<u64>().unwrap_or(0);
        Ok(count)
    }

    /// Click an element by its WOM ID (data-wom-id attribute).
    /// Returns true if found and clicked.
    pub async fn click_by_wom_id(&self, wom_id: &str) -> Result<bool, Box<dyn std::error::Error>> {
        let js = format!(
            r#"
            (() => {{
                const _doc = {active_doc};
                const el = _doc.querySelector('[data-wom-id="{wom_id}"]');
                if (!el) return 'not_found';
                el.scrollIntoViewIfNeeded();
                el.click();
                const t = (el.textContent || el.value || el.getAttribute('aria-label') || '').trim();
                return 'clicked[' + '{wom_id}' + ']: ' + t.substring(0, 60);
            }})()
            "#,
            active_doc = Self::ACTIVE_DOC_JS,
            wom_id = wom_id
        );
        let result = self.eval_string(&js).await?;
        let found = !result.starts_with("not_found");
        if found {
            eprintln!("[ENGINE] {result}");
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        } else {
            eprintln!("[ENGINE] wom_id not found: '{wom_id}'");
        }
        Ok(found)
    }

    /// Focus an element by its WOM ID.
    pub async fn focus_by_wom_id(&self, wom_id: &str) -> Result<bool, Box<dyn std::error::Error>> {
        let js = format!(
            r#"
            (() => {{
                const _doc = {active_doc};
                const el = _doc.querySelector('[data-wom-id="{wom_id}"]');
                if (!el) return 'not_found';
                el.scrollIntoViewIfNeeded();
                el.focus();
                el.click();
                const label = el.placeholder || el.getAttribute('aria-label') || el.tagName;
                return 'focused[' + '{wom_id}' + ']: ' + label;
            }})()
            "#,
            active_doc = Self::ACTIVE_DOC_JS,
            wom_id = wom_id
        );
        let result = self.eval_string(&js).await?;
        let found = !result.starts_with("not_found");
        if found {
            eprintln!("[ENGINE] {result}");
        } else {
            eprintln!("[ENGINE] wom_id not found for focus: '{wom_id}'");
        }
        Ok(found)
    }

    /// Capture outerHTML with SPA stability wait.
    async fn capture_html(&self) -> Result<String, Box<dyn std::error::Error>> {
        // Frame-aware: capture HTML from the active document (frame with most content).
        // In a frameset, the top document is just <frameset> tags — we want the frame content.
        let js = format!(
            r#"
            new Promise((resolve) => {{
                const _doc = {active_doc};
                let prev = -1, stable = 0, elapsed = 0;
                const check = () => {{
                    const count = _doc.querySelectorAll('a,button,input,select,textarea,[role]').length;
                    const textLen = (_doc.body ? _doc.body.innerText.length : 0);
                    elapsed += 300;
                    if ((count > 3 && count === prev && textLen > 30) || stable >= 2 || elapsed > 5000) {{
                        resolve(_doc.documentElement.outerHTML);
                    }} else {{
                        if (count === prev) {{ stable++; }} else {{ stable = 0; }}
                        prev = count;
                        setTimeout(check, 300);
                    }}
                }};
                setTimeout(check, 500);
            }})
            "#,
            active_doc = Self::ACTIVE_DOC_JS,
        );
        self.eval_string(&js).await
    }

    /// Raw semantic dump.
    pub async fn see_raw(&self) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let t0 = Instant::now();
        let html = self.capture_html().await?;

        let dom = parse_document(RcDom::default(), Default::default())
            .from_utf8()
            .read_from(&mut html.as_bytes())?;

        let mut output = Vec::new();
        semantic::walk(&dom.document, 0, &mut output);

        let mut stats = semantic::PageStats::new();
        semantic::count_nodes(&dom.document, &mut stats);

        eprintln!(
            "[ENGINE] see: {} lines, {:.1}KB | {}L {}B {}F {}H | {}ms",
            output.len(),
            output.join("\n").len() as f64 / 1024.0,
            stats.links, stats.buttons, stats.forms, stats.headings,
            t0.elapsed().as_millis(),
        );

        Ok(output)
    }

    /// AI Vision — semantic dump + page classification + available actions.
    pub async fn see(&self) -> Result<vision::PageView, Box<dyn std::error::Error>> {
        let t0 = Instant::now();
        let html = self.capture_html().await?;
        let url = self.current_url().await.unwrap_or_default();
        let title = self.current_title().await.unwrap_or_default();

        let dom = parse_document(RcDom::default(), Default::default())
            .from_utf8()
            .read_from(&mut html.as_bytes())?;

        let view = vision::analyze(&dom.document, &url, &title);

        eprintln!(
            "[VISION] {} | {} | {} lines | {} actions | {}ms",
            view.page_type, title, view.content.len(), view.actions.len(),
            t0.elapsed().as_millis(),
        );

        Ok(view)
    }

    /// Lightweight page view — extracts what a user SEES via JS.
    /// No HTML parsing, no WOM, no html5ever. Just runs JS in the active frame
    /// and returns structured text: title, URL, interactive elements, visible text.
    /// This is what agents should use 90% of the time.
    pub async fn see_page(&self) -> Result<String, Box<dyn std::error::Error>> {
        let t0 = Instant::now();
        // When inside a frame, use document directly (already in frame context).
        // On main page, use ACTIVE_DOC_JS to auto-detect best frame.
        let doc_expr = if self.active_frame_id.is_some() { "document" } else { Self::ACTIVE_DOC_JS };
        let js = format!(
            r#"
            (() => {{
                const _doc = {active_doc};
                const url = window.location.href;
                const title = _doc.title || document.title || '';

                // Collect interactive elements (visible only)
                const els = [];
                _doc.querySelectorAll('input, textarea, select, button, a, [role="button"], [role="link"], summary, [onclick]').forEach(el => {{
                    // Skip hidden
                    if (el.offsetParent === null && el.tagName !== 'BODY' && el.tagName !== 'HTML'
                        && !el.closest('frame, iframe')) return;

                    const tag = el.tagName.toLowerCase();
                    const text = (el.textContent || '').trim().substring(0, 80);
                    const placeholder = el.placeholder || '';
                    const name = el.name || el.id || '';
                    const type = el.type || '';
                    const value = el.value || '';
                    const ariaLabel = el.getAttribute('aria-label') || '';
                    const title = el.title || '';
                    const href = el.href || '';

                    // Build label
                    let label = ariaLabel || placeholder || text || name || title;
                    if (!label && tag === 'input') label = type;
                    if (!label) return;
                    label = label.replace(/\s+/g, ' ').trim().substring(0, 60);

                    let entry = '';
                    if (tag === 'input' || tag === 'textarea') {{
                        const t = (type === 'password') ? 'password' : (type === 'hidden') ? null : 'input';
                        if (!t) return;
                        const cur = value ? ' = "' + value.substring(0, 30) + '"' : '';
                        entry = '[' + t + ' "' + label + '"]' + cur;
                    }} else if (tag === 'select') {{
                        const opts = Array.from(el.options).slice(0, 8).map(o => o.text.trim().substring(0, 30));
                        const cur = el.selectedIndex >= 0 ? el.options[el.selectedIndex].text.trim() : '';
                        entry = '[select "' + label + '"] selected="' + cur + '" options: ' + opts.join(' | ');
                    }} else if (tag === 'button' || el.getAttribute('role') === 'button') {{
                        entry = '[button "' + label + '"]';
                    }} else if (tag === 'a') {{
                        if (!text || text.length < 2) return;
                        entry = '[link "' + label + '"]';
                    }} else if (tag === 'summary') {{
                        entry = '[toggle "' + label + '"]';
                    }} else {{
                        entry = '[action "' + label + '"]';
                    }}

                    els.push(entry);
                }});

                // Deduplicate
                const seen = new Set();
                const unique = els.filter(e => {{
                    if (seen.has(e)) return false;
                    seen.add(e);
                    return true;
                }});

                // Extract visible text (headings + paragraphs, skip nav/footer noise)
                const textParts = [];
                let inNav = false;
                _doc.querySelectorAll('h1,h2,h3,h4,h5,h6,p,td,th,li,label,span,div').forEach(el => {{
                    // Skip if inside nav/header/footer
                    const parent = el.closest('nav, header, footer, [role="navigation"], [role="banner"]');
                    if (parent) return;

                    // Skip hidden
                    if (el.offsetParent === null && el.tagName !== 'BODY') return;

                    const tag = el.tagName.toLowerCase();
                    let t = '';

                    // For headings, get direct text
                    if (tag.startsWith('h')) {{
                        t = el.textContent.trim();
                        if (t) t = '#'.repeat(parseInt(tag[1])) + ' ' + t;
                    }} else if (tag === 'p' || tag === 'label') {{
                        t = el.textContent.trim();
                    }} else if (tag === 'td' || tag === 'th') {{
                        t = el.textContent.trim();
                    }} else if (tag === 'li') {{
                        // Only direct text, not nested lists
                        const directText = Array.from(el.childNodes)
                            .filter(n => n.nodeType === 3)
                            .map(n => n.textContent.trim())
                            .join(' ');
                        if (directText) t = '- ' + directText;
                    }} else if (tag === 'div' || tag === 'span') {{
                        // Only leaf divs/spans with meaningful text
                        if (el.children.length === 0) {{
                            t = el.textContent.trim();
                        }}
                    }}

                    if (t && t.length > 2 && t.length < 2000) {{
                        textParts.push(t.substring(0, 1000));
                    }}
                }});

                // Deduplicate text (many nested elements repeat text)
                const seenText = new Set();
                const uniqueText = textParts.filter(t => {{
                    const key = t.substring(0, 120).toLowerCase();
                    if (seenText.has(key)) return false;
                    seenText.add(key);
                    return true;
                }});

                // Build output
                const lines = [];
                lines.push('Page: ' + title);
                lines.push('URL: ' + url);
                lines.push('');

                if (unique.length > 0) {{
                    lines.push('Interactive:');
                    unique.forEach(e => lines.push('  ' + e));
                    lines.push('');
                }}

                if (uniqueText.length > 0) {{
                    lines.push('Content:');
                    uniqueText.slice(0, 200).forEach(t => lines.push('  ' + t));
                    if (uniqueText.length > 80) {{
                        lines.push('  ... (' + (uniqueText.length - 80) + ' more)');
                    }}
                }}

                return lines.join('\n');
            }})()
            "#,
            active_doc = doc_expr,
        );

        let mut result = self.eval_string(&js).await?;

        // When viewing inside a frame, prepend frame context
        if let Some(ref fid) = self.active_frame_id {
            let mode = if self.active_frame_session_id.is_some() { "OOP" } else { "same-process" };
            result = format!("[FRAME: {} ({})]\n{}", fid, mode, result);
        }

        eprintln!(
            "[SEE] {}chars | {}ms",
            result.len(),
            t0.elapsed().as_millis(),
        );

        Ok(result)
    }

    /// WOM output — structured for AI agents.
    pub async fn see_wom(&self, revision: u64) -> Result<wom::WomDocument, Box<dyn std::error::Error>> {
        let t0 = Instant::now();
        let html = self.capture_html().await?;
        eprintln!("[WOM] captured {}KB ({}ms)", html.len() / 1024, t0.elapsed().as_millis());

        let url = self.current_url().await.unwrap_or_default();
        let title = self.current_title().await.unwrap_or_default();
        let html_bytes = html.len();

        let dom = parse_document(RcDom::default(), Default::default())
            .from_utf8()
            .read_from(&mut html.as_bytes())?;

        // Tag DOM elements FIRST, then capture HTML so WOM builder can read the IDs
        let tagged = self.tag_dom_elements().await.unwrap_or(0);

        // Re-capture HTML with tags embedded
        let html = self.capture_html().await?;
        let html_bytes = html.len();
        let dom = parse_document(RcDom::default(), Default::default())
            .from_utf8()
            .read_from(&mut html.as_bytes())?;

        let doc = wom::build(&dom.document, &url, &title, html_bytes, "engine", revision);

        eprintln!(
            "[WOM] {} | {} nodes | {} actions | {} tagged | {:.1}x compression | {}ms",
            doc.page.page_class,
            doc.nodes.len(),
            doc.actions.len(),
            tagged,
            doc.compression.compression_ratio,
            t0.elapsed().as_millis(),
        );

        Ok(doc)
    }

    // ─── Interaction ───

    pub async fn click(&self, text: &str) -> Result<bool, Box<dyn std::error::Error>> {
        // Smart click: collect ALL candidates, score them, click the best.
        // Frame-aware: searches inside the active frame if in a frameset.
        let js = format!(
            r#"
            (() => {{
                const _doc = {active_doc};
                const target = {target_json};
                const lower = target.toLowerCase();

                // Detect if an element is inside navigation/header
                function inNav(el) {{
                    let p = el;
                    while (p) {{
                        const tag = (p.tagName || '').toLowerCase();
                        const role = (p.getAttribute && p.getAttribute('role') || '').toLowerCase();
                        if (tag === 'nav' || tag === 'header' || role === 'navigation' || role === 'banner') return true;
                        p = p.parentElement;
                    }}
                    return false;
                }}

                // Collect candidates with scores
                const candidates = [];
                for (const el of _doc.querySelectorAll('button, a, [role="button"], [role="link"], [role="menuitem"], [role="option"], [role="tab"], input[type="submit"], summary, [aria-label], [title], [onclick], [data-testid], [class*="cursor-pointer"], [tabindex="0"]')) {{
                    // Get the best text representation
                    const texts = [
                        (el.textContent || '').trim(),
                        el.value || '',
                        el.getAttribute('aria-label') || '',
                        el.getAttribute('title') || '',
                    ];
                    // Find the text that best matches our target
                    let bestText = '';
                    let bestScore = -1;
                    for (const t of texts) {{
                        if (!t) continue;
                        const tl = t.toLowerCase();
                        if (!tl.includes(lower)) continue;

                        let score = 0;
                        // Exact match (text IS the target)
                        if (tl === lower) score += 100;
                        // Tight match (target is most of the text)
                        else if (t.length < lower.length * 3) score += 60;
                        // Starts with target
                        else if (tl.startsWith(lower)) score += 40;
                        // Loose partial
                        else score += 10;

                        if (score > bestScore) {{
                            bestScore = score;
                            bestText = t;
                        }}
                    }}

                    if (bestScore < 0) continue;

                    // Bonus: actionable elements (buttons) over links
                    const tag = (el.tagName || '').toLowerCase();
                    const role = (el.getAttribute('role') || '').toLowerCase();
                    if (tag === 'button' || role === 'button' || tag === 'input') bestScore += 15;

                    // Penalty: elements in nav/header (less likely to be the target action)
                    if (inNav(el)) bestScore -= 30;

                    // Penalty: hidden elements
                    if (el.offsetParent === null && tag !== 'body') bestScore -= 50;

                    candidates.push({{el, score: bestScore, text: bestText}});
                }}

                if (candidates.length === 0) return 'not_found';

                // Sort by score descending, pick the best
                candidates.sort((a, b) => b.score - a.score);
                const best = candidates[0];
                best.el.scrollIntoViewIfNeeded();
                best.el.click();

                const alt = candidates.length > 1 ? ' ('+candidates.length+' candidates, 2nd: '+candidates[1].text.substring(0,40)+')' : '';
                return 'clicked[' + best.score + ']: ' + best.text.substring(0, 60) + alt;
            }})()
            "#,
            active_doc = Self::ACTIVE_DOC_JS,
            target_json = serde_json::to_string(text)?
        );

        let result = self.eval_string(&js).await?;
        let found = !result.starts_with("not_found");
        if found {
            eprintln!("[ENGINE] {result}");
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        } else {
            eprintln!("[ENGINE] click not found: '{text}'");
        }
        Ok(found)
    }

    pub async fn focus(&self, text: &str) -> Result<bool, Box<dyn std::error::Error>> {
        let js = format!(
            r#"
            (() => {{
                const _doc = {active_doc};
                const target = {target_json};
                const lower = target.toLowerCase();

                for (const el of _doc.querySelectorAll('input, textarea, [contenteditable="true"], [role="textbox"]')) {{
                    const p = (el.placeholder || el.getAttribute('aria-label') || el.getAttribute('data-placeholder') || '').toLowerCase();
                    if (p.includes(lower) || lower === '') {{
                        el.scrollIntoViewIfNeeded();
                        el.focus();
                        el.click();
                        return 'focused: ' + (el.placeholder || el.getAttribute('aria-label') || el.tagName);
                    }}
                }}

                for (const el of _doc.querySelectorAll('[contenteditable="true"]')) {{
                    el.scrollIntoViewIfNeeded();
                    el.focus();
                    el.click();
                    return 'focused-contenteditable';
                }}

                return 'not_found';
            }})()
            "#,
            active_doc = Self::ACTIVE_DOC_JS,
            target_json = serde_json::to_string(text)?
        );

        let result = self.eval_string(&js).await?;
        let found = !result.starts_with("not_found");
        if found {
            eprintln!("[ENGINE] {result}");
        } else {
            eprintln!("[ENGINE] focus not found: '{text}'");
        }
        Ok(found)
    }

    pub async fn type_text(&self, text: &str) -> Result<(), Box<dyn std::error::Error>> {
        let t = self.scoped_main();
        // Clear existing value first (Ctrl+A then Backspace)
        input::press_key(&t, "a", Some("KeyA"), Some(input::MODIFIER_CTRL)).await.map_err(cdp_err)?;
        input::press_key(&t, "Backspace", Some("Backspace"), None).await.map_err(cdp_err)?;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Use Input.insertText — this triggers InputEvent with inputType='insertText'
        // which is what Vue v-model, React onChange, and Angular ngModel listen to.
        // Much more reliable than individual keyDown/keyUp events.
        input::type_text(&t, text).await.map_err(cdp_err)?;

        // Small delay then dispatch change event via JS for frameworks that need it
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        self.eval_string(&format!(
            r#"(() => {{
                const el = document.activeElement;
                if (el) {{
                    el.dispatchEvent(new Event('input', {{bubbles: true}}));
                    el.dispatchEvent(new Event('change', {{bubbles: true}}));
                }}
            }})()"#
        )).await.ok();

        eprintln!("[ENGINE] typed {} chars (insertText)", text.len());
        Ok(())
    }

    pub async fn press(&self, key: &str) -> Result<(), Box<dyn std::error::Error>> {
        let (key_val, code, vkc) = match key {
            "Enter" | "enter" => ("Enter", "Enter", 13),
            "Tab" | "tab" => ("Tab", "Tab", 9),
            "Escape" | "escape" | "Esc" => ("Escape", "Escape", 27),
            "Backspace" | "backspace" => ("Backspace", "Backspace", 8),
            "Space" | "space" => (" ", "Space", 32),
            "ArrowUp" | "up" => ("ArrowUp", "ArrowUp", 38),
            "ArrowDown" | "down" => ("ArrowDown", "ArrowDown", 40),
            "ArrowLeft" | "left" => ("ArrowLeft", "ArrowLeft", 37),
            "ArrowRight" | "right" => ("ArrowRight", "ArrowRight", 39),
            _ => (key, key, 0),
        };

        let t = self.scoped_main();
        // Use typed dispatch with windowsVirtualKeyCode for proper key handling
        input::dispatch_key_event(&t, input::DispatchKeyEventParams {
            type_: "keyDown".into(),
            key: Some(key_val.into()),
            code: Some(code.into()),
            windows_virtual_key_code: Some(vkc),
            ..Default::default()
        }).await.map_err(cdp_err)?;
        input::dispatch_key_event(&t, input::DispatchKeyEventParams {
            type_: "keyUp".into(),
            key: Some(key_val.into()),
            code: Some(code.into()),
            windows_virtual_key_code: Some(vkc),
            ..Default::default()
        }).await.map_err(cdp_err)?;

        eprintln!("[ENGINE] pressed {key}");
        Ok(())
    }

    /// Send a message via contenteditable input + send button.
    /// Uses execCommand('insertText') which triggers React/framework state updates.
    /// Works with LinkedIn, Slack, Discord, WhatsApp Web, etc.
    pub async fn send_message(
        &self,
        text: &str,
        input_selector: &str,
        button_selector: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let escaped = text.replace('\\', "\\\\").replace('\'', "\\'").replace('\n', "\\n");
        let btn_sel = if button_selector.is_empty() {
            // Auto-detect: find nearest submit/send button
            "document.querySelector('button[type=\"submit\"], button[class*=\"send\"], button[aria-label*=\"Send\"], button[aria-label*=\"Enviar\"]')"
        } else {
            &format!("document.querySelector('{}')", button_selector.replace('\'', "\\'"))
        };

        let js = format!(r#"
            (async () => {{
                const el = document.querySelector('{input_sel}');
                if (!el) return 'NO_INPUT';

                // Focus and clear
                el.focus();
                document.execCommand('selectAll', false, null);
                document.execCommand('delete', false, null);

                // Insert text via execCommand (triggers React/framework state)
                document.execCommand('insertText', false, '{text}');

                // Fire events to activate send button (React needs these)
                el.dispatchEvent(new InputEvent('beforeinput', {{bubbles: true, cancelable: true, inputType: 'insertText', data: '.'}}));
                el.dispatchEvent(new InputEvent('input', {{bubbles: true, cancelable: true, inputType: 'insertText', data: '.'}}));
                el.dispatchEvent(new KeyboardEvent('keydown', {{key: '.', code: 'Period', bubbles: true}}));
                el.dispatchEvent(new KeyboardEvent('keyup', {{key: '.', code: 'Period', bubbles: true}}));

                // Wait for framework to process
                await new Promise(r => setTimeout(r, 300));

                // Find and click send button
                const btn = {btn_sel};
                if (!btn) return 'NO_BUTTON';
                if (btn.disabled) return 'BUTTON_DISABLED';
                btn.click();

                // Wait and verify
                await new Promise(r => setTimeout(r, 2000));
                const body = document.body.innerText;
                if (body.includes('Volver a intentar') || body.includes('Error al enviar') || body.includes('Try again')) {{
                    return 'SEND_ERROR';
                }}
                return 'SENT';
            }})()
        "#,
            input_sel = input_selector.replace('\'', "\\'"),
            text = escaped,
            btn_sel = btn_sel,
        );

        let result = self.eval_string(&js).await?;
        eprintln!("[ENGINE] send_message: {result}");
        Ok(result)
    }

    pub async fn scroll(&self, direction: &str) -> Result<(), Box<dyn std::error::Error>> {
        let delta = match direction {
            "down" | "d" => 400,
            "up" | "u" => -400,
            "bottom" => 99999,
            "top" => -99999,
            _ => 400,
        };
        self.eval_string(&format!("window.scrollBy(0, {delta})"))
            .await?;
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        eprintln!("[ENGINE] scroll {direction}");
        Ok(())
    }

    pub async fn screenshot(&self) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let result = page::capture_screenshot(&self.scoped_main(), page::ScreenshotParams {
            format: Some("jpeg".into()),
            quality: Some(40),
            ..Default::default()
        }).await.map_err(cdp_err)?;

        use base64::Engine;
        let data = base64::engine::general_purpose::STANDARD.decode(&result.data)?;
        eprintln!("[ENGINE] screenshot: {}KB", data.len() / 1024);
        Ok(data)
    }

    pub async fn eval(&self, js: &str) -> Result<String, Box<dyn std::error::Error>> {
        if self.active_frame_id.is_some() {
            return self.eval_in_active_frame(js).await;
        }
        self.eval_string(js).await
    }

    // ─── Frames (cross-origin iframe support via CDP sessions) ───

    /// Returns the CDP session ID for Input.dispatch* commands.
    /// Reserved for future OOP frame input routing.
    #[allow(dead_code)]
    fn active_session_id(&self) -> &str {
        self.active_frame_session_id.as_deref().unwrap_or(&self.page_session_id)
    }

    pub async fn list_frames(&self) -> Result<Value, Box<dyn std::error::Error>> {
        let tree = page::get_frame_tree(&self.scoped_main()).await.map_err(cdp_err)?;
        let tree_value = serde_json::to_value(&tree)?;
        let mut frames: Vec<Value> = Vec::new();
        fn collect_frames(node: &Value, frames: &mut Vec<Value>) {
            if let Some(frame) = node.get("frame") {
                frames.push(json!({
                    "id": frame.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                    "url": frame.get("url").and_then(|v| v.as_str()).unwrap_or(""),
                    "name": frame.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                    "parentId": frame.get("parentId").and_then(|v| v.as_str()).unwrap_or(""),
                    "securityOrigin": frame.get("securityOrigin").and_then(|v| v.as_str()).unwrap_or(""),
                }));
            }
            if let Some(children) = node.get("childFrames").and_then(|v| v.as_array()) {
                for child in children { collect_frames(child, frames); }
            }
        }
        // FrameTree serializes with "frame" and "childFrames" keys
        collect_frames(&tree_value, &mut frames);
        for (idx, frame) in frames.iter_mut().enumerate() {
            let fid = frame["id"].as_str().unwrap_or("").to_string();
            let score = self.score_frame(&fid).await.unwrap_or(0);
            frame["index"] = json!(idx);
            frame["score"] = json!(score);
        }
        eprintln!("[ENGINE] list_frames: {} frames found", frames.len());
        Ok(json!(frames))
    }

    async fn score_frame(&self, frame_id: &str) -> Result<u64, Box<dyn std::error::Error>> {
        let ctx = page::create_isolated_world(
            &self.scoped_main(), frame_id,
            Some("neobrowser_frame_score"), Some(true),
        ).await.map_err(cdp_err)?;
        let result = runtime::evaluate(
            &self.scoped_main(),
            runtime::EvaluateParams {
                expression: "document.querySelectorAll('a,button,input,select,textarea,[role=\"button\"],[role=\"link\"]').length".into(),
                context_id: Some(ctx),
                return_by_value: Some(true),
                ..runtime::EvaluateParams::new("")
            },
        ).await.map_err(cdp_err)?;
        Ok(result.result.value.as_ref().and_then(|v| v.as_u64()).unwrap_or(0))
    }

    pub async fn switch_frame(&mut self, frame_id: &str) -> Result<String, Box<dyn std::error::Error>> {
        let tree = page::get_frame_tree(&self.scoped_main()).await.map_err(cdp_err)?;
        let tree_value = serde_json::to_value(&tree)?;
        let main_fid = tree_value["frame"]["id"].as_str().unwrap_or("");
        if frame_id == main_fid { return self.switch_to_main_frame().await; }
        let frame_url = Self::find_frame_url(&tree_value, frame_id).unwrap_or_default();
        // Strategy 1: OOP frame via Target.attachToTarget
        let targets = target::get_targets(&self.cdp, None).await.map_err(cdp_err)?;
        let iframe_target = targets.iter().find(|t| t.type_ == "iframe"
            && (t.target_id == frame_id
                || (!frame_url.is_empty() && t.url == frame_url)));
        if let Some(t) = iframe_target {
            let sid = target::attach_to_target(&self.cdp, &t.target_id, Some(true)).await.map_err(cdp_err)?;
            let frame_scoped = ScopedTransport::new(&self.cdp, &sid);
            runtime::enable(&frame_scoped).await.map_err(cdp_err)?;
            self.active_frame_session_id = Some(sid.clone());
            self.active_frame_id = Some(frame_id.to_string());
            eprintln!("[ENGINE] Switched to OOP frame: {} (session={})", frame_id, &sid[..8.min(sid.len())]);
            return Ok(format!("switched to frame (OOP, url={})", frame_url));
        }
        // Strategy 2: Same-process frame via isolated world
        page::create_isolated_world(
            &self.scoped_main(), frame_id,
            Some("neobrowser_frame"), Some(true),
        ).await.map_err(cdp_err)?;
        self.active_frame_session_id = None;
        self.active_frame_id = Some(frame_id.to_string());
        eprintln!("[ENGINE] Switched to same-process frame: {} (url={})", frame_id, frame_url);
        Ok(format!("switched to frame (same-process, url={})", frame_url))
    }

    fn find_frame_url(node: &Value, target_id: &str) -> Option<String> {
        if let Some(frame) = node.get("frame") {
            if frame.get("id").and_then(|v| v.as_str()) == Some(target_id) {
                return frame.get("url").and_then(|v| v.as_str()).map(String::from);
            }
        }
        if let Some(children) = node.get("childFrames").and_then(|v| v.as_array()) {
            for child in children {
                if let Some(url) = Self::find_frame_url(child, target_id) { return Some(url); }
            }
        }
        None
    }

    pub async fn switch_to_main_frame(&mut self) -> Result<String, Box<dyn std::error::Error>> {
        self.active_frame_session_id = None;
        self.active_frame_id = None;
        eprintln!("[ENGINE] Switched to main frame");
        Ok("switched to main frame".to_string())
    }

    pub async fn auto_switch_frame(&mut self, text_hint: &str) -> Result<String, Box<dyn std::error::Error>> {
        let tree = page::get_frame_tree(&self.scoped_main()).await.map_err(cdp_err)?;
        let tree_value = serde_json::to_value(&tree)?;
        let mut fids: Vec<String> = Vec::new();
        fn collect_ids(node: &Value, ids: &mut Vec<String>) {
            if let Some(f) = node.get("frame") { if let Some(id) = f.get("id").and_then(|v| v.as_str()) { ids.push(id.to_string()); } }
            if let Some(ch) = node.get("childFrames").and_then(|v| v.as_array()) { for c in ch { collect_ids(c, ids); } }
        }
        collect_ids(&tree_value, &mut fids);
        let escaped = text_hint.replace('\\', "\\\\").replace('\'', "\\'");
        let search_js = format!("!!(document.body && document.body.innerText.includes('{}'))", escaped);
        for fid in &fids {
            let ctx = match page::create_isolated_world(
                &self.scoped_main(), fid,
                Some("neobrowser_auto_frame"), Some(true),
            ).await { Ok(id) => id, Err(_) => continue };
            let result = runtime::evaluate(
                &self.scoped_main(),
                runtime::EvaluateParams {
                    expression: search_js.clone(),
                    context_id: Some(ctx),
                    return_by_value: Some(true),
                    ..runtime::EvaluateParams::new("")
                },
            ).await;
            if let Ok(r) = result {
                if r.result.value.as_ref().and_then(|v| v.as_bool()) == Some(true) {
                    eprintln!("[ENGINE] auto_frame: text '{}' found in frame {}", text_hint, fid);
                    return self.switch_frame(fid).await;
                }
            }
        }
        Err(format!("Text '{}' not found in any frame", text_hint).into())
    }

    pub async fn eval_in_active_frame(&self, expression: &str) -> Result<String, Box<dyn std::error::Error>> {
        // Helper to extract value from EvaluateResult
        fn extract_result(r: &runtime::EvaluateResult) -> Result<String, Box<dyn std::error::Error>> {
            if let Some(ref exc) = r.exception_details {
                return Err(exc.text.clone().into());
            }
            match &r.result.value {
                Some(Value::String(s)) => Ok(s.clone()),
                Some(Value::Null) | None => Ok(String::new()),
                Some(other) => Ok(other.to_string()),
            }
        }

        if let Some(ref fsid) = self.active_frame_session_id {
            let frame_scoped = ScopedTransport::new(&self.cdp, fsid);
            let result = runtime::evaluate(
                &frame_scoped,
                runtime::EvaluateParams {
                    expression: expression.into(),
                    return_by_value: Some(true),
                    await_promise: Some(true),
                    ..runtime::EvaluateParams::new("")
                },
            ).await.map_err(cdp_err)?;
            return extract_result(&result);
        }
        if let Some(ref fid) = self.active_frame_id {
            let ctx = page::create_isolated_world(
                &self.scoped_main(), fid,
                Some("neobrowser_frame_eval"), Some(true),
            ).await.map_err(cdp_err)?;
            let result = runtime::evaluate(
                &self.scoped_main(),
                runtime::EvaluateParams {
                    expression: expression.into(),
                    context_id: Some(ctx),
                    return_by_value: Some(true),
                    await_promise: Some(true),
                    ..runtime::EvaluateParams::new("")
                },
            ).await.map_err(cdp_err)?;
            return extract_result(&result);
        }
        // No frame active — eval directly on the page (avoids recursion with eval_string)
        let result = runtime::evaluate(
            &self.scoped_main(),
            runtime::EvaluateParams {
                expression: expression.into(),
                return_by_value: Some(true),
                await_promise: Some(true),
                ..runtime::EvaluateParams::new("")
            },
        ).await.map_err(cdp_err)?;
        extract_result(&result)
    }

    pub fn active_frame_info(&self) -> Value {
        json!({
            "frame_id": self.active_frame_id.as_deref().unwrap_or("none"),
            "has_oop_session": self.active_frame_session_id.is_some(),
            "mode": if self.active_frame_session_id.is_some() { "oop" } else if self.active_frame_id.is_some() { "same-process" } else { "main" }
        })
    }

    // ─── Tabs / Pages ───

    pub async fn pages(&self) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let targets = target::get_targets(&self.cdp, None).await.map_err(cdp_err)?;
        let mut pages = Vec::new();
        for t in &targets {
            if t.type_ == "page" {
                pages.push(format!("{} | {}", t.title, t.url));
            }
        }
        Ok(pages)
    }

    pub async fn switch_tab(&mut self, index: usize) -> Result<(), Box<dyn std::error::Error>> {
        let targets = target::get_targets(&self.cdp, None).await.map_err(cdp_err)?;
        let page_targets: Vec<&target::TargetInfo> = targets
            .iter()
            .filter(|t| t.type_ == "page")
            .collect();

        if index >= page_targets.len() {
            return Err(format!("Tab {index} not found (have {})", page_targets.len()).into());
        }

        let t = page_targets[index];
        let new_target_id = t.target_id.clone();

        // Attach to the new target
        let session_id = target::attach_to_target(&self.cdp, &new_target_id, Some(true))
            .await.map_err(cdp_err)?;

        let tab_scoped = ScopedTransport::new(&self.cdp, &session_id);
        page::enable(&tab_scoped).await.map_err(cdp_err)?;
        runtime::enable(&tab_scoped).await.map_err(cdp_err)?;

        self.target_id = new_target_id;
        self.last_url = t.url.clone();
        self.page_session_id = session_id;
        // Reset frame context when switching tabs
        self.active_frame_session_id = None;
        self.active_frame_id = None;

        eprintln!("[ENGINE] Switched to tab {index}: {}", self.last_url);
        Ok(())
    }

    // ─── Dialog handling ───

    pub async fn setup_dialog_handler(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.eval_string(r#"
            window.__neo_dialogs = [];
            window.alert = function(msg) { window.__neo_dialogs.push({type:'alert',message:msg}); };
            window.confirm = function(msg) { window.__neo_dialogs.push({type:'confirm',message:msg}); return true; };
            window.prompt = function(msg,def) { window.__neo_dialogs.push({type:'prompt',message:msg}); return def || ''; };
            window.onbeforeunload = null;
            'ok'
        "#).await?;
        eprintln!("[ENGINE] dialog handler installed");
        Ok(())
    }

    pub async fn get_dialogs(&self) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
        let result = self.eval_string(r#"
            (() => {
                const d = window.__neo_dialogs || [];
                window.__neo_dialogs = [];
                return JSON.stringify(d);
            })()
        "#).await?;
        let dialogs: Vec<Value> = serde_json::from_str(&result).unwrap_or_default();
        Ok(dialogs)
    }

    // ─── Bulk form fill ───

    pub async fn fill_form(
        &self,
        fields: &[(String, String)],
    ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let mut results = Vec::new();
        for (target, value) in fields {
            let focused = self.focus(target).await?;
            if focused {
                // type_text already clears with Ctrl+A + Backspace
                self.type_text(value).await?;
                results.push(format!(
                    "filled: {target} = {}",
                    if value.len() > 20 { &value[..20] } else { value }
                ));
            } else {
                results.push(format!("not_found: {target}"));
            }
        }
        eprintln!("[ENGINE] fill_form: {} fields", fields.len());
        Ok(results)
    }

    // ─── Hover ───

    pub async fn hover(&self, text: &str) -> Result<bool, Box<dyn std::error::Error>> {
        let js = format!(
            r#"
            (() => {{
                const _doc = {active_doc};
                const target = {target_json};
                const lower = target.toLowerCase();
                for (const el of _doc.querySelectorAll('a, button, [role="button"], [role="link"], [role="menuitem"]')) {{
                    const t = (el.textContent || el.getAttribute('aria-label') || '').trim();
                    if (t.toLowerCase().includes(lower)) {{
                        el.scrollIntoViewIfNeeded();
                        const rect = el.getBoundingClientRect();
                        return JSON.stringify({{x: rect.x + rect.width/2, y: rect.y + rect.height/2, text: t.substring(0,60)}});
                    }}
                }}
                return 'not_found';
            }})()
            "#,
            active_doc = Self::ACTIVE_DOC_JS,
            target_json = serde_json::to_string(text)?
        );

        let result = self.eval_string(&js).await?;
        if result == "not_found" {
            eprintln!("[ENGINE] hover not found: '{text}'");
            return Ok(false);
        }

        let coords: Value = serde_json::from_str(&result)?;
        let x = coords["x"].as_f64().unwrap_or(0.0);
        let y = coords["y"].as_f64().unwrap_or(0.0);

        self.cdp
            .send_to(
                &self.page_session_id,
                "Input.dispatchMouseEvent",
                Some(json!({
                    "type": "mouseMoved",
                    "x": x,
                    "y": y,
                })),
            )
            .await?;

        eprintln!("[ENGINE] hovered: {}", coords["text"].as_str().unwrap_or(""));
        Ok(true)
    }

    // ─── Select option ───

    pub async fn select_option(
        &self,
        target: &str,
        value: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let js = format!(
            r#"
            (() => {{
                const _doc = {active_doc};
                const target = {target_json};
                const value = {value_json};
                const lower = target.toLowerCase();
                for (const el of _doc.querySelectorAll('select')) {{
                    const label = (el.getAttribute('aria-label') || el.name || el.id || '').toLowerCase();
                    if (label.includes(lower) || lower === '') {{
                        el.value = value;
                        el.dispatchEvent(new Event('change', {{bubbles: true}}));
                        return 'selected: ' + value;
                    }}
                }}
                return 'not_found';
            }})()
            "#,
            active_doc = Self::ACTIVE_DOC_JS,
            target_json = serde_json::to_string(target)?,
            value_json = serde_json::to_string(value)?
        );

        let result = self.eval_string(&js).await?;
        let found = !result.starts_with("not_found");
        if found {
            eprintln!("[ENGINE] {result}");
        } else {
            eprintln!("[ENGINE] select not found: '{target}'");
        }
        Ok(found)
    }

    // ─── Network capture ───

    pub async fn start_network_capture(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.eval_string(r#"
            window.__neo_net = [];
            const origFetch = window.fetch;
            window.fetch = function(...args) {
                const url = typeof args[0] === 'string' ? args[0] : args[0]?.url || '';
                const method = args[1]?.method || 'GET';
                const entry = {type:'fetch', method, url, ts: Date.now()};
                window.__neo_net.push(entry);
                return origFetch.apply(this, args).then(r => { entry.status = r.status; return r; });
            };
            const origXHR = XMLHttpRequest.prototype.open;
            XMLHttpRequest.prototype.open = function(method, url) {
                this.__neo = {type:'xhr', method, url, ts: Date.now()};
                window.__neo_net.push(this.__neo);
                return origXHR.apply(this, arguments);
            };
            const origSend = XMLHttpRequest.prototype.send;
            XMLHttpRequest.prototype.send = function() {
                this.addEventListener('load', () => { if(this.__neo) this.__neo.status = this.status; });
                return origSend.apply(this, arguments);
            };
            'ok'
        "#).await?;
        eprintln!("[ENGINE] network capture started");
        Ok(())
    }

    pub async fn read_network(&self) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
        let result = self.eval_string(r#"
            (() => {
                const d = window.__neo_net || [];
                window.__neo_net = [];
                return JSON.stringify(d);
            })()
        "#).await?;
        let reqs: Vec<Value> = serde_json::from_str(&result).unwrap_or_default();
        Ok(reqs)
    }

    // ─── Console capture ───

    pub async fn start_console_capture(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.eval_string(r#"
            window.__neo_console = [];
            for (const level of ['log','warn','error','info','debug']) {
                const orig = console[level];
                console[level] = function(...args) {
                    window.__neo_console.push({level, message: args.map(a=>String(a)).join(' '), ts: Date.now()});
                    return orig.apply(console, args);
                };
            }
            window.addEventListener('error', (e) => {
                window.__neo_console.push({level:'exception', message: e.message, ts: Date.now()});
            });
            'ok'
        "#).await?;
        eprintln!("[ENGINE] console capture started");
        Ok(())
    }

    pub async fn read_console(&self) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
        let result = self.eval_string(r#"
            (() => {
                const d = window.__neo_console || [];
                window.__neo_console = [];
                return JSON.stringify(d);
            })()
        "#).await?;
        let msgs: Vec<Value> = serde_json::from_str(&result).unwrap_or_default();
        Ok(msgs)
    }

    // ─── Cookie / Storage extraction ───

    pub async fn get_all_cookies(&self) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
        let cookies = network::get_all_cookies(&self.scoped_main()).await.map_err(cdp_err)?;
        // Convert typed cookies back to Value for backward compat
        let values: Vec<Value> = cookies
            .into_iter()
            .filter_map(|c| serde_json::to_value(c).ok())
            .collect();
        Ok(values)
    }

    pub async fn get_local_storage(
        &self,
    ) -> Result<std::collections::HashMap<String, String>, Box<dyn std::error::Error>> {
        let result = self.eval_string(
            "JSON.stringify(Object.fromEntries(Object.keys(localStorage).map(k=>[k,localStorage[k]])))"
        ).await?;
        let data: std::collections::HashMap<String, String> =
            serde_json::from_str(&result).unwrap_or_default();
        Ok(data)
    }

    pub async fn set_local_storage(
        &self,
        data: &std::collections::HashMap<String, String>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        for (key, value) in data {
            let js = format!(
                "localStorage.setItem({}, {})",
                serde_json::to_string(key)?,
                serde_json::to_string(value)?
            );
            self.eval_string(&js).await?;
        }
        eprintln!("[ENGINE] set {} localStorage items", data.len());
        Ok(())
    }

    // ─── Session Storage ───

    pub async fn get_session_storage(
        &self,
    ) -> Result<std::collections::HashMap<String, String>, Box<dyn std::error::Error>> {
        let result = self.eval_string(
            "JSON.stringify(Object.fromEntries(Object.keys(sessionStorage).map(k=>[k,sessionStorage[k]])))"
        ).await?;
        let data: std::collections::HashMap<String, String> =
            serde_json::from_str(&result).unwrap_or_default();
        Ok(data)
    }

    pub async fn set_session_storage(
        &self,
        data: &std::collections::HashMap<String, String>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        for (key, value) in data {
            let js = format!(
                "sessionStorage.setItem({}, {})",
                serde_json::to_string(key)?,
                serde_json::to_string(value)?
            );
            self.eval_string(&js).await?;
        }
        eprintln!("[ENGINE] set {} sessionStorage items", data.len());
        Ok(())
    }

    /// Export full browser state: cookies + localStorage + sessionStorage + URL.
    pub async fn export_state(&self) -> Result<Value, Box<dyn std::error::Error>> {
        let cookies = self.get_all_cookies().await?;
        let local_storage = self.get_local_storage().await?;
        let session_storage = self.get_session_storage().await?;
        let url = self.eval_string("location.href").await.unwrap_or_default();

        Ok(serde_json::json!({
            "url": url,
            "cookies": cookies,
            "localStorage": local_storage,
            "sessionStorage": session_storage,
            "exportedAt": chrono::Utc::now().to_rfc3339(),
        }))
    }

    /// Import state from a previous export.
    pub async fn import_state(&self, state: &Value) -> Result<String, Box<dyn std::error::Error>> {
        let mut imported = Vec::new();

        // Cookies via CDP (batch set)
        if let Some(cookies) = state["cookies"].as_array() {
            let cookie_params: Vec<network::CookieParam> = cookies
                .iter()
                .filter_map(|c| serde_json::from_value(c.clone()).ok())
                .collect();
            let count = cookie_params.len();
            if !cookie_params.is_empty() {
                let _ = network::set_cookies(&self.scoped_main(), cookie_params).await;
            }
            imported.push(format!("{} cookies", count));
        }

        // localStorage
        if let Some(ls) = state["localStorage"].as_object() {
            let data: std::collections::HashMap<String, String> = ls.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect();
            self.set_local_storage(&data).await?;
            imported.push(format!("{} localStorage", data.len()));
        }

        // sessionStorage
        if let Some(ss) = state["sessionStorage"].as_object() {
            let data: std::collections::HashMap<String, String> = ss.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect();
            self.set_session_storage(&data).await?;
            imported.push(format!("{} sessionStorage", data.len()));
        }

        Ok(imported.join(", "))
    }

    /// Check if current session appears healthy (logged in, no errors).
    pub async fn check_session_health(&self) -> Result<Value, Box<dyn std::error::Error>> {
        let js = r#"(() => {
            const url = location.href;
            const title = document.title;
            const hasLoginForm = !!document.querySelector('input[type="password"], form[action*="login"], form[action*="signin"]');
            const has401 = document.body?.innerText?.includes('401') || document.body?.innerText?.includes('Unauthorized');
            const hasCaptcha = !!document.querySelector('[class*="captcha"], [id*="captcha"], iframe[src*="recaptcha"]');
            const hasError = !!document.querySelector('.error, .alert-danger, [role="alert"]');
            const cookieCount = document.cookie.split(';').filter(c => c.trim()).length;

            return JSON.stringify({
                url, title, cookieCount,
                hasLoginForm, has401, hasCaptcha, hasError,
                healthy: !hasLoginForm && !has401 && !hasCaptcha && !hasError
            });
        })()"#;

        let result = self.eval_string(js).await?;
        let data: Value = serde_json::from_str(&result).unwrap_or(serde_json::json!({"healthy": false}));
        Ok(data)
    }

    // ─── Network Intelligence (CDP-level) ───

    /// Start CDP-level network capture via Network.enable events.
    /// Captures ALL requests including cross-origin iframes and survives navigation.
    pub async fn start_cdp_network_capture(&self) -> Result<(), Box<dyn std::error::Error>> {
        // Clear previous entries
        {
            let mut entries = self.cdp_network_entries.lock().await;
            entries.clear();
        }

        // Enable Network domain
        network::enable(&self.scoped_main(), None, None, None).await.map_err(cdp_err)?;

        // Mark as active
        self.cdp_network_active.store(true, std::sync::atomic::Ordering::SeqCst);

        // Listen for Network.requestWillBeSent — captures method, url, headers, postData
        let entries_clone = self.cdp_network_entries.clone();
        let active_clone = self.cdp_network_active.clone();
        self.cdp.on("Network.requestWillBeSent", Arc::new(move |params| {
            if !active_clone.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            let request = &params["request"];
            let entry = CdpNetworkEntry {
                request_id: params["requestId"].as_str().unwrap_or("").to_string(),
                method: request["method"].as_str().unwrap_or("GET").to_string(),
                url: request["url"].as_str().unwrap_or("").to_string(),
                request_headers: request["headers"].clone(),
                post_data: request["postData"].as_str().map(|s| s.to_string()),
                resource_type: params["type"].as_str().unwrap_or("Other").to_string(),
                timestamp: params["timestamp"].as_f64().unwrap_or(0.0),
                response_status: None,
                response_headers: None,
                response_mime: None,
                frame_id: params["frameId"].as_str().unwrap_or("").to_string(),
            };
            let entries = entries_clone.clone();
            tokio::spawn(async move {
                entries.lock().await.push(entry);
            });
        })).await;

        // Listen for Network.responseReceived — captures status, headers, mimeType
        let entries_clone2 = self.cdp_network_entries.clone();
        let active_clone2 = self.cdp_network_active.clone();
        self.cdp.on("Network.responseReceived", Arc::new(move |params| {
            if !active_clone2.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            let request_id = params["requestId"].as_str().unwrap_or("").to_string();
            let response = &params["response"];
            let status = response["status"].as_i64();
            let headers = Some(response["headers"].clone());
            let mime = response["mimeType"].as_str().map(|s| s.to_string());
            let entries = entries_clone2.clone();
            tokio::spawn(async move {
                let mut entries = entries.lock().await;
                // Find the matching request entry and update it with response data
                for entry in entries.iter_mut().rev() {
                    if entry.request_id == request_id {
                        entry.response_status = status;
                        entry.response_headers = headers;
                        entry.response_mime = mime;
                        break;
                    }
                }
            });
        })).await;

        eprintln!("[ENGINE] CDP network capture started (survives navigation, captures iframes)");
        Ok(())
    }

    /// Read CDP-captured network entries. Drains the buffer.
    /// Optionally filter by URL pattern (simple substring match).
    pub async fn read_cdp_network(&self, url_filter: Option<&str>) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
        let mut entries = self.cdp_network_entries.lock().await;
        let drained: Vec<CdpNetworkEntry> = entries.drain(..).collect();
        drop(entries);

        let results: Vec<Value> = drained.iter()
            .filter(|e| {
                match url_filter {
                    Some(pat) => e.url.contains(pat),
                    None => true,
                }
            })
            .map(|e| e.to_json())
            .collect();

        Ok(results)
    }

    /// Get response body for a request ID via CDP.
    pub async fn get_response_body(&self, request_id: &str) -> Result<String, Box<dyn std::error::Error>> {
        let (body, _base64_encoded) = network::get_response_body(&self.scoped_main(), request_id)
            .await.map_err(cdp_err)?;
        Ok(body)
    }

    /// Stop CDP network capture and disable Network domain.
    pub async fn stop_cdp_network_capture(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.cdp_network_active.store(false, std::sync::atomic::Ordering::SeqCst);
        network::disable(&self.scoped_main()).await.map_err(cdp_err)?;
        eprintln!("[ENGINE] CDP network capture stopped");
        Ok(())
    }

    /// Intercept requests matching a URL pattern and respond with custom data.
    /// Uses Fetch.enable + Fetch.requestPaused -> Fetch.fulfillRequest.
    pub async fn intercept_requests(
        &self,
        url_pattern: &str,
        response_body: &str,
        status_code: Option<u16>,
        content_type: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Store the rule
        {
            let mut rules = self.intercept_rules.lock().await;
            rules.push(InterceptRule {
                url_pattern: url_pattern.to_string(),
                response_body: response_body.to_string(),
                status_code: status_code.unwrap_or(200),
                content_type: content_type.unwrap_or("application/json").to_string(),
            });
        }

        // Enable Fetch domain with the pattern
        fetch::enable(
            &self.scoped_main(),
            Some(vec![fetch::RequestPattern {
                url_pattern: Some(url_pattern.to_string()),
                resource_type: None,
                request_stage: None,
            }]),
            None,
        ).await.map_err(cdp_err)?;

        // TODO: migrate Fetch.requestPaused callback to typed fetch module
        // (fire-and-forget pattern with clone_tx makes this non-trivial)
        let rules_clone = self.intercept_rules.clone();
        let cdp_tx = self.cdp.clone_tx();
        let id_counter = self.cdp.shared_id_counter();
        let session_id = self.page_session_id.clone();
        self.cdp.on("Fetch.requestPaused", Arc::new(move |params| {
            let request_id = params["requestId"].as_str().unwrap_or("").to_string();
            let request_url = params["request"]["url"].as_str().unwrap_or("").to_string();
            let rules = rules_clone.clone();
            let cdp_tx = cdp_tx.clone();
            let id_counter = id_counter.clone();
            let session_id = session_id.clone();
            tokio::spawn(async move {
                let rules = rules.lock().await;
                // Find matching rule
                let matched = rules.iter().find(|r| {
                    // Simple glob: * matches anything
                    let pat = &r.url_pattern;
                    if pat.contains('*') {
                        let parts: Vec<&str> = pat.split('*').collect();
                        let mut pos = 0usize;
                        for part in &parts {
                            if part.is_empty() { continue; }
                            match request_url[pos..].find(part) {
                                Some(idx) => pos += idx + part.len(),
                                None => return false,
                            }
                        }
                        true
                    } else {
                        request_url.contains(pat)
                    }
                });

                let cmd_id = id_counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

                if let Some(rule) = matched {
                    // Encode body as base64 for Fetch.fulfillRequest
                    use base64::Engine;
                    let body_b64 = base64::engine::general_purpose::STANDARD.encode(&rule.response_body);
                    let fulfill = json!({
                        "id": cmd_id,
                        "method": "Fetch.fulfillRequest",
                        "sessionId": session_id,
                        "params": {
                            "requestId": request_id,
                            "responseCode": rule.status_code,
                            "responseHeaders": [
                                {"name": "Content-Type", "value": rule.content_type},
                                {"name": "Access-Control-Allow-Origin", "value": "*"},
                            ],
                            "body": body_b64,
                        }
                    });
                    let _ = cdp_tx.send(fulfill.to_string());
                    eprintln!("[ENGINE] Intercepted & fulfilled: {request_url}");
                } else {
                    // Continue the request unmodified
                    let cont = json!({
                        "id": cmd_id,
                        "method": "Fetch.continueRequest",
                        "sessionId": session_id,
                        "params": {
                            "requestId": request_id,
                        }
                    });
                    let _ = cdp_tx.send(cont.to_string());
                }
            });
        })).await;

        eprintln!("[ENGINE] Request interception enabled for: {url_pattern}");
        Ok(())
    }

    /// Clear all intercept rules and disable Fetch domain.
    pub async fn clear_intercepts(&self) -> Result<(), Box<dyn std::error::Error>> {
        {
            let mut rules = self.intercept_rules.lock().await;
            rules.clear();
        }
        fetch::disable(&self.scoped_main()).await.map_err(cdp_err)?;
        eprintln!("[ENGINE] Request interception cleared");
        Ok(())
    }

    /// Capture full request/response via JS monkeypatch (original approach).
    /// Works for same-origin requests only. Gets wiped on navigation.
    pub async fn start_js_network_capture(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.eval_string(r#"
            window.__neo_full_net = [];
            const origFetch = window.__origFetch || window.fetch;
            window.__origFetch = origFetch;
            window.fetch = async function(...args) {
                const url = typeof args[0] === 'string' ? args[0] : args[0]?.url || '';
                const method = args[1]?.method || 'GET';
                const headers = args[1]?.headers || {};
                const reqBody = args[1]?.body || null;
                const entry = {type:'fetch', method, url, reqHeaders: headers, ts: Date.now()};
                try {
                    const r = await origFetch.apply(this, args);
                    entry.status = r.status;
                    entry.resHeaders = Object.fromEntries(r.headers.entries());
                    const clone = r.clone();
                    try {
                        const text = await clone.text();
                        entry.body = text.substring(0, 4096);
                    } catch(e) {}
                    window.__neo_full_net.push(entry);
                    return r;
                } catch(e) {
                    entry.error = e.message;
                    window.__neo_full_net.push(entry);
                    throw e;
                }
            };

            const origXHR = XMLHttpRequest.prototype.open;
            const origXHRSend = XMLHttpRequest.prototype.send;
            XMLHttpRequest.prototype.open = function(method, url) {
                this.__neo = {type:'xhr', method, url, ts: Date.now()};
                return origXHR.apply(this, arguments);
            };
            XMLHttpRequest.prototype.send = function(body) {
                if (this.__neo) {
                    this.__neo.reqBody = typeof body === 'string' ? body.substring(0, 2048) : null;
                }
                this.addEventListener('load', () => {
                    if (this.__neo) {
                        this.__neo.status = this.status;
                        this.__neo.resHeaders = this.getAllResponseHeaders();
                        this.__neo.body = this.responseText?.substring(0, 4096);
                        window.__neo_full_net.push(this.__neo);
                    }
                });
                return origXHRSend.apply(this, arguments);
            };
            'ok'
        "#).await?;
        eprintln!("[ENGINE] JS network capture started (monkeypatch)");
        Ok(())
    }

    /// Alias: start network capture (defaults to JS mode for backward compat).
    pub async fn start_full_network_capture(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.start_js_network_capture().await
    }

    /// Read JS-captured network data with bodies.
    pub async fn read_js_network(&self) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
        let result = self.eval_string(r#"
            (() => {
                const d = window.__neo_full_net || [];
                window.__neo_full_net = [];
                return JSON.stringify(d);
            })()
        "#).await?;
        let reqs: Vec<Value> = serde_json::from_str(&result).unwrap_or_default();
        Ok(reqs)
    }

    /// Alias: read network data (defaults to JS mode for backward compat).
    pub async fn read_full_network(&self) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
        self.read_js_network().await
    }

    /// Export captured network data as simplified HAR.
    /// Reads from whichever capture mode has data (prefers CDP, falls back to JS).
    pub async fn export_har(&self, source: Option<&str>) -> Result<Value, Box<dyn std::error::Error>> {
        let requests = match source.unwrap_or("auto") {
            "cdp" => {
                self.read_cdp_network(None).await?
            }
            "js" => {
                self.read_js_network().await?
            }
            _ => {
                // auto: try CDP first, fall back to JS
                let cdp_reqs = self.read_cdp_network(None).await?;
                if cdp_reqs.is_empty() {
                    self.read_js_network().await?
                } else {
                    cdp_reqs
                }
            }
        };

        let entries: Vec<Value> = requests.iter().map(|r| {
            // Handle both CDP format and JS format
            let method = r["method"].as_str().unwrap_or("GET");
            let url = r["url"].as_str().unwrap_or("");
            let status = r["status"].as_i64().unwrap_or(0);
            json!({
                "startedDateTime": r.get("timestamp").or(r.get("ts")).unwrap_or(&Value::Null),
                "request": {
                    "method": method,
                    "url": url,
                    "headers": r.get("requestHeaders").or(r.get("reqHeaders")).unwrap_or(&Value::Null),
                    "postData": r.get("postData").or(r.get("reqBody")).unwrap_or(&Value::Null),
                },
                "response": {
                    "status": status,
                    "headers": r.get("responseHeaders").or(r.get("resHeaders")).unwrap_or(&Value::Null),
                    "content": {
                        "text": r.get("body").unwrap_or(&Value::Null),
                        "mimeType": r.get("mimeType").unwrap_or(&Value::Null),
                    }
                },
                "frameId": r.get("frameId").unwrap_or(&Value::Null),
            })
        }).collect();

        Ok(json!({
            "log": {
                "version": "1.2",
                "entries": entries,
            }
        }))
    }

    // ─── CSS selector click ───

    /// Click an element by CSS selector. Returns true if found and clicked.
    pub async fn click_css(&self, selector: &str) -> Result<bool, Box<dyn std::error::Error>> {
        let js = format!(r#"
            (() => {{
                const el = document.querySelector({sel});
                if (!el) return 'not_found';
                el.scrollIntoViewIfNeeded();
                el.click();
                return 'clicked: ' + (el.textContent || '').trim().substring(0, 40);
            }})()"#,
            sel = serde_json::to_string(selector)?
        );
        let result = self.eval_string(&js).await?;
        let found = result.starts_with("clicked");
        if found {
            eprintln!("[ENGINE] css_click {selector} → {result}");
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        } else {
            eprintln!("[ENGINE] css_click not found: {selector}");
        }
        Ok(found)
    }

    // ─── Coordinate click ───

    /// Click at specific page coordinates (x, y).
    pub async fn click_at(&self, x: f64, y: f64) -> Result<(), Box<dyn std::error::Error>> {
        input::click(&self.scoped_main(), x, y).await.map_err(cdp_err)?;
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        eprintln!("[ENGINE] click_at ({x}, {y})");
        Ok(())
    }

    // ─── React-compatible type ───

    /// Type into a React input that ignores CDP key events.
    /// Uses nativeInputValueSetter to bypass React's synthetic event system,
    /// then dispatches input/change events to trigger React state updates.
    pub async fn type_react(&self, selector: &str, value: &str) -> Result<bool, Box<dyn std::error::Error>> {
        let js = format!(r#"
            (() => {{
                const el = document.querySelector({sel});
                if (!el) return 'not_found';
                el.scrollIntoViewIfNeeded();
                el.focus();
                el.click();
                // Use native setter to bypass React's controlled input
                const proto = el.tagName === 'TEXTAREA'
                    ? window.HTMLTextAreaElement.prototype
                    : window.HTMLInputElement.prototype;
                const nativeSetter = Object.getOwnPropertyDescriptor(proto, 'value')?.set;
                if (nativeSetter) {{
                    nativeSetter.call(el, {val});
                }} else {{
                    el.value = {val};
                }}
                // Dispatch events React listens to
                el.dispatchEvent(new Event('input', {{ bubbles: true }}));
                el.dispatchEvent(new Event('change', {{ bubbles: true }}));
                // Also fire keyboard events for autocomplete triggers
                el.dispatchEvent(new KeyboardEvent('keydown', {{ key: 'a', bubbles: true }}));
                el.dispatchEvent(new KeyboardEvent('keyup', {{ key: 'a', bubbles: true }}));
                return 'typed: ' + el.value?.substring(0, 40);
            }})()"#,
            sel = serde_json::to_string(selector)?,
            val = serde_json::to_string(value)?,
        );
        let result = self.eval_string(&js).await?;
        let ok = result.starts_with("typed");
        eprintln!("[ENGINE] type_react {selector} → {result}");
        Ok(ok)
    }

    // ─── Keyboard combos ───

    /// Press a key combination (e.g., "Ctrl+a", "Shift+Tab", "Meta+c").
    pub async fn press_combo(&self, combo: &str) -> Result<(), Box<dyn std::error::Error>> {
        // Parse combo: "Ctrl+a" → modifiers + key
        let parts: Vec<&str> = combo.split('+').collect();
        let key_part = parts.last().unwrap_or(&"");

        let (key_val, code, _) = match *key_part {
            "a" | "A" => ("a", "KeyA", 65),
            "c" | "C" => ("c", "KeyC", 67),
            "v" | "V" => ("v", "KeyV", 86),
            "x" | "X" => ("x", "KeyX", 88),
            "z" | "Z" => ("z", "KeyZ", 90),
            "Tab" | "tab" => ("Tab", "Tab", 9),
            "Enter" | "enter" => ("Enter", "Enter", 13),
            "Backspace" | "backspace" => ("Backspace", "Backspace", 8),
            "Delete" | "delete" => ("Delete", "Delete", 46),
            "ArrowUp" | "up" => ("ArrowUp", "ArrowUp", 38),
            "ArrowDown" | "down" => ("ArrowDown", "ArrowDown", 40),
            "ArrowLeft" | "left" => ("ArrowLeft", "ArrowLeft", 37),
            "ArrowRight" | "right" => ("ArrowRight", "ArrowRight", 39),
            _ => (key_part.clone(), key_part.clone(), 0),
        };

        // Determine modifier flags from prefix parts
        let mut modifiers = 0i32;
        for p in &parts[..parts.len().saturating_sub(1)] {
            match p.to_lowercase().as_str() {
                "alt" => modifiers |= input::MODIFIER_ALT,
                "ctrl" | "control" => modifiers |= input::MODIFIER_CTRL,
                "meta" | "cmd" | "command" => modifiers |= input::MODIFIER_META,
                "shift" => modifiers |= input::MODIFIER_SHIFT,
                _ => {}
            }
        }

        let t = self.scoped_main();
        input::dispatch_key_event(&t, input::DispatchKeyEventParams {
            type_: "keyDown".into(),
            key: Some(key_val.into()),
            code: Some(code.into()),
            modifiers: Some(modifiers),
            ..Default::default()
        }).await.map_err(cdp_err)?;

        input::dispatch_key_event(&t, input::DispatchKeyEventParams {
            type_: "keyUp".into(),
            key: Some(key_val.into()),
            code: Some(code.into()),
            modifiers: Some(modifiers),
            ..Default::default()
        }).await.map_err(cdp_err)?;

        eprintln!("[ENGINE] press_combo: {combo}");
        Ok(())
    }

    // ─── Wait for element ───

    /// Wait for a CSS selector to appear (become visible) in the DOM.
    /// Polls every 500ms up to timeout_ms.
    pub async fn wait_for_selector(&self, selector: &str, timeout_ms: u64) -> Result<bool, Box<dyn std::error::Error>> {
        let start = std::time::Instant::now();
        let deadline = std::time::Duration::from_millis(timeout_ms);

        loop {
            let js = format!(r#"
                (() => {{
                    const el = document.querySelector({sel});
                    if (!el) return 'absent';
                    if (el.offsetParent === null && el.tagName !== 'BODY') return 'hidden';
                    return 'visible';
                }})()"#,
                sel = serde_json::to_string(selector)?
            );
            let result = self.eval_string(&js).await?;
            if result == "visible" {
                eprintln!("[ENGINE] wait_for_selector: {selector} found in {}ms", start.elapsed().as_millis());
                return Ok(true);
            }

            if start.elapsed() > deadline {
                eprintln!("[ENGINE] wait_for_selector: {selector} timeout after {timeout_ms}ms (last: {result})");
                return Ok(false);
            }

            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }

    /// Scroll a specific element into view by CSS selector.
    pub async fn scroll_to(&self, selector: &str) -> Result<bool, Box<dyn std::error::Error>> {
        let js = format!(r#"
            (() => {{
                const el = document.querySelector({sel});
                if (!el) return 'not_found';
                el.scrollIntoView({{ behavior: 'smooth', block: 'center' }});
                return 'scrolled';
            }})()"#,
            sel = serde_json::to_string(selector)?
        );
        let result = self.eval_string(&js).await?;
        let found = result == "scrolled";
        eprintln!("[ENGINE] scroll_to {selector} → {result}");
        Ok(found)
    }

    /// Get bounding box of element by CSS selector (for coordinate-based clicks).
    pub async fn get_element_bounds(&self, selector: &str) -> Result<Option<(f64, f64, f64, f64)>, Box<dyn std::error::Error>> {
        let js = format!(r#"
            (() => {{
                const el = document.querySelector({sel});
                if (!el) return 'null';
                const r = el.getBoundingClientRect();
                return JSON.stringify({{x: r.x, y: r.y, w: r.width, h: r.height}});
            }})()"#,
            sel = serde_json::to_string(selector)?
        );
        let result = self.eval_string(&js).await?;
        if result == "null" {
            return Ok(None);
        }
        let v: serde_json::Value = serde_json::from_str(&result)?;
        Ok(Some((
            v["x"].as_f64().unwrap_or(0.0),
            v["y"].as_f64().unwrap_or(0.0),
            v["w"].as_f64().unwrap_or(0.0),
            v["h"].as_f64().unwrap_or(0.0),
        )))
    }

    // ─── Reliability: click with fallback chain ───

    /// Click with fallback: text → aria-label → placeholder → partial → role → CSS.
    pub async fn click_reliable(&self, target: &str) -> Result<(bool, String), Box<dyn std::error::Error>> {
        // Strategy 0: CSS selector (if target starts with . # [ or contains :)
        if target.starts_with('.') || target.starts_with('#') || target.starts_with('[')
            || target.contains("::") || target.contains(":nth")
        {
            if self.click_css(target).await? {
                return Ok((true, "css_selector".into()));
            }
        }

        // Strategy 1: direct text match (default)
        if self.click(target).await? {
            return Ok((true, "text_match".into()));
        }

        // Strategy 2: aria-label match
        let js_aria = format!(r#"
            (() => {{
                const el = document.querySelector('[aria-label={}]');
                if (el) {{ el.click(); return 'clicked'; }}
                return 'not_found';
            }})()"#,
            serde_json::to_string(target)?
        );
        let result = self.eval_string(&js_aria).await?;
        if result == "clicked" {
            return Ok((true, "aria_label".into()));
        }

        // Strategy 3: placeholder match (for clicking on inputs)
        let js_placeholder = format!(r#"
            (() => {{
                const t = {}.toLowerCase();
                for (const el of document.querySelectorAll('input, textarea, [contenteditable]')) {{
                    const p = (el.placeholder || el.getAttribute('aria-label') || '').toLowerCase();
                    if (p.includes(t)) {{
                        el.scrollIntoViewIfNeeded();
                        el.focus();
                        el.click();
                        return 'clicked';
                    }}
                }}
                return 'not_found';
            }})()"#,
            serde_json::to_string(target)?
        );
        let result = self.eval_string(&js_placeholder).await?;
        if result == "clicked" {
            return Ok((true, "placeholder".into()));
        }

        // Strategy 4: partial text match (contains)
        let js_partial = format!(r#"
            (() => {{
                const t = {};
                const els = [...document.querySelectorAll('button, a, [role="button"], input[type="submit"]')];
                const el = els.find(e => e.textContent?.trim().toLowerCase().includes(t.toLowerCase()));
                if (el) {{ el.click(); return 'clicked'; }}
                return 'not_found';
            }})()"#,
            serde_json::to_string(target)?
        );
        let result = self.eval_string(&js_partial).await?;
        if result == "clicked" {
            return Ok((true, "partial_text".into()));
        }

        // Strategy 5: role-based match
        let js_role = format!(r#"
            (() => {{
                const t = {};
                const els = [...document.querySelectorAll('[role]')];
                const el = els.find(e => e.textContent?.trim().toLowerCase().includes(t.toLowerCase()));
                if (el) {{ el.click(); return 'clicked'; }}
                return 'not_found';
            }})()"#,
            serde_json::to_string(target)?
        );
        let result = self.eval_string(&js_role).await?;
        if result == "clicked" {
            return Ok((true, "role_match".into()));
        }

        Ok((false, "all_strategies_failed".into()))
    }

    /// Take a screenshot and return as base64 (for observability).
    pub async fn screenshot_base64(&self) -> Result<String, Box<dyn std::error::Error>> {
        let result = page::capture_screenshot(&self.scoped_main(), page::ScreenshotParams {
            format: Some("jpeg".into()),
            quality: Some(40),
            ..Default::default()
        }).await.map_err(cdp_err)?;
        Ok(result.data)
    }

    // ─── Form Analysis ───

    /// Analyze all forms on the page, including Vue/React virtual forms.
    /// Returns structured JSON with fields, types, validation, selects, hidden fields.
    pub async fn analyze_forms(&self) -> Result<String, Box<dyn std::error::Error>> {
        let js = r#"
(() => {
  try {
    const results = [];

    function getLabel(el) {
      try {
        if (el.id) {
          const lbl = document.querySelector('label[for="' + CSS.escape(el.id) + '"]');
          if (lbl) return lbl.textContent.trim();
        }
        const parent = el.closest('label');
        if (parent) {
          const clone = parent.cloneNode(true);
          clone.querySelectorAll('input,select,textarea,button').forEach(c => c.remove());
          const txt = clone.textContent.trim();
          if (txt) return txt;
        }
        if (el.getAttribute('aria-label')) return el.getAttribute('aria-label');
        if (el.getAttribute('aria-labelledby')) {
          const ref = document.getElementById(el.getAttribute('aria-labelledby'));
          if (ref) return ref.textContent.trim();
        }
        const prev = el.previousElementSibling;
        if (prev && (prev.tagName === 'LABEL' || prev.tagName === 'SPAN'))
          return prev.textContent.trim();
        return el.getAttribute('placeholder') || '';
      } catch(e) { return ''; }
    }

    function isRequired(el) {
      try {
        if (el.required || el.hasAttribute('required')) return true;
        if (el.getAttribute('aria-required') === 'true') return true;
        const vv = el.getAttribute('v-validate') || el.getAttribute('data-vv-rules') || '';
        if (vv.includes('required')) return true;
        const lbl = getLabel(el);
        if (lbl && /\*/.test(lbl)) return true;
        if (el.className && /required/i.test(el.className)) return true;
        const wrapper = el.closest('.required, .is-required, [class*="required"]');
        if (wrapper) return true;
        return false;
      } catch(e) { return false; }
    }

    function buildSelector(el) {
      try {
        if (el.id) return '#' + CSS.escape(el.id);
        if (el.name) return el.tagName.toLowerCase() + '[name="' + el.name + '"]';
        const parent = el.parentElement;
        if (!parent) return el.tagName.toLowerCase();
        const siblings = Array.from(parent.children).filter(c => c.tagName === el.tagName);
        const idx = siblings.indexOf(el) + 1;
        return el.tagName.toLowerCase() + ':nth-of-type(' + idx + ')';
      } catch(e) { return el.tagName ? el.tagName.toLowerCase() : 'unknown'; }
    }

    function fieldInfo(el) {
      try {
        const tag = el.tagName.toLowerCase();
        const type = el.getAttribute('type') || tag;
        const info = {
          tag: tag,
          name: el.name || el.getAttribute('data-vv-name') || el.id || null,
          type: type,
          required: isRequired(el),
          label: getLabel(el),
          placeholder: el.getAttribute('placeholder') || null,
          value: (type === 'password') ? (el.value ? '***' : '') : (el.value || null),
          pattern: el.getAttribute('pattern') || null,
          minlength: el.getAttribute('minlength') ? +el.getAttribute('minlength') : null,
          maxlength: el.getAttribute('maxlength') ? +el.getAttribute('maxlength') : null,
          min: el.getAttribute('min') || null,
          max: el.getAttribute('max') || null,
          autocomplete: el.getAttribute('autocomplete') || null,
          disabled: el.disabled || false,
          readonly: el.readOnly || false,
          hidden: (type === 'hidden' || el.offsetParent === null),
          vue_model: el.getAttribute('v-model') || null,
          css_selector: buildSelector(el),
        };
        if (tag === 'select') {
          info.options = Array.from(el.options).map(o => ({
            value: o.value,
            label: o.textContent.trim(),
            selected: o.selected
          }));
        }
        return info;
      } catch(e) { return { error: e.message }; }
    }

    function findSubmit(container) {
      try {
        const btn = container.querySelector('button[type="submit"], input[type="submit"]');
        if (btn) return btn.textContent.trim() || btn.value || 'Submit';
        const buttons = container.querySelectorAll('button, [role="button"], input[type="button"]');
        if (buttons.length > 0) {
          const last = buttons[buttons.length - 1];
          return last.textContent.trim() || last.value || 'Submit';
        }
        return null;
      } catch(e) { return null; }
    }

    // 1. Standard <form> elements
    const forms = document.querySelectorAll('form');
    const formElements = new Set();
    forms.forEach((form, idx) => {
      try {
        const inputs = form.querySelectorAll('input, select, textarea');
        inputs.forEach(el => formElements.add(el));
        const fields = [];
        const hiddenFields = [];
        inputs.forEach(el => {
          const info = fieldInfo(el);
          if (info.type === 'hidden' || info.hidden) {
            hiddenFields.push(info);
          } else {
            fields.push(info);
          }
        });
        results.push({
          form_index: idx,
          source: 'form_tag',
          action: form.action || null,
          method: (form.method || 'get').toUpperCase(),
          id: form.id || null,
          name: form.getAttribute('name') || null,
          css_selector: buildSelector(form),
          fields: fields,
          hidden_fields: hiddenFields,
          submit_button: findSubmit(form),
          field_count: fields.length,
        });
      } catch(e) { results.push({ form_index: idx, error: e.message }); }
    });

    // 2. Detect orphan inputs (not inside any <form>)
    const allInputs = document.querySelectorAll('input, select, textarea');
    const orphans = Array.from(allInputs).filter(el => !formElements.has(el) && el.offsetParent !== null);
    if (orphans.length > 0) {
      const groups = new Map();
      orphans.forEach(el => {
        let container = el.closest('fieldset, [class*="form"], [class*="Form"], [data-form], section, .modal, .dialog, [role="form"]');
        if (!container) container = el.parentElement;
        const key = buildSelector(container);
        if (!groups.has(key)) groups.set(key, { container, elements: [] });
        groups.get(key).elements.push(el);
      });

      let vIdx = 0;
      groups.forEach(({ container, elements }, key) => {
        try {
          const fields = [];
          const hiddenFields = [];
          elements.forEach(el => {
            const info = fieldInfo(el);
            if (info.type === 'hidden') {
              hiddenFields.push(info);
            } else {
              fields.push(info);
            }
          });
          if (fields.length >= 2) {
            results.push({
              form_index: forms.length + vIdx,
              source: 'virtual_form',
              container_selector: key,
              fields: fields,
              hidden_fields: hiddenFields,
              submit_button: findSubmit(container),
              field_count: fields.length,
            });
            vIdx++;
          }
        } catch(e) {}
      });
    }

    // 3. Detect Vue/React form patterns via v-model / data-vv-name
    const vueInputs = document.querySelectorAll('[v-model], [data-vv-name], [v-bind\\:value]');
    const extraVue = Array.from(vueInputs).filter(el => !formElements.has(el));
    if (extraVue.length > 0) {
      const fields = extraVue.map(el => fieldInfo(el));
      const visible = fields.filter(f => !f.hidden);
      if (visible.length > 0) {
        results.push({
          form_index: results.length,
          source: 'vue_virtual',
          framework: 'vue',
          fields: visible,
          hidden_fields: fields.filter(f => f.hidden),
          field_count: visible.length,
        });
      }
    }

    return JSON.stringify({
      total_forms: results.length,
      forms: results,
      url: location.href,
    });
  } catch(e) {
    return JSON.stringify({ error: e.message, url: location.href });
  }
})()
"#;
        let result = self.eval_string(js).await?;
        eprintln!("[ENGINE] analyze_forms: {} bytes", result.len());
        Ok(result)
    }

    /// Analyze JS bundles on the page for API endpoint patterns.
    /// Looks at inline scripts, fetch/XHR patterns, and framework stores.
    pub async fn analyze_api_from_js(&self) -> Result<String, Box<dyn std::error::Error>> {
        let js = r#"
(async () => {
  try {
    const endpoints = [];
    const seen = new Set();

    const apiPatterns = [
      /["'`](\/api\/[^"'`\s]{3,})["'`]/g,
      /["'`](https?:\/\/[^"'`\s]*\/api\/[^"'`\s]{3,})["'`]/g,
      /["'`](\/v[0-9]+\/[^"'`\s]{3,})["'`]/g,
      /["'`](\/graphql[^"'`\s]*)["'`]/g,
      /["'`](\/rest\/[^"'`\s]{3,})["'`]/g,
      /["'`](\/ws\/[^"'`\s]{3,})["'`]/g,
      /fetch\s*\(\s*["'`]([^"'`\s]+)["'`]/g,
      /axios\s*\.\s*(?:get|post|put|patch|delete)\s*\(\s*["'`]([^"'`\s]+)["'`]/g,
      /\.(?:get|post|put|patch|delete)\s*\(\s*["'`](\/[^"'`\s]+)["'`]/g,
      /XMLHttpRequest[\s\S]{0,200}\.open\s*\(\s*["'`]\w+["'`]\s*,\s*["'`]([^"'`\s]+)["'`]/g,
      /baseURL\s*[:=]\s*["'`]([^"'`\s]+)["'`]/g,
    ];

    function extractFromText(text, source) {
      try {
        for (const pattern of apiPatterns) {
          pattern.lastIndex = 0;
          let match;
          while ((match = pattern.exec(text)) !== null) {
            const url = match[1];
            if (!seen.has(url) && url.length < 500 && !/\.(js|css|png|jpg|gif|svg|ico|woff|ttf|eot)($|\?)/.test(url)) {
              seen.add(url);
              let method = 'GET';
              const ctx = text.substring(Math.max(0, match.index - 100), match.index + match[0].length);
              if (/\.post\s*\(|method:\s*["']POST/i.test(ctx)) method = 'POST';
              else if (/\.put\s*\(|method:\s*["']PUT/i.test(ctx)) method = 'PUT';
              else if (/\.patch\s*\(|method:\s*["']PATCH/i.test(ctx)) method = 'PATCH';
              else if (/\.delete\s*\(|method:\s*["']DELETE/i.test(ctx)) method = 'DELETE';
              endpoints.push({ url, method, source });
            }
          }
        }
      } catch(e) {}
    }

    // 1. Inline scripts
    const scripts = document.querySelectorAll('script:not([src])');
    scripts.forEach((s, i) => {
      try {
        if (s.textContent.length > 0 && s.textContent.length < 500000) {
          extractFromText(s.textContent, 'inline_script_' + i);
        }
      } catch(e) {}
    });

    // 2. Window globals (SSR frameworks)
    const globalChecks = [
      '__NUXT__', '__NEXT_DATA__', '__APP_CONFIG__', '__VUE_SSR_CONTEXT__',
      '__INITIAL_STATE__', '__PRELOADED_STATE__',
    ];
    for (const g of globalChecks) {
      try {
        const obj = window[g];
        if (obj) {
          const text = JSON.stringify(obj).substring(0, 200000);
          extractFromText(text, 'global_' + g);
        }
      } catch(e) {}
    }

    // 3. Vuex/Pinia stores
    try {
      if (window.__vue_app__ || document.querySelector('[data-v-app]')) {
        const app = window.__vue_app__ || document.querySelector('[data-v-app]')?.__vue_app__;
        if (app && app.config && app.config.globalProperties) {
          const store = app.config.globalProperties.$store;
          if (store && store.state) {
            extractFromText(JSON.stringify(store.state).substring(0, 200000), 'vuex_store');
          }
          const pinia = app.config.globalProperties.$pinia;
          if (pinia && pinia.state && pinia.state.value) {
            extractFromText(JSON.stringify(pinia.state.value).substring(0, 200000), 'pinia_store');
          }
        }
      }
    } catch(e) {}

    // 4. External JS bundles (main/app/bundle only, max 3)
    const extScripts = Array.from(document.querySelectorAll('script[src]'))
      .filter(s => s.src && (s.src.includes('/chunk') || s.src.includes('/app') || s.src.includes('/main') || s.src.includes('/bundle') || s.src.includes('/vendor')))
      .slice(0, 3);

    for (const s of extScripts) {
      try {
        const resp = await fetch(s.src);
        if (resp.ok) {
          const text = await resp.text();
          if (text.length < 1000000) {
            extractFromText(text.substring(0, 500000), 'bundle_' + new URL(s.src).pathname.split('/').pop());
          }
        }
      } catch(e) {}
    }

    // 5. Meta tags with API hints
    const metas = document.querySelectorAll('meta[name*="api"], meta[name*="endpoint"], meta[property*="api"]');
    metas.forEach(m => {
      try {
        const val = m.content;
        if (val && val.startsWith('http')) {
          if (!seen.has(val)) {
            seen.add(val);
            endpoints.push({ url: val, method: 'GET', source: 'meta_tag' });
          }
        }
      } catch(e) {}
    });

    // 6. Manifest link
    try {
      const link = document.querySelector('link[rel="manifest"]');
      if (link && link.href) {
        endpoints.push({ url: link.href, method: 'GET', source: 'manifest' });
      }
    } catch(e) {}

    return JSON.stringify({
      total_endpoints: endpoints.length,
      endpoints: endpoints,
      url: location.href,
    });
  } catch(e) {
    return JSON.stringify({ error: e.message, url: location.href });
  }
})()
"#;
        let result = self.eval_string(js).await?;
        eprintln!("[ENGINE] analyze_api_from_js: {} bytes", result.len());
        Ok(result)
    }

    // ─── Workflow Mapper ───

    /// Start recording a new workflow.
    pub fn workflow_start(&mut self, name: &str) {
        let url = self.last_url.clone();
        self.active_workflow = Some(Workflow::new(name, &url));
        eprintln!("[WORKFLOW] Started: {name} at {url}");
    }

    /// Rich observation of the current page — captures everything an LLM needs
    /// to understand the page: interactive elements, form fields with full metadata,
    /// Vue/React component state, visible labels, errors, and dropdown options.
    pub async fn workflow_observe(&mut self) -> Result<Value, Box<dyn std::error::Error>> {
        let t0 = Instant::now();

        // 1. Run the rich observation JS in the browser
        let js = Self::WORKFLOW_OBSERVE_JS;
        let raw = self.eval_string(js).await?;
        let observation: Value = serde_json::from_str(&raw).unwrap_or_else(|_| json!({"raw": raw}));

        // 2. Also get analyze_forms output for deep field metadata
        let forms_raw = self.analyze_forms().await.unwrap_or_default();
        let forms: Value = serde_json::from_str(&forms_raw).unwrap_or(json!(null));

        // 3. Combine into a single rich observation
        let url = self.last_url.clone();
        let result = json!({
            "url": url,
            "page": observation,
            "forms": forms,
            "elapsed_ms": t0.elapsed().as_millis() as u64,
        });

        // 4. If a workflow is active, record this as an observe step
        if let Some(ref mut wf) = self.active_workflow {
            let step_num = wf.steps.len() + 1;
            let now = chrono_now();
            wf.steps.push(WorkflowStep {
                step_number: step_num,
                action: "observe".into(),
                target: None,
                value: None,
                url: url.clone(),
                observation: result.clone(),
                network_requests: Vec::new(),
                timestamp: now,
                notes: String::new(),
            });

            // Accumulate field_map from forms
            if let Some(forms_arr) = forms.get("forms").and_then(|f| f.as_array()) {
                for form in forms_arr {
                    if let Some(fields) = form.get("fields").and_then(|f| f.as_array()) {
                        for field in fields {
                            let label = field["label"].as_str().unwrap_or("");
                            let selector = field["css_selector"].as_str().unwrap_or("");
                            let name = field["name"].as_str().unwrap_or("");
                            if !label.is_empty() && !selector.is_empty() {
                                wf.field_map[label] = json!({
                                    "selector": selector,
                                    "name": name,
                                    "type": field["type"],
                                    "required": field["required"],
                                });
                            }
                        }
                    }
                }
            }

            // Accumulate vue_model_schema from observation
            if let Some(vue) = observation.get("vue_state") {
                if vue != &json!(null) {
                    wf.vue_model_schema = vue.clone();
                }
            }
        }

        eprintln!("[WORKFLOW] observe: {}ms", t0.elapsed().as_millis());
        Ok(result)
    }

    /// Perform an action during workflow recording: click, type, select, navigate.
    /// Captures network requests and page state changes.
    pub async fn workflow_act(
        &mut self,
        action: &str,
        target: &str,
        value: &str,
        notes: &str,
    ) -> Result<Value, Box<dyn std::error::Error>> {
        let t0 = Instant::now();
        let url_before = self.last_url.clone();

        // Start capturing network requests for this action
        let was_capturing = self.cdp_network_active.load(std::sync::atomic::Ordering::SeqCst);
        if !was_capturing {
            self.start_cdp_network_capture().await?;
        } else {
            // Drain existing entries so we only get new ones
            let mut entries = self.cdp_network_entries.lock().await;
            entries.clear();
        }

        // Perform the action
        let action_result = match action {
            "click" => {
                let (found, strategy) = self.click_reliable(target).await?;
                if found {
                    json!({"ok": true, "strategy": strategy})
                } else {
                    json!({"ok": false, "error": format!("target not found: {target}")})
                }
            }
            "type" => {
                self.focus(target).await?;
                self.type_text(value).await?;
                json!({"ok": true, "typed": value.len()})
            }
            "select" => {
                let found = self.select_option(target, value).await?;
                json!({"ok": found})
            }
            "navigate" | "goto" => {
                self.goto(target).await?;
                json!({"ok": true, "url": self.last_url.clone()})
            }
            "press" => {
                self.press(value).await?;
                json!({"ok": true, "key": value})
            }
            "scroll" => {
                let dir = if value.is_empty() { "down" } else { value };
                self.scroll(dir).await?;
                json!({"ok": true, "direction": dir})
            }
            "hover" => {
                let found = self.hover(target).await?;
                json!({"ok": found})
            }
            "wait" => {
                let secs: f64 = value.parse().unwrap_or(1.0);
                self.wait(secs).await;
                json!({"ok": true, "waited_secs": secs})
            }
            _ => {
                return Err(format!("Unknown workflow action: {action}").into());
            }
        };

        // Small delay for network requests to complete
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        // Read network requests made during the action
        let network = self.read_cdp_network(None).await.unwrap_or_default();

        // Filter to only API-relevant requests (skip static assets)
        let api_requests: Vec<Value> = network.into_iter()
            .filter(|r| {
                let url = r["url"].as_str().unwrap_or("");
                let rtype = r["resourceType"].as_str().unwrap_or("");
                // Keep XHR/Fetch/Document, skip Image/Stylesheet/Script/Font
                matches!(rtype, "XHR" | "Fetch" | "Document" | "Other")
                    || url.contains("/api/")
                    || url.contains("/graphql")
                    || (r["method"].as_str().unwrap_or("") != "GET"
                        && !url.ends_with(".js")
                        && !url.ends_with(".css"))
            })
            .collect();

        // Stop capturing if we started it
        if !was_capturing {
            self.stop_cdp_network_capture().await?;
        }

        // Capture page state after the action
        let page_after = self.see_page().await.unwrap_or_default();
        let url_after = self.last_url.clone();

        let result = json!({
            "action": action,
            "target": target,
            "value": value,
            "action_result": action_result,
            "url_before": url_before,
            "url_after": url_after,
            "url_changed": url_before != url_after,
            "network_requests": api_requests,
            "page_after_summary": page_after.chars().take(2000).collect::<String>(),
            "elapsed_ms": t0.elapsed().as_millis() as u64,
        });

        // Record in active workflow
        if let Some(ref mut wf) = self.active_workflow {
            let step_num = wf.steps.len() + 1;
            let now = chrono_now();

            // Accumulate discovered API endpoints
            for req in &api_requests {
                let endpoint = json!({
                    "url": req["url"],
                    "method": req["method"],
                    "status": req["status"],
                    "postData": req["postData"],
                    "discovered_at_step": step_num,
                });
                // Deduplicate by URL+method
                let key = format!("{}:{}", req["method"].as_str().unwrap_or(""), req["url"].as_str().unwrap_or(""));
                if !wf.api_endpoints_discovered.iter().any(|e| {
                    format!("{}:{}", e["method"].as_str().unwrap_or(""), e["url"].as_str().unwrap_or("")) == key
                }) {
                    wf.api_endpoints_discovered.push(endpoint);
                }
            }

            wf.steps.push(WorkflowStep {
                step_number: step_num,
                action: action.into(),
                target: Some(target.into()),
                value: if value.is_empty() { None } else { Some(value.into()) },
                url: url_after,
                observation: json!({"page_summary": page_after.chars().take(2000).collect::<String>()}),
                network_requests: api_requests,
                timestamp: now,
                notes: notes.into(),
            });
        }

        eprintln!("[WORKFLOW] act {action} {target}: {}ms", t0.elapsed().as_millis());
        Ok(result)
    }

    /// Save the current workflow as a reusable playbook.
    pub fn workflow_save(&self, path: &str) -> Result<String, Box<dyn std::error::Error>> {
        let wf = self.active_workflow.as_ref()
            .ok_or("No active workflow")?;

        let json = serde_json::to_string_pretty(wf)?;
        std::fs::write(path, &json)?;

        eprintln!("[WORKFLOW] Saved {} steps to {path}", wf.steps.len());
        Ok(format!("Saved workflow '{}' ({} steps, {} API endpoints, {} field mappings) to {path}",
            wf.name, wf.steps.len(), wf.api_endpoints_discovered.len(),
            wf.field_map.as_object().map(|m| m.len()).unwrap_or(0)))
    }

    /// Stop recording the current workflow.
    pub fn workflow_stop(&mut self) -> Result<Value, Box<dyn std::error::Error>> {
        let wf = self.active_workflow.take()
            .ok_or("No active workflow")?;

        let summary = json!({
            "name": wf.name,
            "steps": wf.steps.len(),
            "api_endpoints": wf.api_endpoints_discovered.len(),
            "field_mappings": wf.field_map.as_object().map(|m| m.len()).unwrap_or(0),
            "start_url": wf.start_url,
        });

        eprintln!("[WORKFLOW] Stopped: {}", wf.name);
        Ok(summary)
    }

    /// Load a saved workflow and replay it step by step.
    /// At each step, verifies the page matches expected state.
    /// Returns results per step and pauses on mismatches.
    pub async fn workflow_replay(&mut self, path: &str) -> Result<Value, Box<dyn std::error::Error>> {
        let json_str = std::fs::read_to_string(path)?;
        let wf: Workflow = serde_json::from_str(&json_str)?;
        let t0 = Instant::now();

        eprintln!("[WORKFLOW] Replaying '{}' ({} steps)", wf.name, wf.steps.len());

        let mut results = Vec::new();
        let mut stopped_at: Option<usize> = None;

        for step in &wf.steps {
            // Skip observe-only steps during replay
            if step.action == "observe" {
                results.push(json!({
                    "step": step.step_number,
                    "action": "observe",
                    "outcome": "skipped",
                }));
                continue;
            }

            let target = step.target.as_deref().unwrap_or("");
            let value = step.value.as_deref().unwrap_or("");

            // Verify we're on the expected URL (allow hash/query differences)
            let current_url = self.last_url.clone();
            let url_base = |u: &str| -> String {
                u.split('?').next().unwrap_or(u).split('#').next().unwrap_or(u).to_string()
            };
            let url_match = url_base(&current_url) == url_base(&step.url)
                || step.action == "navigate" || step.action == "goto";

            if !url_match {
                results.push(json!({
                    "step": step.step_number,
                    "action": step.action,
                    "outcome": "url_mismatch",
                    "expected_url": step.url,
                    "actual_url": current_url,
                }));
                stopped_at = Some(step.step_number);
                break;
            }

            // Execute the action
            let action_result = match step.action.as_str() {
                "click" => {
                    let (found, strategy) = self.click_reliable(target).await?;
                    if found {
                        json!({"ok": true, "strategy": strategy})
                    } else {
                        // Try using field_map selector as fallback
                        if let Some(selector) = wf.field_map.get(target)
                            .and_then(|f| f["selector"].as_str()) {
                            let found2 = self.click_css(selector).await.unwrap_or(false);
                            json!({"ok": found2, "fallback": "field_map_selector"})
                        } else {
                            json!({"ok": false, "error": "target not found"})
                        }
                    }
                }
                "type" => {
                    // Try to focus via field_map first
                    let focused = if let Some(selector) = wf.field_map.get(target)
                        .and_then(|f| f["selector"].as_str()) {
                        self.click_css(selector).await.unwrap_or(false)
                    } else {
                        self.focus(target).await.unwrap_or(false)
                    };
                    if focused {
                        self.type_text(value).await?;
                        json!({"ok": true})
                    } else {
                        json!({"ok": false, "error": "could not focus target"})
                    }
                }
                "select" => {
                    let found = self.select_option(target, value).await?;
                    json!({"ok": found})
                }
                "navigate" | "goto" => {
                    self.goto(target).await?;
                    json!({"ok": true})
                }
                "press" => {
                    self.press(value).await?;
                    json!({"ok": true})
                }
                "scroll" => {
                    let dir = if value.is_empty() { "down" } else { value };
                    self.scroll(dir).await?;
                    json!({"ok": true})
                }
                "wait" => {
                    let secs: f64 = value.parse().unwrap_or(1.0);
                    self.wait(secs).await;
                    json!({"ok": true})
                }
                _ => json!({"ok": false, "error": format!("unknown action: {}", step.action)}),
            };

            let ok = action_result["ok"].as_bool().unwrap_or(false);
            results.push(json!({
                "step": step.step_number,
                "action": step.action,
                "target": target,
                "outcome": if ok { "ok" } else { "failed" },
                "detail": action_result,
            }));

            if !ok {
                stopped_at = Some(step.step_number);
                break;
            }

            // Small delay between steps
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }

        let completed = results.iter().filter(|r| r["outcome"] == "ok").count();
        let total = wf.steps.iter().filter(|s| s.action != "observe").count();

        Ok(json!({
            "workflow": wf.name,
            "status": if stopped_at.is_some() { "paused" } else { "completed" },
            "stopped_at_step": stopped_at,
            "steps_completed": completed,
            "steps_total": total,
            "total_ms": t0.elapsed().as_millis() as u64,
            "results": results,
            "field_map": wf.field_map,
        }))
    }

    /// The rich observation JS — extracts everything about the current page.
    const WORKFLOW_OBSERVE_JS: &'static str = r#"
(() => {
  try {
    const result = {};

    // 1. Page metadata
    result.url = location.href;
    result.title = document.title;

    // 2. All visible text labels ordered top to bottom
    const labels = [];
    document.querySelectorAll('label, h1, h2, h3, h4, h5, h6, legend, .label, [class*="label"], th').forEach(el => {
      if (el.offsetParent === null && el.tagName !== 'BODY') return;
      const text = el.textContent.trim().substring(0, 120);
      if (text && text.length > 1) {
        const rect = el.getBoundingClientRect();
        labels.push({ text, y: Math.round(rect.top), tag: el.tagName.toLowerCase() });
      }
    });
    labels.sort((a, b) => a.y - b.y);
    result.labels = labels;

    // 3. All interactive elements with full metadata
    const interactive = [];
    document.querySelectorAll('input, textarea, select, button, [role="button"], [role="combobox"], [role="listbox"], [contenteditable="true"]').forEach(el => {
      if (el.offsetParent === null && el.getAttribute('type') !== 'hidden') return;
      const tag = el.tagName.toLowerCase();
      const type = el.getAttribute('type') || tag;
      const rect = el.getBoundingClientRect();

      const info = {
        tag,
        type,
        name: el.name || el.id || null,
        value: (type === 'password') ? (el.value ? '***' : '') : (el.value || ''),
        placeholder: el.getAttribute('placeholder') || null,
        'aria-label': el.getAttribute('aria-label') || null,
        required: el.required || el.hasAttribute('required') || el.getAttribute('aria-required') === 'true',
        disabled: el.disabled || false,
        readonly: el.readOnly || false,
        css_selector: buildSelector(el),
        y: Math.round(rect.top),
        x: Math.round(rect.left),
      };

      // Label discovery
      if (el.id) {
        const lbl = document.querySelector('label[for="' + CSS.escape(el.id) + '"]');
        if (lbl) info.label = lbl.textContent.trim();
      }
      if (!info.label) {
        const parent = el.closest('label');
        if (parent) {
          const clone = parent.cloneNode(true);
          clone.querySelectorAll('input,select,textarea,button').forEach(c => c.remove());
          info.label = clone.textContent.trim();
        }
      }
      if (!info.label && el.getAttribute('aria-label')) {
        info.label = el.getAttribute('aria-label');
      }

      // Select options
      if (tag === 'select') {
        info.options = Array.from(el.options).map(o => ({
          value: o.value, text: o.textContent.trim(), selected: o.selected
        }));
        info.selectedText = el.selectedIndex >= 0 ? el.options[el.selectedIndex].textContent.trim() : '';
      }

      // Validation attributes
      if (el.getAttribute('pattern')) info.pattern = el.getAttribute('pattern');
      if (el.getAttribute('minlength')) info.minlength = +el.getAttribute('minlength');
      if (el.getAttribute('maxlength')) info.maxlength = +el.getAttribute('maxlength');
      if (el.getAttribute('min')) info.min = el.getAttribute('min');
      if (el.getAttribute('max')) info.max = el.getAttribute('max');

      // Vue-specific
      if (el.getAttribute('v-model')) info.vue_model = el.getAttribute('v-model');
      if (el.getAttribute('data-vv-name')) info.vee_validate_name = el.getAttribute('data-vv-name');
      const vvRules = el.getAttribute('v-validate') || el.getAttribute('data-vv-rules');
      if (vvRules) info.validation_rules = vvRules;

      // Button text
      if (tag === 'button' || el.getAttribute('role') === 'button') {
        info.text = el.textContent.trim().substring(0, 80);
      }

      interactive.push(info);
    });
    interactive.sort((a, b) => a.y - b.y);
    result.interactive = interactive;

    // 4. Open dropdown/listbox options
    const openDropdowns = [];
    document.querySelectorAll('[role="listbox"], .dropdown-menu.show, .v-select__content, .vs__dropdown-menu, [class*="dropdown"][class*="open"], [class*="dropdown"][class*="show"], ul.show, [aria-expanded="true"] + ul, [aria-expanded="true"] + div').forEach(el => {
      if (el.offsetParent === null) return;
      const items = [];
      el.querySelectorAll('[role="option"], li, .dropdown-item, .v-list-item').forEach(item => {
        const text = item.textContent.trim().substring(0, 100);
        if (text) items.push({
          text,
          value: item.getAttribute('data-value') || item.getAttribute('value') || text,
          selected: item.classList.contains('selected') || item.getAttribute('aria-selected') === 'true',
        });
      });
      if (items.length > 0) {
        openDropdowns.push({ selector: buildSelector(el), items });
      }
    });
    result.open_dropdowns = openDropdowns;

    // 5. Error messages
    const errors = [];
    document.querySelectorAll('.error, .is-invalid, .has-error, [class*="error-message"], [class*="field-error"], [role="alert"], .invalid-feedback, .text-danger, .v-messages__message').forEach(el => {
      if (el.offsetParent === null) return;
      const text = el.textContent.trim();
      if (text && text.length > 1 && text.length < 200) {
        errors.push(text);
      }
    });
    result.errors = [...new Set(errors)];

    // 6. Vue/React component state extraction
    const vueState = extractVueState();
    if (vueState) result.vue_state = vueState;

    return JSON.stringify(result);
  } catch(e) {
    return JSON.stringify({ error: e.message });
  }

  function buildSelector(el) {
    try {
      if (el.id) return '#' + CSS.escape(el.id);
      if (el.name) return el.tagName.toLowerCase() + '[name="' + el.name + '"]';
      const parent = el.parentElement;
      if (!parent) return el.tagName.toLowerCase();
      const siblings = Array.from(parent.children).filter(c => c.tagName === el.tagName);
      const idx = siblings.indexOf(el) + 1;
      return el.tagName.toLowerCase() + ':nth-of-type(' + idx + ')';
    } catch(e) { return 'unknown'; }
  }

  function extractVueState() {
    try {
      // Strategy 1: Vue 3 app instance (Pinia/Vuex)
      const appEl = document.querySelector('[data-v-app]') || document.getElementById('app') || document.getElementById('__nuxt');
      if (appEl && appEl.__vue_app__) {
        const app = appEl.__vue_app__;
        const state = {};

        // Vuex store
        try {
          const store = app.config.globalProperties.$store;
          if (store && store.state) {
            state.vuex = JSON.parse(JSON.stringify(store.state));
          }
        } catch(e) {}

        // Pinia stores
        try {
          const pinia = app.config.globalProperties.$pinia;
          if (pinia && pinia.state && pinia.state.value) {
            state.pinia = JSON.parse(JSON.stringify(pinia.state.value));
          }
        } catch(e) {}

        if (Object.keys(state).length > 0) return state;
      }

      // Strategy 2: Vue 2 — walk from inputs up to find form component data
      const inputs = document.querySelectorAll('input, select, textarea');
      const formModels = {};
      let found = false;
      for (const input of inputs) {
        try {
          let el = input;
          // Walk up to find __vue__ instance with form data
          for (let i = 0; i < 15 && el; i++) {
            const vm = el.__vue__;
            if (!vm) { el = el.parentElement; continue; }

            // Check for ValidationProvider (vee-validate)
            if (vm.$options && vm.$options.name === 'ValidationProvider') {
              const field = vm.fieldName || vm.name || ('field_' + i);
              formModels['vee_' + field] = {
                name: field,
                rules: vm.rules,
                value: vm.value,
                errors: vm.errors,
                valid: vm.flags && vm.flags.valid,
              };
              found = true;
            }

            // Check for component with localForm / form / formData
            const data = vm.$data;
            if (data) {
              for (const key of ['localForm', 'form', 'formData', 'model', 'formModel']) {
                if (data[key] && typeof data[key] === 'object') {
                  formModels[vm.$options.name || 'component'] = JSON.parse(JSON.stringify(data[key]));
                  found = true;
                  break;
                }
              }
            }

            if (found) break;
            el = el.parentElement;
          }
        } catch(e) {}
      }

      if (found) return formModels;

      // Strategy 3: React fiber state
      try {
        const reactRoot = document.getElementById('root') || document.getElementById('__next');
        if (reactRoot && reactRoot._reactRootContainer) {
          return { react: 'detected_but_state_extraction_not_implemented' };
        }
      } catch(e) {}

      return null;
    } catch(e) { return null; }
  }
})()
"#;

    // ─── Lifecycle ───

    pub async fn wait(&self, secs: f64) {
        tokio::time::sleep(std::time::Duration::from_secs_f64(secs)).await;
    }

    pub async fn close(mut self) -> Result<(), Box<dyn std::error::Error>> {
        // Chrome with --user-data-dir does NOT persist cookies to SQLite on its own.
        // Export all cookies via CDP and write them to the profile database ourselves.
        if let Ok(cookies) = self.get_all_cookies().await {
            if !cookies.is_empty() {
                let profile_dir = if self.connected_mode {
                    None // Don't write to user's Chrome profile
                } else {
                    Some(default_profile_dir())
                };
                if let Some(dir) = profile_dir {
                    match save_cookies_to_profile(&dir, &cookies) {
                        Ok(n) => eprintln!("[ENGINE] Saved {n} cookies to profile"),
                        Err(e) => eprintln!("[ENGINE] Cookie save warning: {e}"),
                    }
                }
            }
        }

        // Close the browser gracefully
        let _ = browser_domain::close(&self.cdp).await;

        // Wait for Chrome to finish before killing
        if let Some(ref mut child) = self.chrome_process {
            match tokio::time::timeout(
                std::time::Duration::from_secs(3),
                child.wait(),
            ).await {
                Ok(_) => {}
                Err(_) => {
                    eprintln!("[ENGINE] Chrome didn't exit in 3s, killing");
                    let _ = child.kill().await;
                }
            }
        }

        eprintln!("[ENGINE] Closed");
        Ok(())
    }
}
