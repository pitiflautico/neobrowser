//! NeoRender V2 — AI Browser Engine
//! Binary entry point: CLI + MCP server mode.

use std::sync::{Arc, Mutex};

use neo_dom::Html5everDom;
use neo_engine::config::EngineConfig;
use neo_engine::{BrowserEngine, NeoSession};
use neo_extract::DefaultExtractor;
use neo_http::{CookieStore, DiskCache, RquestClient, SqliteCookieStore};
use neo_interact::DomInteractor;
use neo_trace::file_tracer::FileTracer;
use neo_trace::noop::NoopTracer;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    match args.get(1).map(|s| s.as_str()) {
        Some("mcp") => run_mcp(),
        Some("see") => run_see(&args),
        Some("search") => run_search(&args),
        Some("import-cookies") => run_import_cookies(&args),
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

    let config = EngineConfig::default(); // execute_js = true by default

    // Create V8 runtime with shared HttpClient for op_fetch.
    let http_for_v8: std::sync::Arc<dyn neo_http::HttpClient> =
        std::sync::Arc::new(RquestClient::default());
    let rt_config = neo_runtime::RuntimeConfig::default();
    let runtime: Option<Box<dyn neo_runtime::JsRuntime>> =
        match neo_runtime::v8::DenoRuntime::new_with_http(&rt_config, http_for_v8) {
            Ok(rt) => Some(Box::new(rt)),
            Err(e) => {
                eprintln!("[NeoRender V2] V8 runtime init failed: {e} -- falling back to no-JS");
                None
            }
        };

    let cookie_store = SqliteCookieStore::default_store()
        .expect("failed to open cookie store at ~/.neorender/cookies.db");
    let disk_cache =
        DiskCache::default_cache().expect("failed to open disk cache at ~/.neorender/cache/http/");

    NeoSession::new_shared(
        Box::new(http),
        shared_dom,
        runtime,
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
                serde_json::to_string_pretty(&result)
                    .unwrap_or_else(|e| format!("JSON error: {e}"))
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

fn run_search(args: &[String]) {
    // Parse: neorender search <query> [--num N] [--deep] [--deep-num N]
    let mut query_parts: Vec<&str> = Vec::new();
    let mut num: u64 = 10;
    let mut deep = false;
    let mut deep_num: u64 = 3;
    let mut i = 2;

    while i < args.len() {
        match args[i].as_str() {
            "--num" => {
                i += 1;
                if let Some(v) = args.get(i).and_then(|s| s.parse().ok()) {
                    num = v;
                }
            }
            "--deep" => deep = true,
            "--deep-num" => {
                i += 1;
                if let Some(v) = args.get(i).and_then(|s| s.parse().ok()) {
                    deep_num = v;
                }
            }
            _ => query_parts.push(args[i].as_str()),
        }
        i += 1;
    }

    if query_parts.is_empty() {
        eprintln!("Usage: neorender search <query> [--num 10] [--deep] [--deep-num 3]");
        std::process::exit(1);
    }

    let query = query_parts.join(" ");
    let search_args = serde_json::json!({
        "query": query,
        "num": num,
        "deep": deep,
        "deep_num": deep_num,
    });

    // search tool doesn't use the engine, but call_tool requires McpState
    let engine = create_engine();
    let mut state = neo_mcp::state::McpState::new(Box::new(engine));

    match neo_mcp::tools::call_tool("search", search_args, &mut state) {
        Ok(result) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&result)
                    .unwrap_or_else(|e| format!("JSON error: {e}"))
            );
        }
        Err(e) => {
            eprintln!("Search failed: {e}");
            std::process::exit(1);
        }
    }
}

fn run_import_cookies(args: &[String]) {
    // Parse: neorender import-cookies --chrome-profile <name> [--domain <domain>]
    let mut profile: Option<&str> = None;
    let mut domain: Option<&str> = None;
    let mut i = 2;

    while i < args.len() {
        match args[i].as_str() {
            "--chrome-profile" => {
                i += 1;
                profile = args.get(i).map(|s| s.as_str());
            }
            "--domain" => {
                i += 1;
                domain = args.get(i).map(|s| s.as_str());
            }
            _ => {
                eprintln!("Unknown flag: {}", args[i]);
                eprintln!(
                    "Usage: neorender import-cookies --chrome-profile <name> [--domain <domain>]"
                );
                std::process::exit(1);
            }
        }
        i += 1;
    }

    let profile = match profile {
        Some(p) => p,
        None => {
            eprintln!(
                "Usage: neorender import-cookies --chrome-profile <name> [--domain <domain>]"
            );
            std::process::exit(1);
        }
    };

    eprintln!(
        "[NeoRender] Importing cookies from Chrome profile \"{}\"{}",
        profile,
        domain
            .map(|d| format!(" (domain: {d})"))
            .unwrap_or_default()
    );

    let importer = neo_http::ChromeCookieImporter::new(profile, domain);
    let cookies = match importer.import() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[NeoRender] Chrome import failed: {e}");
            std::process::exit(1);
        }
    };

    if cookies.is_empty() {
        eprintln!("[NeoRender] No cookies found.");
        return;
    }

    // Print summary per domain (name only, never values).
    let mut by_domain: std::collections::HashMap<&str, Vec<&str>> =
        std::collections::HashMap::new();
    for c in &cookies {
        by_domain.entry(&c.domain).or_default().push(&c.name);
    }
    for (dom, names) in &by_domain {
        eprintln!("  {dom}: {} cookies", names.len());
    }

    // Convert to neo_types::Cookie (already the right type) and import.
    let store = SqliteCookieStore::default_store()
        .expect("failed to open cookie store at ~/.neorender/cookies.db");
    store.import(&cookies);

    eprintln!(
        "[NeoRender] Imported {} cookies into ~/.neorender/cookies.db",
        cookies.len()
    );
}

fn print_help() {
    println!("NeoRender V2 — AI Browser Engine");
    println!();
    println!("Usage:");
    println!("  neorender mcp                            Start MCP server (JSON-RPC over stdio)");
    println!("  neorender see <url>                      Navigate to URL and print WOM as JSON");
    println!("  neorender see --cookies <file> <url>     Import cookies from JSON, then navigate");
    println!("  neorender search <query> [--num N] [--deep] [--deep-num N]");
    println!("                                           Search the web via DuckDuckGo");
    println!(
        "  neorender import-cookies --chrome-profile <name> [--domain <domain>]"
    );
    println!("                                           Import cookies from Chrome profile");
    println!("  neorender --help                         Show this help");
}
