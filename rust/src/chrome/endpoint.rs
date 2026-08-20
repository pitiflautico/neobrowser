//! Talking to Chrome's HTTP endpoint: ports, readiness, and tab lifecycle.
//!
//! `wait_for_chrome` is where a launch failure becomes a useful message rather than a
//! timeout. When the port never opens, the interesting information is in Chrome's stderr —
//! usually one line about a missing library or a locked profile — so it is captured and
//! returned instead of discarded.

use std::time::{Duration, Instant};

use serde::Deserialize;

use super::ChromeError;
use crate::paths;

pub(super) fn validate_port(port: u16) -> Result<(), ChromeError> {
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
    let joined = body.join("\n  ");
    // `sandbox_support` only reports blockers it can prove, so a host it cleared
    // can still fail on one it couldn't see (seccomp filters, restricted
    // containers, an unreadable /proc). Chrome names the sandbox in stderr when
    // that happens — turn that into the same actionable advice instead of an
    // opaque timeout that invites the user to reach for --no-sandbox blindly.
    let sandbox_hint = if joined.to_ascii_lowercase().contains("sandbox") {
        "\nThis looks like a sandbox failure. Prefer fixing the host (run as a \
         non-root user; enable unprivileged user namespaces) over disabling the \
         sandbox. NEOBROWSER_ALLOW_NO_SANDBOX=1 exists as a last resort and is \
         refused outright together with NEOBROWSER_REAL_PROFILE."
    } else {
        ""
    };
    format!(".\nchrome stderr:\n  {joined}{sandbox_hint}")
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
