//! NeoRender V2 — AI Browser Engine
//! Binary entry point: CLI + MCP server mode.

use std::sync::{Arc, Mutex};

use neo_dom::Html5everDom;
use neo_engine::config::EngineConfig;
use neo_engine::{BrowserEngine, NeoSession};
use neo_extract::DefaultExtractor;
use neo_http::{DiskCache, RquestClient, SqliteCookieStore};
use neo_interact::DomInteractor;
use neo_trace::file_tracer::FileTracer;
use neo_trace::noop::NoopTracer;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    match args.get(1).map(|s| s.as_str()) {
        Some("mcp") => run_mcp(),
        Some("see") => run_see(&args),
        Some("--help") | Some("-h") | None => print_help(),
        Some(cmd) => {
            eprintln!("Unknown command: {cmd}");
            print_help();
            std::process::exit(1);
        }
    }
}

/// Create the real browser engine with all subsystems wired together.
fn create_engine() -> NeoSession {
    let http = RquestClient::default();
    let dom: Box<dyn neo_dom::DomEngine> = Box::new(Html5everDom::new());
    let shared_dom = Arc::new(Mutex::new(dom));

    let interactor = DomInteractor::new(Arc::clone(&shared_dom));
    let extractor = DefaultExtractor::new();
    let tracer = FileTracer::new(None);
    let lifecycle_tracer = NoopTracer::new();

    let mut config = EngineConfig::default();
    // Disable JS execution — no V8 runtime wired yet.
    config.execute_js = false;

    let cookie_store = SqliteCookieStore::default_store()
        .expect("failed to open cookie store at ~/.neorender/cookies.db");
    let disk_cache = DiskCache::default_cache()
        .expect("failed to open disk cache at ~/.neorender/cache/http/");

    NeoSession::new_shared(
        Box::new(http),
        shared_dom,
        None, // No JS runtime yet
        Box::new(interactor),
        Box::new(extractor),
        Box::new(tracer),
        Box::new(lifecycle_tracer),
        config,
    )
    .with_cookie_store(Box::new(cookie_store))
    .with_http_cache(Box::new(disk_cache))
}

fn run_mcp() {
    eprintln!("[NeoRender V2] MCP server starting...");
    let engine = create_engine();
    if let Err(e) = neo_mcp::run_server(Box::new(engine)) {
        eprintln!("[NeoRender V2] MCP server error: {e}");
        std::process::exit(1);
    }
}

fn run_see(args: &[String]) {
    // Parse: neorender see [--cookies <path>] <url>
    let mut cookies_path: Option<&str> = None;
    let mut url: Option<&str> = None;
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--cookies" => {
                i += 1;
                cookies_path = args.get(i).map(|s| s.as_str());
                if cookies_path.is_none() {
                    eprintln!("--cookies requires a file path");
                    std::process::exit(1);
                }
            }
            _ => {
                url = Some(args[i].as_str());
            }
        }
        i += 1;
    }

    let url = match url {
        Some(u) => u,
        None => {
            eprintln!("Usage: neorender see [--cookies <path>] <url>");
            std::process::exit(1);
        }
    };

    let mut engine = create_engine();

    // Import cookies from JSON file if provided.
    if let Some(path) = cookies_path {
        import_cookies_file(&mut engine, path);
    }

    match engine.navigate(url) {
        Ok(result) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&result).unwrap_or_else(|e| format!("JSON error: {e}"))
            );
        }
        Err(e) => {
            eprintln!("Navigation failed: {e}");
            std::process::exit(1);
        }
    }
}

/// Import cookies from a JSON file into the engine's cookie store.
///
/// Expected format: array of objects with name, value, domain, path,
/// and optional expires, http_only, secure, same_site fields.
fn import_cookies_file(engine: &mut NeoSession, path: &str) {
    let data = match std::fs::read_to_string(path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Failed to read cookies file {path}: {e}");
            std::process::exit(1);
        }
    };
    let cookies: Vec<neo_types::Cookie> = match serde_json::from_str(&data) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to parse cookies JSON: {e}");
            std::process::exit(1);
        }
    };
    engine.import_cookies(&cookies);
    eprintln!("[NeoRender] Imported {} cookies from {path}", cookies.len());
}

fn print_help() {
    println!("NeoRender V2 — AI Browser Engine");
    println!();
    println!("Usage:");
    println!("  neorender mcp                            Start MCP server (JSON-RPC over stdio)");
    println!("  neorender see <url>                      Navigate to URL and print WOM as JSON");
    println!("  neorender see --cookies <file> <url>     Import cookies from JSON, then navigate");
    println!("  neorender --help                         Show this help");
}
