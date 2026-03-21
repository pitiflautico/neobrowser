//! NeoRender V2 — AI Browser Engine
//! Binary entry point: CLI + MCP server mode.

use std::sync::{Arc, Mutex};

use neo_dom::Html5everDom;
use neo_engine::config::EngineConfig;
use neo_engine::{BrowserEngine, NeoSession};
use neo_extract::DefaultExtractor;
use neo_http::RquestClient;
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
    let url = match args.get(2) {
        Some(u) => u.as_str(),
        None => {
            eprintln!("Usage: neorender see <url>");
            std::process::exit(1);
        }
    };

    let mut engine = create_engine();
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

fn print_help() {
    println!("NeoRender V2 — AI Browser Engine");
    println!();
    println!("Usage:");
    println!("  neorender mcp           Start MCP server (JSON-RPC over stdio)");
    println!("  neorender see <url>     Navigate to URL and print WOM as JSON");
    println!("  neorender --help        Show this help");
}
