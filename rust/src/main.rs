//! NeoBrowser — Rust port.
//!
//! A fast, stealthy MCP browser-automation server that drives real Chrome.
//! This binary currently exposes a `doctor` subcommand while the MCP server
//! (Phase 2) and tools (Phase 3+) are ported. See `src/cdp.rs` for the CDP core
//! and `src/chrome.rs` for the process manager.

use neobrowser::{cdp, chrome, mcp, paths};

use std::time::Duration;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let arg = std::env::args().nth(1).unwrap_or_default();
    match arg.as_str() {
        "doctor" => doctor().await,
        "--version" | "-v" => println!("{}", env!("CARGO_PKG_VERSION")),
        "" | "serve" => mcp::serve().await,
        other => {
            eprintln!("neobrowser: unknown command '{other}'. Try `doctor`.");
            std::process::exit(2);
        }
    }
}

/// Report Chrome discovery, version, and home-dir layout — the Rust equivalent of
/// the Python `neobrowser doctor`.
async fn doctor() {
    let bin = chrome::chrome_bin();
    println!("NeoBrowser doctor");
    println!("  home:        {}", paths::home().display());
    println!("  chrome bin:  {}", bin.display());
    println!("  chrome found: {}", bin.exists());
    match chrome::detect_chrome_major(bin) {
        Some(major) => println!("  chrome major: {major}"),
        None => println!("  chrome major: <unknown>"),
    }
    match chrome::chrome_user_agent() {
        Some(ua) => println!("  user-agent:  {ua}"),
        None => println!("  user-agent:  <default>"),
    }
    // Prove the process/CDP path works end-to-end if Chrome is present.
    if bin.exists() {
        print!("  launch+CDP:  ");
        match smoke_test().await {
            Ok(title) => println!("ok (about:blank title={title:?})"),
            Err(e) => println!("FAILED: {e}"),
        }
    }
}

/// Launch headless Chrome, open a tab, connect CDP, evaluate a trivial expression.
async fn smoke_test() -> Result<String, String> {
    let profile = paths::profiles_base().join("doctor");
    let mut proc = chrome::ChromeProcess::launch(&profile)
        .await
        .map_err(|e| e.to_string())?;
    let result = async {
        chrome::wait_for_chrome(proc.port, Duration::from_secs(10))
            .await
            .map_err(|e| e.to_string())?;
        let tab = chrome::open_new_tab(proc.port)
            .await
            .map_err(|e| e.to_string())?;
        let client = cdp::CdpClient::connect(&tab.web_socket_debugger_url)
            .await
            .map_err(|e| e.to_string())?;
        client
            .send("Page.enable", serde_json::json!({}))
            .await
            .map_err(|e| e.to_string())?;
        let title = client
            .eval("document.title")
            .await
            .map_err(|e| e.to_string())?;
        Ok::<String, String>(title.as_str().unwrap_or("").to_string())
    }
    .await;
    proc.kill(true).await;
    result
}
