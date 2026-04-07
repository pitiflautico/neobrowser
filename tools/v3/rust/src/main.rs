//! NeoBrowser-RS: AI browser with dual engine + WOM output.
//!
//! LIGHT: rquest + html5ever (no Chrome, no JS, fast)
//! CHROME: chromiumoxide CDP (full browser, JS, cookies)
//! VISION: intelligent page perception (type, state, actions, content)
//! WOM: Web Object Model — AI-native structured output with stable IDs

mod auth;
mod cdp;
mod cors_proxy;
mod delta;
mod engine;
mod ghost;
mod http_client;
mod identity;
mod mcp;
mod neorender;
mod pool;
mod runner;
mod semantic;
mod stealth;
mod trace;
mod vision;
mod wom;

use clap::{Parser, Subcommand};
use html5ever::parse_document;
use html5ever::tendril::TendrilSink;
use markup5ever_rcdom::RcDom;
use std::io::{self, BufRead, Write};
use std::time::Instant;

#[derive(Parser)]
#[command(name = "neobrowser", about = "AI browser — see the web semantically")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Light mode: fetch + parse (no Chrome, no JS)
    Fetch {
        url: String,
        #[arg(short, long, default_value = "80")]
        lines: usize,
    },
    /// Chrome mode: full browser (one-shot)
    Browse {
        url: String,
        #[arg(short, long, default_value = "80")]
        lines: usize,
    },
    /// Auto: try light, fall back to Chrome if JS needed
    See {
        url: String,
        #[arg(short, long, default_value = "80")]
        lines: usize,
    },
    /// WOM output: structured JSON for AI agents (light mode)
    Wom {
        url: String,
        /// Output compact format instead of full WOM
        #[arg(long)]
        compact: bool,
    },
    /// MCP server mode (JSON-RPC over stdio for AI agents)
    Mcp,
    /// CORS proxy: relay requests with permissive CORS headers
    Proxy {
        #[arg(short, long, default_value = "8888")]
        port: u16,
    },
    /// First-time setup: opens Chrome, guides you to login, saves profile
    Setup {
        /// Sites to login to (e.g. linkedin.com,chatgpt.com)
        #[arg(short, long)]
        sites: Option<String>,
    },
    /// Login to a site: opens visible Chrome, you login, cookies are saved.
    /// Use this for sites with CAPTCHAs, 2FA, or complex auth flows.
    /// After login, cookies persist for headless use.
    Login {
        /// URL to open (e.g. https://linkedin.com or just linkedin.com)
        url: String,
        /// Custom profile directory (default: ~/.neobrowser/profile)
        #[arg(short, long)]
        profile: Option<String>,
        /// Additional sites to open in new tabs
        #[arg(short, long)]
        also: Option<String>,
    },
    /// Interactive session with full browser
    Session {
        /// Cookie file (Playwright storageState or JSON array)
        #[arg(short, long)]
        cookies: Option<String>,
        /// Starting URL
        #[arg(short, long)]
        url: Option<String>,
        /// Max content lines per see
        #[arg(short, long, default_value = "50")]
        lines: usize,
        /// Connect to existing Chrome on this port
        #[arg(short, long)]
        port: Option<u16>,
        /// Connect to the user's running Chrome (reads DevToolsActivePort)
        #[arg(long)]
        connect: bool,
        /// Use Chrome user-data-dir (gets all cookies/sessions from profile)
        #[arg(long)]
        profile: Option<String>,
    },
}

// ─── Light mode ───

async fn fetch_and_see(url: &str, max_lines: usize) -> Result<(), Box<dyn std::error::Error>> {
    let t0 = Instant::now();
    let client = http_client::light()?;

    let resp = client.get(url).send().await?;
    let status = resp.status();
    let html = resp.text().await?;

    let dom = parse_document(RcDom::default(), Default::default())
        .from_utf8()
        .read_from(&mut html.as_bytes())?;

    let view = vision::analyze(&dom.document, url, "");

    let needs_js = html.contains("__NEXT_DATA__")
        || html.contains("window.__INITIAL_STATE__")
        || html.contains(r#"id="root"></div>"#)
        || html.contains(r#"id="app"></div>"#)
        || (view.content.len() < 3 && html.len() > 1000);

    eprintln!(
        "[LIGHT] {status} | {:.1}KB HTML | {}ms",
        html.len() as f64 / 1024.0,
        t0.elapsed().as_millis(),
    );
    if needs_js {
        eprintln!("[LIGHT] ⚠ Page likely needs JS — use `browse` or `see`");
    }

    println!("{}", vision::format_view(&view, max_lines));
    Ok(())
}

// ─── Chrome one-shot ───

async fn browse_and_see(url: &str, max_lines: usize) -> Result<(), Box<dyn std::error::Error>> {
    let mut session = engine::Session::launch(None).await?;
    session.goto(url).await?;
    let view = session.see().await?;
    println!("{}", vision::format_view(&view, max_lines));
    session.close().await?;
    Ok(())
}

// ─── WOM mode (light) ───

async fn wom_see(url: &str, compact_mode: bool) -> Result<(), Box<dyn std::error::Error>> {
    let t0 = Instant::now();
    let client = http_client::light()?;

    let resp = client.get(url).send().await?;
    let html = resp.text().await?;
    let html_bytes = html.len();

    let dom = parse_document(RcDom::default(), Default::default())
        .from_utf8()
        .read_from(&mut html.as_bytes())?;

    let doc = wom::build(&dom.document, url, "", html_bytes, "light", 1);

    eprintln!(
        "[WOM] {} | {} nodes | {} actions | {:.1}x compression | {}ms",
        doc.page.page_class,
        doc.nodes.len(),
        doc.actions.len(),
        doc.compression.compression_ratio,
        t0.elapsed().as_millis(),
    );

    if compact_mode {
        let c = wom::compact(&doc);
        println!("{}", wom::format_compact(&c));
    } else {
        println!("{}", wom::format_json(&doc));
    }
    Ok(())
}

// ─── Auto mode ───

async fn auto_see(url: &str, max_lines: usize) -> Result<(), Box<dyn std::error::Error>> {
    let client = http_client::light()?;

    let resp = client.get(url).send().await?;
    let html = resp.text().await?;

    let dom = parse_document(RcDom::default(), Default::default())
        .from_utf8()
        .read_from(&mut html.as_bytes())?;

    let mut raw_lines = Vec::new();
    semantic::walk(&dom.document, 0, &mut raw_lines);

    let needs_js = html.contains("__NEXT_DATA__")
        || html.contains("window.__INITIAL_STATE__")
        || html.contains(r#"id="root"></div>"#)
        || html.contains(r#"id="app"></div>"#)
        || (raw_lines.len() < 5 && html.len() > 1000);

    if needs_js {
        eprintln!("[AUTO] Light got {} lines — needs JS, using Chrome...", raw_lines.len());
        browse_and_see(url, max_lines).await
    } else {
        let view = vision::analyze(&dom.document, url, "");
        eprintln!("[AUTO] Light mode ✓");
        println!("{}", vision::format_view(&view, max_lines));
        Ok(())
    }
}

// ─── Interactive session ───

/// Check if a string looks like a WOM ID (e.g. btn_042, lnk_015, fld_003)
fn is_wom_id(s: &str) -> bool {
    let parts: Vec<&str> = s.splitn(2, '_').collect();
    if parts.len() != 2 { return false; }
    let prefix = parts[0];
    let suffix = parts[1];
    matches!(prefix, "btn" | "lnk" | "fld" | "h" | "sel" | "form" | "img" | "p")
        && suffix.chars().all(|c| c.is_ascii_digit())
}

async fn run_session(
    cookies: Option<String>,
    url: Option<String>,
    max_lines: usize,
    port: Option<u16>,
    connect: bool,
    profile: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Persist cookies to profile BEFORE launching Chrome (writes to SQLite)
    if let Some(cookie_path) = &cookies {
        if !connect && port.is_none() {
            let profile_dir = match &profile {
                Some(p) => std::path::PathBuf::from(p),
                None => engine::default_profile_dir(),
            };
            engine::persist_cookies_to_profile(&profile_dir, cookie_path)?;
        }
    }

    // Create session — connect to existing or launch new
    let mut session = if connect {
        engine::Session::connect_running().await?
    } else if let Some(p) = port {
        engine::Session::connect_port(p).await?
    } else {
        engine::Session::launch(profile.as_deref()).await?
    };

    // Also inject via CDP for immediate availability (no restart needed)
    if let Some(cookie_path) = &cookies {
        session.load_cookies(cookie_path).await?;
    }
    if let Some(start_url) = &url {
        session.goto(start_url).await?;
    }

    let mut wom_revision: u64 = 0;
    let mut prev_wom: Option<wom::WomDocument> = None;

    eprintln!("NeoBrowser session. Commands:");
    eprintln!("  goto <url>       see              click <text>");
    eprintln!("  focus <text>     type <text>       press <key>");
    eprintln!("  scroll <dir>     back / forward    reload");
    eprintln!("  tabs             tab <n>           screenshot");
    eprintln!("  eval <js>        cookies <file>    raw");
    eprintln!("  wom              wom-compact        wom-delta");
    eprintln!("  wait <secs>      quit");
    eprintln!();

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    loop {
        eprint!("neo> ");
        stdout.flush()?;

        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {
            break;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let (cmd, arg) = match line.split_once(' ') {
            Some((c, a)) => (c, a.trim()),
            None => (line, ""),
        };

        let result: Result<(), Box<dyn std::error::Error>> = async {
            match cmd {
                "goto" | "go" | "nav" => {
                    if arg.is_empty() {
                        eprintln!("  usage: goto <url>");
                    } else {
                        session.goto(arg).await?;
                    }
                }
                "see" | "look" | "view" => {
                    let view = session.see().await?;
                    println!("{}", vision::format_view(&view, max_lines));
                }
                "raw" => {
                    let lines = session.see_raw().await?;
                    for l in lines.iter().take(max_lines) {
                        println!("{l}");
                    }
                    if lines.len() > max_lines {
                        eprintln!("... ({} more)", lines.len() - max_lines);
                    }
                }
                "wom" | "json" => {
                    wom_revision += 1;
                    let doc = session.see_wom(wom_revision).await?;
                    println!("{}", wom::format_json(&doc));
                    prev_wom = Some(doc);
                }
                "wom-compact" | "wc" => {
                    wom_revision += 1;
                    let doc = session.see_wom(wom_revision).await?;
                    let c = wom::compact(&doc);
                    println!("{}", wom::format_compact(&c));
                    prev_wom = Some(doc);
                }
                "wom-delta" | "wd" | "delta" => {
                    wom_revision += 1;
                    let doc = session.see_wom(wom_revision).await?;
                    if let Some(ref prev) = prev_wom {
                        let d = delta::diff(prev, &doc);
                        println!("{}", serde_json::to_string_pretty(&d).unwrap_or_default());
                    } else {
                        eprintln!("  No previous revision — showing full WOM");
                        let c = wom::compact(&doc);
                        println!("{}", wom::format_compact(&c));
                    }
                    prev_wom = Some(doc);
                }
                "click" | "cl" => {
                    if arg.is_empty() {
                        eprintln!("  usage: click <text|wom_id>");
                    } else if is_wom_id(arg) {
                        session.click_by_wom_id(arg).await?;
                    } else {
                        session.click(arg).await?;
                    }
                }
                "focus" | "fo" => {
                    if !arg.is_empty() && is_wom_id(arg) {
                        session.focus_by_wom_id(arg).await?;
                    } else {
                        session.focus(if arg.is_empty() { "" } else { arg }).await?;
                    }
                }
                "type" | "ty" => {
                    if arg.is_empty() {
                        eprintln!("  usage: type <text>");
                    } else {
                        session.type_text(arg).await?;
                    }
                }
                "press" | "key" => {
                    session.press(if arg.is_empty() { "Enter" } else { arg }).await?;
                }
                "scroll" | "sc" => {
                    session.scroll(if arg.is_empty() { "down" } else { arg }).await?;
                }
                "back" => { session.back().await?; }
                "forward" | "fwd" => { session.forward().await?; }
                "reload" | "refresh" => { session.reload().await?; }
                "tabs" | "pages" => {
                    let tabs = session.pages().await?;
                    for (i, t) in tabs.iter().enumerate() {
                        println!("  [{i}] {t}");
                    }
                }
                "tab" => {
                    let idx: usize = arg.parse().unwrap_or(0);
                    session.switch_tab(idx).await?;
                }
                "screenshot" | "shot" => {
                    let data = session.screenshot().await?;
                    let path = "/tmp/neo_screenshot.jpg";
                    std::fs::write(path, &data)?;
                    println!("  Saved to {path} ({}KB)", data.len() / 1024);
                }
                "eval" | "js" => {
                    if arg.is_empty() {
                        eprintln!("  usage: eval <js>");
                    } else {
                        let result = session.eval(arg).await?;
                        println!("{result}");
                    }
                }
                "cookies" | "cookie" => {
                    if arg.is_empty() {
                        eprintln!("  usage: cookies <file>");
                    } else {
                        session.load_cookies(arg).await?;
                    }
                }
                "wait" | "sleep" => {
                    let secs: f64 = arg.parse().unwrap_or(1.0);
                    session.wait(secs).await;
                    eprintln!("  waited {secs}s");
                }
                "quit" | "exit" | "q" => {
                    return Err("quit".into());
                }
                _ => {
                    eprintln!("  unknown: {cmd}");
                }
            }
            Ok(())
        }
        .await;

        match result {
            Err(e) if e.to_string() == "quit" => break,
            Err(e) => eprintln!("  error: {e}"),
            Ok(()) => {}
        }
    }

    session.close().await?;
    Ok(())
}

// ─── Setup wizard ───

async fn run_setup(sites: Option<String>) -> Result<(), Box<dyn std::error::Error>> {
    let profile_dir = engine::default_profile_dir();
    let profile_str = profile_dir.to_string_lossy();

    println!();
    println!("  ╔══════════════════════════════════════════╗");
    println!("  ║         NeoBrowser — First Setup          ║");
    println!("  ╚══════════════════════════════════════════╝");
    println!();
    println!("  Profile: {profile_str}");
    println!();
    println!("  A Chrome window will open. Log into any sites you");
    println!("  want NeoBrowser to access (LinkedIn, ChatGPT, etc).");
    println!();
    println!("  Your sessions will persist across restarts.");
    println!("  NeoBrowser never sees your passwords — only cookies.");
    println!();

    // Launch headed Chrome
    let mut session = engine::Session::launch_ex(None, false).await?;

    // Navigate to sites if provided
    let urls: Vec<String> = if let Some(ref s) = sites {
        s.split(',')
            .map(|s| {
                let s = s.trim();
                if s.starts_with("http") {
                    s.to_string()
                } else {
                    format!("https://{s}")
                }
            })
            .collect()
    } else {
        vec!["https://www.google.com".to_string()]
    };

    // Open first site
    if let Some(url) = urls.first() {
        println!("  Opening {url}...");
        session.goto(url).await?;
    }

    println!();
    println!("  ✓ Chrome is open. Log into your sites now.");
    println!();
    println!("  When you're done, type 'done' and press Enter.");
    println!("  Type a URL to navigate somewhere else.");
    println!();

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    loop {
        print!("  neobrowser> ");
        stdout.flush()?;

        let mut line = String::new();
        stdin.lock().read_line(&mut line)?;
        let input = line.trim();

        if input.is_empty() {
            continue;
        }

        if input == "done" || input == "quit" || input == "exit" {
            break;
        }

        // Navigate to URL
        let url = if input.starts_with("http") {
            input.to_string()
        } else {
            format!("https://{input}")
        };
        println!("  Opening {url}...");
        if let Err(e) = session.goto(&url).await {
            println!("  Error: {e}");
        }
    }

    // Save cookies
    println!();
    println!("  Saving session...");

    // Close triggers save_cookies_to_profile
    session.close().await?;

    println!("  ✓ Session saved to {profile_str}");
    println!();
    println!("  Next steps:");
    println!("  1. Add to Claude Code:");
    println!("     {{");
    println!("       \"mcpServers\": {{");
    println!("         \"neobrowser\": {{");
    println!("           \"type\": \"stdio\",");
    println!("           \"command\": \"neobrowser_rs\",");
    println!("           \"args\": [\"mcp\"]");
    println!("         }}");
    println!("       }}");
    println!("     }}");
    println!();
    println!("  2. Or run interactively:");
    println!("     neobrowser_rs session");
    println!();
    println!("  Environment variables:");
    println!("     NEOBROWSER_HEADLESS=1   Run without visible window");
    println!("     NEOBROWSER_PROFILE=path Custom profile directory");
    println!();

    Ok(())
}

// ─── Login command ───

async fn run_login(url: &str, profile: Option<String>, also: Option<String>) -> Result<(), Box<dyn std::error::Error>> {
    let url = if url.starts_with("http") { url.to_string() } else { format!("https://{url}") };
    let profile_dir = match &profile {
        Some(p) => std::path::PathBuf::from(p),
        None => engine::default_profile_dir(),
    };
    let profile_str = profile_dir.to_string_lossy().to_string();

    println!();
    println!("  NeoBrowser Login");
    println!("  ────────────────");
    println!("  Profile: {profile_str}");
    println!("  Opening: {url}");
    println!();
    println!("  A Chrome window will open. Log in normally.");
    println!("  Handle any CAPTCHAs, 2FA, or verification steps.");
    println!("  Your cookies will be saved when you're done.");
    println!();

    // Launch headed Chrome (never headless for login)
    let mut session = engine::Session::launch_ex(profile.as_deref(), false).await?;
    session.goto(&url).await?;

    // Open additional sites in background if requested
    if let Some(ref sites) = also {
        for site in sites.split(',') {
            let site = site.trim();
            let site_url = if site.starts_with("http") { site.to_string() } else { format!("https://{site}") };
            eprintln!("  Also opening: {site_url}");
            // Open in new tab via eval
            session.eval_string(&format!("window.open('{site_url}', '_blank')")).await.ok();
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }

    println!("  ✓ Chrome is ready. Log in to your sites.");
    println!();
    println!("  Commands:");
    println!("    done     — save cookies and exit");
    println!("    open URL — navigate to another site");
    println!("    status   — show captured cookies count");
    println!();

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    loop {
        print!("  login> ");
        stdout.flush()?;

        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 { break; }
        let input = line.trim();
        if input.is_empty() { continue; }

        match input {
            "done" | "quit" | "exit" | "q" => break,
            "status" => {
                let cookies = session.eval_string("document.cookie.split(';').length").await.unwrap_or_default();
                let url = session.eval_string("document.URL").await.unwrap_or_default();
                println!("  URL: {url}");
                println!("  Cookies (JS-visible): {cookies}");
            }
            _ => {
                let nav_url = if input.starts_with("http") || input.starts_with("open ") {
                    input.trim_start_matches("open ").trim().to_string()
                } else {
                    format!("https://{input}")
                };
                let nav_url = if nav_url.starts_with("http") { nav_url } else { format!("https://{nav_url}") };
                println!("  Opening {nav_url}...");
                if let Err(e) = session.goto(&nav_url).await {
                    println!("  Error: {e}");
                }
            }
        }
    }

    println!();
    println!("  Saving cookies...");
    session.close().await?;
    println!("  ✓ Cookies saved to {profile_str}");
    println!();
    println!("  Now use headless mode:");
    println!("    NEOBROWSER_HEADLESS=1 neobrowser_rs mcp");
    println!("  Or via npx:");
    println!("    npx neobrowser login {url}  (to re-login later)");
    println!();

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Command::Mcp => mcp::run().await,
        Command::Fetch { url, lines } => fetch_and_see(&url, lines).await,
        Command::Browse { url, lines } => browse_and_see(&url, lines).await,
        Command::See { url, lines } => auto_see(&url, lines).await,
        Command::Wom { url, compact } => wom_see(&url, compact).await,
        Command::Proxy { port } => cors_proxy::run(port).await,
        Command::Setup { sites } => run_setup(sites).await,
        Command::Login { url, profile, also } => run_login(&url, profile, also).await,
        Command::Session {
            cookies, url, lines, port, connect, profile,
        } => run_session(cookies, url, lines, port, connect, profile).await,
    }
}
