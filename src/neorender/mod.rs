//! NeoRender — AI's own browser engine.
//!
//! Renders web pages as action maps instead of pixels.
//! Uses V8 (deno_core) for JS execution + Ghost HTTP for networking.
//! No Chrome, no window, no detection.

pub mod ops;
pub mod v8_runtime;
pub mod dom_export;
pub mod session;
pub mod net;
pub mod storage;
pub mod cookie_jar;
pub mod interact;
pub mod rate_limit;
pub mod dom_tree;
pub mod network_log;
pub mod wait;
pub mod extract;
pub mod pool;
pub mod stealth;
pub mod http_cache;
pub mod error_info;

use crate::ghost;

/// Render a page: fetch HTML, execute JS, export as HTML for WOM.
/// `local_storage` injects key-value pairs into JS localStorage before scripts run.
pub async fn render_page(
    ghost_browser: &mut ghost::GhostBrowser,
    url: &str,
    local_storage: Option<&std::collections::HashMap<String, String>>,
) -> Result<RenderResult, String> {
    let start = std::time::Instant::now();

    // 1. Fetch HTML with a fresh Chrome-impersonating client per render
    //    Cookie store handles redirects + Set-Cookie automatically.
    //    Cookies from Ghost's CookieJar are injected via header on the initial request.
    let client = rquest::Client::builder()
        .emulation(rquest_util::Emulation::Chrome136)
        .cookie_store(true)
        .redirect(rquest::redirect::Policy::limited(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Client error: {e}"))?;

    // Inject cookies from our jar via Cookie header (rquest's cookie_store picks them up)
    let mut headers = rquest::header::HeaderMap::new();
    if let Some(domain) = url::Url::parse(url).ok().and_then(|u| u.host_str().map(|s| s.to_string())) {
        if let Some(cookie_header) = ghost_browser.cookies.header_for(&domain) {
            if let Ok(v) = rquest::header::HeaderValue::from_str(&cookie_header) {
                headers.insert(rquest::header::COOKIE, v);
            }
        }
    }

    let resp = client.get(url)
        .headers(headers)
        .send()
        .await
        .map_err(|e| format!("HTTP error: {e}"))?;

    let status = resp.status().as_u16();
    let final_url = resp.url().to_string();

    // Store response cookies back into our jar (for V8 JS injection later)
    if let Some(domain) = resp.url().host_str() {
        for cookie in resp.headers().get_all(rquest::header::SET_COOKIE) {
            if let Ok(s) = cookie.to_str() {
                ghost_browser.cookies.store_from_header(domain, s);
            }
        }
    }

    let html = resp.text().await.map_err(|e| format!("Body error: {e}"))?;
    // Detect WAF/bot challenge pages — don't waste time executing their JS
    let waf_type = detect_waf_challenge(&html);
    if let Some(waf) = &waf_type {
        eprintln!("[NEORENDER] WAF challenge detected: {}", waf);
        return Ok(RenderResult {
            url: final_url,
            status,
            html: html.clone(),
            original_html_len: html.len(),
            render_time_ms: 0,
            scripts_count: 0,
            errors: vec![format!("WAF challenge: {} — use Chrome mode or inject cookies", waf)],
        });
    }

    let html_len = html.len();

    // 2. Extract all scripts (external + inline) preserving order and type
    let mut all_scripts = extract_all_scripts(&html, &final_url);
    let ext_count = all_scripts.iter().filter(|s| s.url.is_some()).count();
    let mod_count = all_scripts.iter().filter(|s| s.is_module).count();
    eprintln!("[NEORENDER] {} scripts ({} external, {} modules) in {}",
        all_scripts.len(), ext_count, mod_count, final_url);

    // 3. Fetch external scripts
    for script in all_scripts.iter_mut() {
        if let Some(url) = &script.url {
            match tokio::time::timeout(
                std::time::Duration::from_secs(10),
                ghost_browser.client_ref().get(url).send(),
            ).await {
                Ok(Ok(resp)) => {
                    if let Ok(text) = resp.text().await {
                        script.content = Some(text);
                    }
                }
                _ => eprintln!("[NEORENDER] Skip slow script: {}", url),
            }
        }
    }

    let fetched_bytes: usize = all_scripts.iter()
        .filter_map(|s| s.content.as_ref().map(|c| c.len()))
        .sum();
    eprintln!("[NEORENDER] {} bytes fetched", fetched_bytes);

    // 4. Create V8 runtime — linkedom loads first, then bootstrap parses HTML
    //    Inject HTML + cookies + localStorage BEFORE bootstrap.js runs (it calls parseHTML)
    let (mut runtime, store) = v8_runtime::create_runtime_with_html(&html, &final_url, &ghost_browser.cookies, local_storage)?;

    // 4b. Pre-populate module store with fetched scripts (so imports resolve)
    {
        let mut s = store.borrow_mut();
        for script in &all_scripts {
            if let (Some(url), Some(content)) = (&script.url, &script.content) {
                s.scripts.insert(url.clone(), content.clone());
            }
        }
    }

    // 4c. Pre-fetch all static imports from module scripts (recursive, depth 3)
    //     This prevents the module loader from needing to do sync HTTP during V8 execution
    {
        let mut to_scan: Vec<(String, String)> = Vec::new(); // (url, content)
        for script in &all_scripts {
            if script.is_module {
                if let (Some(url), Some(content)) = (&script.url, &script.content) {
                    to_scan.push((url.clone(), content.clone()));
                }
            }
        }

        for _depth in 0..3 {
            let mut next_round = Vec::new();
            for (script_url, content) in &to_scan {
                let imports = extract_es_imports(content, script_url);
                for import_url in imports {
                    if store.borrow().scripts.contains_key(&import_url) { continue; }
                    eprintln!("[NEORENDER] Pre-fetching import: {}", import_url.rsplit('/').next().unwrap_or(&import_url));
                    match tokio::time::timeout(
                        std::time::Duration::from_secs(15),
                        ghost_browser.client_ref().get(&import_url).send(),
                    ).await {
                        Ok(Ok(resp)) => {
                            if let Ok(text) = resp.text().await {
                                store.borrow_mut().scripts.insert(import_url.clone(), text.clone());
                                next_round.push((import_url, text));
                            }
                        }
                        _ => eprintln!("[NEORENDER] Skip slow import: {}", import_url),
                    }
                }
            }
            to_scan = next_round;
            if to_scan.is_empty() { break; }
        }
        eprintln!("[NEORENDER] Module store: {} scripts pre-loaded", store.borrow().scripts.len());
    }

    // 5. Execute scripts in document order
    //    - Regular scripts → execute_script
    //    - ES modules → native deno_core module loading (proper import/export)
    let scripts_count = all_scripts.len();
    let mut errors = Vec::new();
    let mut first_module = true;
    for (i, script) in all_scripts.into_iter().enumerate() {
        let Some(content) = script.content else { continue };
        let script_url = script.url.as_deref().unwrap_or(&final_url);
        let name = if script.url.is_some() { format!("script:{i}") } else { format!("inline:{i}") };

        let err = if script.is_module {
            // Use deno_core's native ES module system
            // Content is already in the store; module loader will serve it
            if first_module {
                first_module = false;
                v8_runtime::execute_module(&mut runtime, script_url, name).await
            } else {
                v8_runtime::execute_side_module(&mut runtime, script_url, name).await
            }
        } else {
            v8_runtime::execute_script(&mut runtime, content, name)
        };
        if let Some(e) = err {
            errors.push(e);
        }
    }

    // 6. Fire lifecycle events that SPAs depend on to bootstrap
    let lifecycle_js = r#"
        try { document.dispatchEvent(new Event('DOMContentLoaded', {bubbles:true})); } catch(e){}
        try { dispatchEvent(new Event('DOMContentLoaded', {bubbles:true})); } catch(e){}
        try { dispatchEvent(new Event('load')); } catch(e){}
        try { document.readyState = 'interactive'; } catch(e){}
        try { document.readyState = 'complete'; } catch(e){}
    "#;
    runtime.execute_script("<neorender:lifecycle>", lifecycle_js.to_string())
        .map_err(|e| format!("Lifecycle events error: {e}"))?;

    // 7. Run event loop (fetch promises, timers) with timeout
    v8_runtime::run_event_loop(&mut runtime, 10_000).await?;

    // 8. Export DOM as HTML
    let rendered_html = v8_runtime::export_dom_html(&mut runtime)?;
    let render_time = start.elapsed();

    eprintln!("[NEORENDER] Rendered in {:?} — {} → {} bytes",
        render_time, html_len, rendered_html.len());

    if !errors.is_empty() {
        eprintln!("[NEORENDER] {} script errors: {:?}", errors.len(), errors);
    }

    Ok(RenderResult {
        url: final_url,
        status,
        html: rendered_html,
        original_html_len: html_len,
        render_time_ms: render_time.as_millis() as u64,
        scripts_count,
        errors,
    })
}

pub struct RenderResult {
    pub url: String,
    pub status: u16,
    pub html: String,           // rendered HTML (after JS execution)
    pub original_html_len: usize,
    pub errors: Vec<String>,    // script execution errors
    pub render_time_ms: u64,
    pub scripts_count: usize,
}

// ─── Helpers ───

#[derive(Debug)]
pub(crate) struct ScriptInfo {
    pub(crate) url: Option<String>,     // None for inline
    pub(crate) content: Option<String>, // None for external (fetched later)
    pub(crate) is_module: bool,
}

pub(crate) fn extract_all_scripts(html: &str, base_url: &str) -> Vec<ScriptInfo> {
    use html5ever::parse_document;
    use html5ever::tendril::TendrilSink;
    use markup5ever_rcdom::{RcDom, Handle, NodeData};

    let dom = parse_document(RcDom::default(), Default::default())
        .from_utf8()
        .read_from(&mut html.as_bytes())
        .unwrap();

    let mut scripts = Vec::new();
    fn collect(node: &Handle, base: &str, scripts: &mut Vec<ScriptInfo>) {
        if let NodeData::Element { name, attrs, .. } = &node.data {
            if name.local.as_ref() == "script" {
                let attrs_ref = attrs.borrow();
                let script_type = attrs_ref.iter()
                    .find(|a| a.name.local.as_ref() == "type")
                    .map(|a| a.value.to_string())
                    .unwrap_or_default();
                // Skip non-JS script types (JSON data, LD+JSON, importmaps, etc.)
                let st = script_type.to_lowercase();
                if st.contains("json") || st.contains("importmap") || st.contains("template")
                    || st.contains("html") || st.contains("x-") {
                    for child in node.children.borrow().iter() { collect(child, base, scripts); }
                    return;
                }
                let is_module = script_type == "module";
                let src = attrs_ref.iter()
                    .find(|a| a.name.local.as_ref() == "src")
                    .map(|a| a.value.to_string());

                if let Some(src) = src {
                    let full = if src.starts_with("http") { src }
                    else if src.starts_with("//") { format!("https:{src}") }
                    else if let Ok(base_url) = url::Url::parse(base) {
                        base_url.join(&src).map(|u| u.to_string()).unwrap_or(src)
                    } else { src };
                    scripts.push(ScriptInfo { url: Some(full), content: None, is_module });
                } else {
                    drop(attrs_ref);
                    let text: String = node.children.borrow().iter()
                        .filter_map(|c| match &c.data {
                            NodeData::Text { contents } => Some(contents.borrow().to_string()),
                            _ => None,
                        })
                        .collect();
                    if !text.trim().is_empty() {
                        scripts.push(ScriptInfo { url: None, content: Some(text), is_module });
                    }
                }
            }
        }
        for child in node.children.borrow().iter() { collect(child, base, scripts); }
    }
    collect(&dom.document, base_url, &mut scripts);
    scripts
}

async fn fetch_scripts(ghost: &ghost::GhostBrowser, urls: &[String]) -> Vec<String> {
    let mut scripts = Vec::new();
    // Fetch scripts + follow ES imports
    let mut to_fetch: Vec<String> = urls.to_vec();
    let mut fetched: Vec<String> = Vec::new();

    for _depth in 0..2 {
        let mut next_round = Vec::new();
        for url in &to_fetch {
            if fetched.contains(url) { continue; }
            fetched.push(url.clone());

            match tokio::time::timeout(
                std::time::Duration::from_secs(10),
                ghost.client_ref().get(url).send(),
            ).await {
                Ok(Ok(resp)) => {
                    if let Ok(text) = resp.text().await {
                        // Check for ES module imports
                        let imports = extract_es_imports(&text, url);
                        next_round.extend(imports);
                        scripts.push(text);
                    }
                }
                _ => eprintln!("[NEORENDER] Skip slow script: {url}"),
            }
        }
        to_fetch = next_round;
        if to_fetch.is_empty() { break; }
    }
    scripts
}

pub(crate) fn extract_es_imports(js: &str, script_url: &str) -> Vec<String> {
    let mut imports = Vec::new();
    let base = if let Some(pos) = script_url.rfind('/') { &script_url[..=pos] } else { script_url };

    // import"./path.js" or import "./path.js"
    let patterns = &[
        r#"import\s*"(\./[^"]+)""#,
        r#"import\s*'(\./[^']+)'"#,
    ];
    for pattern in patterns {
        if let Ok(re) = regex_lite::Regex::new(pattern) {
            for cap in re.captures_iter(js) {
                if let Some(path) = cap.get(1) {
                    let relative = path.as_str();
                    let full = if relative.starts_with("./") {
                        format!("{}{}", base, &relative[2..])
                    } else {
                        format!("{}{}", base, relative)
                    };
                    if !imports.contains(&full) { imports.push(full); }
                }
            }
        }
    }
    imports
}

pub(crate) fn detect_waf_challenge(html: &str) -> Option<String> {
    let h = &html[..html.len().min(5000)]; // Only check first 5KB
    if h.contains("_cf_chl_opt") || h.contains("cf-browser-verification") || h.contains("cf_chl_managed") {
        return Some("Cloudflare".to_string());
    }
    if h.contains("AwsWafIntegration") || h.contains("aws-waf-token") || h.contains("challenge-container")
        || h.contains("InterstitialChallenge") || h.contains("bm-verify") {
        return Some("AWS WAF".to_string());
    }
    if h.contains("akamai") && h.contains("challenge") {
        return Some("Akamai".to_string());
    }
    if h.contains("_dd_s") && h.contains("challenge") {
        return Some("DataDome".to_string());
    }
    if h.contains("px-captcha") || h.contains("_pxhd") {
        return Some("PerimeterX".to_string());
    }
    // Generic: tiny body with only noscript/script tags = challenge or empty SPA shell
    if html.len() < 3000 {
        let body_start = html.find("<body").unwrap_or(0);
        let body = &html[body_start..];
        // Check if body has almost no visible content
        let visible: String = regex_lite::Regex::new(r"<[^>]+>").ok()
            .map(|re| re.replace_all(body, " ").to_string())
            .unwrap_or_default();
        let visible = visible.trim();
        if visible.len() < 50 && html.contains("location.reload") {
            return Some("Generic bot challenge".to_string());
        }
    }
    None
}
