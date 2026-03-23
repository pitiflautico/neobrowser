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

/// Wrapper that delegates `CookieStore` to an `Arc<dyn CookieStore>`.
///
/// Needed because `NeoSession::with_cookie_store` takes `Box<dyn CookieStore>`
/// but we share the same store with the V8 runtime via `Arc`.
struct ArcCookieStoreWrapper(Arc<dyn CookieStore>);

impl CookieStore for ArcCookieStoreWrapper {
    fn get_for_request(&self, url: &str, top_level_url: Option<&str>, is_top_level: bool) -> String {
        self.0.get_for_request(url, top_level_url, is_top_level)
    }
    fn store_set_cookie(&self, url: &str, set_cookie: &str) {
        self.0.store_set_cookie(url, set_cookie)
    }
    fn delete(&self, name: &str, domain: &str, path: &str) {
        self.0.delete(name, domain, path)
    }
    fn evict_expired(&self) {
        self.0.evict_expired()
    }
    fn clear_session(&self) {
        self.0.clear_session()
    }
    fn list_for_domain(&self, domain: &str) -> Vec<neo_types::Cookie> {
        self.0.list_for_domain(domain)
    }
    fn export(&self) -> Vec<neo_types::Cookie> {
        self.0.export()
    }
    fn import(&self, cookies: &[neo_types::Cookie]) {
        self.0.import(cookies)
    }
    fn snapshot(&self) -> Vec<neo_types::Cookie> {
        self.0.snapshot()
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    match args.get(1).map(|s| s.as_str()) {
        Some("mcp") => run_mcp(),
        Some("see") => run_see(&args),
        Some("interact") => run_interact(&args),
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

    // Create cookie store FIRST so it can be shared with the V8 runtime.
    let cookie_store = SqliteCookieStore::default_store()
        .expect("failed to open cookie store at ~/.neorender/cookies.db");
    let cookie_store_arc: std::sync::Arc<dyn CookieStore> = std::sync::Arc::new(cookie_store);

    // Create V8 runtime with shared HttpClient, cookie store, and raw client
    // for both op_fetch (non-streaming) and op_fetch_start (streaming G2).
    let wreq_for_v8 = RquestClient::default();
    let raw_client = wreq_for_v8.raw_client();
    let http_for_v8: std::sync::Arc<dyn neo_http::HttpClient> =
        std::sync::Arc::new(wreq_for_v8);
    let rt_config = neo_runtime::RuntimeConfig::default();
    let runtime: Option<Box<dyn neo_runtime::JsRuntime>> =
        match neo_runtime::v8::DenoRuntime::new_with_streaming(
            &rt_config,
            http_for_v8,
            cookie_store_arc.clone(),
            raw_client,
        ) {
            Ok(rt) => Some(Box::new(rt)),
            Err(e) => {
                eprintln!("[NeoRender V2] V8 runtime init failed: {e} -- falling back to no-JS");
                None
            }
        };

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
    .with_cookie_store(Box::new(ArcCookieStoreWrapper(cookie_store_arc)))
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
    // Support both [{...}] and {"cookies": [{...}]} formats
    let cookies: Vec<neo_types::Cookie> = match serde_json::from_str::<Vec<neo_types::Cookie>>(&data) {
        Ok(c) => c,
        Err(_) => {
            // Try wrapped format: {"cookies": [...]}
            match serde_json::from_str::<serde_json::Value>(&data) {
                Ok(serde_json::Value::Object(map)) => {
                    if let Some(arr) = map.get("cookies") {
                        serde_json::from_value(arr.clone()).unwrap_or_else(|e| {
                            eprintln!("Failed to parse cookies array: {e}");
                            std::process::exit(1);
                        })
                    } else {
                        eprintln!("JSON object has no 'cookies' key");
                        std::process::exit(1);
                    }
                }
                _ => {
                    eprintln!("Failed to parse cookies JSON: expected array or object with 'cookies' key");
                    std::process::exit(1);
                }
            }
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

fn run_interact(args: &[String]) {
    let url = match args.get(2) {
        Some(u) => u.as_str(),
        None => {
            eprintln!("Usage: neorender interact <url>");
            eprintln!("  Navigate to URL and enter an interactive REPL.");
            std::process::exit(1);
        }
    };

    let mut engine = create_engine();

    eprintln!("[NeoRender] Navigating to {url}...");
    match engine.navigate(url) {
        Ok(result) => {
            eprintln!(
                "[NeoRender] Loaded: {} ({}, {}ms)",
                result.title, result.url, result.render_ms
            );
            eprintln!(
                "[NeoRender] WOM: {} nodes, type={}",
                result.wom.nodes.len(),
                result.wom.page_type
            );
        }
        Err(e) => {
            eprintln!("Navigation failed: {e}");
            std::process::exit(1);
        }
    }

    eprintln!();
    eprintln!("Interactive REPL — commands:");
    eprintln!("  click <selector>           Click an element");
    eprintln!("  type <selector> <text>     Type text into an input");
    eprintln!("  press <selector> <key>     Press a key (Enter, Tab, Escape)");
    eprintln!("  submit <selector>          Submit a form");
    eprintln!("  find <query>               Smart element search (CSS, text, ARIA, placeholder, name)");
    eprintln!("  fill <name>=<val> [...]    Smart form fill (by name/label/placeholder/aria-label)");
    eprintln!("  extract text|links|semantic|wom  Extract page content");
    eprintln!("  wait <selector> [ms]       Wait for element (default 5000ms)");
    eprintln!("  eval <js>                  Execute JavaScript");
    eprintln!("  trace                      Show interaction trace (event→fetch→state pipeline)");
    eprintln!("  trace clear                Clear interaction trace");
    eprintln!("  diag                       Run full diagnostics (DOM, frameworks, modules)");
    eprintln!("  url                        Show current URL");
    eprintln!("  nav <url>                  Navigate to new URL");
    eprintln!("  quit                       Exit");
    eprintln!();

    let stdin = std::io::stdin();
    let mut line_buf = String::new();

    loop {
        eprint!("neo> ");
        line_buf.clear();
        match stdin.read_line(&mut line_buf) {
            Ok(0) => break, // EOF
            Ok(_) => {}
            Err(e) => {
                eprintln!("Read error: {e}");
                break;
            }
        }

        let trimmed = line_buf.trim();
        if trimmed.is_empty() {
            continue;
        }

        let parts: Vec<&str> = trimmed.splitn(3, ' ').collect();
        let cmd = parts[0];

        match cmd {
            "quit" | "exit" | "q" => break,

            "click" => {
                let selector = match parts.get(1) {
                    Some(s) => *s,
                    None => {
                        eprintln!("Usage: click <selector>");
                        continue;
                    }
                };
                match engine.click(selector) {
                    Ok(result) => println!("{result:?}"),
                    Err(e) => eprintln!("Error: {e}"),
                }
            }

            "type" => {
                if parts.len() < 3 {
                    eprintln!("Usage: type <selector> <text>");
                    continue;
                }
                let selector = parts[1];
                let text = parts[2];
                match engine.type_text(selector, text) {
                    Ok(()) => println!("OK"),
                    Err(e) => eprintln!("Error: {e}"),
                }
            }

            "press" => {
                if parts.len() < 3 {
                    eprintln!("Usage: press <selector> <key>");
                    continue;
                }
                let selector = parts[1];
                let key = parts[2];
                match engine.press_key(selector, key) {
                    Ok(()) => println!("OK"),
                    Err(e) => eprintln!("Error: {e}"),
                }
            }

            "submit" => {
                let selector = parts.get(1).copied();
                match engine.submit(selector) {
                    Ok(result) => println!("{result:?}"),
                    Err(e) => eprintln!("Error: {e}"),
                }
            }

            "extract" => {
                let kind = match parts.get(1) {
                    Some(k) => *k,
                    None => {
                        eprintln!("Usage: extract text|links|semantic|wom");
                        continue;
                    }
                };
                match kind {
                    "text" => match engine.extract_text() {
                        Ok(text) => println!("{text}"),
                        Err(e) => eprintln!("Error: {e}"),
                    },
                    "links" => match engine.extract_links() {
                        Ok(links) => {
                            for (text, href) in &links {
                                println!("  [{text}] -> {href}");
                            }
                            println!("({} links)", links.len());
                        }
                        Err(e) => eprintln!("Error: {e}"),
                    },
                    "semantic" => match engine.extract_semantic() {
                        Ok(text) => println!("{text}"),
                        Err(e) => eprintln!("Error: {e}"),
                    },
                    "wom" => match engine.extract() {
                        Ok(wom) => {
                            println!(
                                "{}",
                                serde_json::to_string_pretty(&wom)
                                    .unwrap_or_else(|e| format!("JSON error: {e}"))
                            );
                        }
                        Err(e) => eprintln!("Error: {e}"),
                    },
                    other => eprintln!("Unknown extract kind: {other}"),
                }
            }

            "wait" => {
                let selector = match parts.get(1) {
                    Some(s) => *s,
                    None => {
                        eprintln!("Usage: wait <selector> [timeout_ms]");
                        continue;
                    }
                };
                let timeout: u32 = parts
                    .get(2)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(5000);
                match engine.wait_for(selector, timeout) {
                    Ok(found) => println!("{}", if found { "found" } else { "not found" }),
                    Err(e) => eprintln!("Error: {e}"),
                }
            }

            "eval" => {
                let js = match parts.get(1..) {
                    Some(rest) if !rest.is_empty() => rest.join(" "),
                    _ => {
                        eprintln!("Usage: eval <javascript>");
                        continue;
                    }
                };
                match engine.eval(&js) {
                    Ok(result) => println!("{result}"),
                    Err(e) => eprintln!("Error: {e}"),
                }
            }

            "url" => match engine.current_url() {
                Ok(url) => println!("{url}"),
                Err(e) => eprintln!("Error: {e}"),
            },

            "nav" => {
                let target = match parts.get(1) {
                    Some(u) => *u,
                    None => {
                        eprintln!("Usage: nav <url>");
                        continue;
                    }
                };
                match engine.navigate(target) {
                    Ok(result) => {
                        println!(
                            "Loaded: {} ({}, {}ms, {} nodes)",
                            result.title,
                            result.url,
                            result.render_ms,
                            result.wom.nodes.len()
                        );
                    }
                    Err(e) => eprintln!("Error: {e}"),
                }
            }

            "find" => {
                let query = match parts.get(1..) {
                    Some(rest) if !rest.is_empty() => rest.join(" "),
                    _ => {
                        eprintln!("Usage: find <query>");
                        continue;
                    }
                };
                match engine.find_element(&query) {
                    Ok(elements) => {
                        if elements.is_empty() {
                            println!("No elements found.");
                        } else {
                            for (i, el) in elements.iter().enumerate() {
                                println!(
                                    "  [{}] <{}> role={} type={} label={:?} value={:?} {}",
                                    i,
                                    el.tag,
                                    el.role,
                                    el.element_type,
                                    el.label,
                                    el.value,
                                    if el.interactable { "interactable" } else { "NOT interactable" }
                                );
                                println!("      selector: {}", el.selector);
                            }
                            println!("({} elements found)", elements.len());
                        }
                    }
                    Err(e) => eprintln!("Error: {e}"),
                }
            }

            "fill" => {
                let rest = match parts.get(1..) {
                    Some(r) if !r.is_empty() => r.join(" "),
                    _ => {
                        eprintln!("Usage: fill <name>=<value> [name2=value2 ...]");
                        continue;
                    }
                };
                let mut fields = std::collections::HashMap::new();
                for pair in rest.split_whitespace() {
                    if let Some(eq_pos) = pair.find('=') {
                        let key = &pair[..eq_pos];
                        let val = &pair[eq_pos + 1..];
                        fields.insert(key.to_string(), val.to_string());
                    } else {
                        eprintln!("Invalid field format: {pair} (expected name=value)");
                    }
                }
                if fields.is_empty() {
                    eprintln!("No valid fields to fill.");
                    continue;
                }
                match engine.fill_form(&fields) {
                    Ok(()) => println!("OK ({} fields filled)", fields.len()),
                    Err(e) => eprintln!("Error: {e}"),
                }
            }

            "trace" => {
                if parts.get(1) == Some(&"clear") {
                    match engine.eval("__neo_clearTrace()") {
                        Ok(_) => println!("Trace cleared."),
                        Err(e) => eprintln!("Error: {e}"),
                    }
                } else {
                    match engine.eval("__neo_getTrace()") {
                        Ok(raw) => {
                            match serde_json::from_str::<serde_json::Value>(&raw) {
                                Ok(val) => {
                                    println!(
                                        "{}",
                                        serde_json::to_string_pretty(&val)
                                            .unwrap_or_else(|_| raw.clone())
                                    );
                                }
                                Err(_) => println!("{raw}"),
                            }
                        }
                        Err(e) => eprintln!("Error: {e}"),
                    }
                }
            }

            "diag" => {
                let diag_js = r#"(function() {
    var d = {};
    d.dom_nodes = document.querySelectorAll('*').length;
    d.react = typeof React;
    d.reactdom = typeof ReactDOM;
    d.next = typeof __NEXT_DATA__;
    d.vite = typeof __vite__mapDeps;
    d.vue = typeof __VUE__;
    d.angular = typeof ng;
    d.svelte = typeof __svelte;
    d.jquery = typeof jQuery;
    var els = document.querySelectorAll('*');
    var fibers = 0;
    for (var i = 0; i < els.length; i++) {
        var keys = Object.keys(els[i]);
        for (var j = 0; j < keys.length; j++) {
            if (keys[j].startsWith('__react')) { fibers++; break; }
        }
    }
    d.react_fibers = fibers;
    var interesting = [];
    for (var k in window) {
        if (k.startsWith('__')) interesting.push(k);
    }
    d.dunder_globals = interesting.slice(0, 30);
    d.dunder_count = interesting.length;
    d.scripts_total = document.querySelectorAll('script').length;
    d.scripts_src = document.querySelectorAll('script[src]').length;
    d.scripts_inline = d.scripts_total - d.scripts_src;
    d.scripts_module = document.querySelectorAll('script[type="module"]').length;
    d.title = document.title;
    d.url = location.href;
    d.ready_state = document.readyState;
    return JSON.stringify(d);
})()"#;
                match engine.eval(diag_js) {
                    Ok(raw) => {
                        // Try to pretty-print JSON, fall back to raw
                        match serde_json::from_str::<serde_json::Value>(&raw) {
                            Ok(val) => {
                                println!(
                                    "{}",
                                    serde_json::to_string_pretty(&val)
                                        .unwrap_or_else(|_| raw.clone())
                                );
                            }
                            Err(_) => println!("{raw}"),
                        }
                    }
                    Err(e) => eprintln!("Error: {e}"),
                }
            }

            other => eprintln!("Unknown command: {other}. Type 'quit' to exit."),
        }
    }
}

fn print_help() {
    println!("NeoRender V2 -- AI Browser Engine");
    println!();
    println!("Usage:");
    println!("  neorender mcp                            Start MCP server (JSON-RPC over stdio)");
    println!("  neorender see <url>                      Navigate to URL and print WOM as JSON");
    println!("  neorender see --cookies <file> <url>     Import cookies from JSON, then navigate");
    println!("  neorender interact <url>                 Navigate and enter interactive REPL");
    println!("  neorender search <query> [--num N] [--deep] [--deep-num N]");
    println!("                                           Search the web via DuckDuckGo");
    println!(
        "  neorender import-cookies --chrome-profile <name> [--domain <domain>]"
    );
    println!("                                           Import cookies from Chrome profile");
    println!("  neorender --help                         Show this help");
}
