//! NeoBrowser-RS: AI browser with dual engine + WOM output.
//!
//! LIGHT: reqwest + html5ever (no Chrome, no JS, fast)
//! CHROME: chromiumoxide CDP (full browser, JS, cookies)
//! VISION: intelligent page perception (type, state, actions, content)
//! WOM: Web Object Model — AI-native structured output with stable IDs

mod auth;
mod cdp;
// mod chrome; // Replaced by engine (raw CDP, no chromiumoxide)
mod delta;
mod engine;
mod mcp;
mod semantic;
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
    let client = reqwest::Client::builder()
        .user_agent(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
             AppleWebKit/537.36 (KHTML, like Gecko) \
             Chrome/134.0.0.0 Safari/537.36",
        )
        .gzip(true)
        .brotli(true)
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()?;

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
    let client = reqwest::Client::builder()
        .user_agent(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
             AppleWebKit/537.36 (KHTML, like Gecko) \
             Chrome/134.0.0.0 Safari/537.36",
        )
        .gzip(true)
        .brotli(true)
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()?;

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
    let client = reqwest::Client::builder()
        .user_agent(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
             AppleWebKit/537.36 (KHTML, like Gecko) \
             Chrome/134.0.0.0 Safari/537.36",
        )
        .gzip(true)
        .brotli(true)
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()?;

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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Command::Mcp => mcp::run().await,
        Command::Fetch { url, lines } => fetch_and_see(&url, lines).await,
        Command::Browse { url, lines } => browse_and_see(&url, lines).await,
        Command::See { url, lines } => auto_see(&url, lines).await,
        Command::Wom { url, compact } => wom_see(&url, compact).await,
        Command::Session {
            cookies, url, lines, port, connect, profile,
        } => run_session(cookies, url, lines, port, connect, profile).await,
    }
}
