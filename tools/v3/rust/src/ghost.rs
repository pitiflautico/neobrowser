//! Ghost Browser — AI's own browser, no Chrome, no window, no detection.
//!
//! Pure Rust HTTP client that speaks like Chrome at the network level.
//! Parses HTML → DOM → WOM for AI navigation. No rendering, no GPU.
//! Invisible to Cloudflare because there's nothing to detect.

use html5ever::parse_document;
use html5ever::tendril::TendrilSink;
use markup5ever_rcdom::RcDom;
use rquest::header::{HeaderMap, HeaderValue, ACCEPT, ACCEPT_ENCODING, ACCEPT_LANGUAGE,
    CACHE_CONTROL, COOKIE, REFERER, SET_COOKIE, UPGRADE_INSECURE_REQUESTS, USER_AGENT};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─── Cookie Jar ───

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GhostCookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub expires: Option<i64>,
    pub http_only: bool,
    pub secure: bool,
}

#[derive(Debug, Default)]
pub struct CookieJar {
    cookies: HashMap<String, Vec<GhostCookie>>, // domain → cookies
}

impl CookieJar {
    pub fn new() -> Self { Self::default() }

    /// Parse Set-Cookie header and store
    pub fn store_from_header(&mut self, domain: &str, header: &str) {
        let parts: Vec<&str> = header.split(';').collect();
        if parts.is_empty() { return; }
        let kv: Vec<&str> = parts[0].splitn(2, '=').collect();
        if kv.len() != 2 { return; }

        let mut cookie = GhostCookie {
            name: kv[0].trim().to_string(),
            value: kv[1].trim().to_string(),
            domain: domain.to_string(),
            path: "/".to_string(),
            expires: None,
            http_only: false,
            secure: false,
        };

        for part in &parts[1..] {
            let p = part.trim().to_lowercase();
            if p.starts_with("domain=") {
                cookie.domain = p[7..].trim_start_matches('.').to_string();
            } else if p.starts_with("path=") {
                cookie.path = p[5..].to_string();
            } else if p == "httponly" {
                cookie.http_only = true;
            } else if p == "secure" {
                cookie.secure = true;
            }
        }

        let cookies = self.cookies.entry(cookie.domain.clone()).or_default();
        cookies.retain(|c| c.name != cookie.name);
        cookies.push(cookie);
    }

    /// Get all cookies as domain → "name=val; name2=val2" map
    pub fn all_headers(&self) -> HashMap<String, String> {
        self.cookies.iter().map(|(domain, cks)| {
            let header = cks.iter()
                .map(|c| format!("{}={}", c.name, c.value))
                .collect::<Vec<_>>()
                .join("; ");
            (domain.clone(), header)
        }).collect()
    }

    /// Get Cookie header value for a domain
    pub fn header_for(&self, domain: &str) -> Option<String> {
        let mut all = Vec::new();
        for (d, cookies) in &self.cookies {
            if domain.ends_with(d.as_str()) || d == domain {
                for c in cookies {
                    all.push(format!("{}={}", c.name, c.value));
                }
            }
        }
        if all.is_empty() { None } else { Some(all.join("; ")) }
    }

    /// Load cookies from a JSON file.
    /// Accepts: array [{name,value,domain,...}] or object {cookies:[...]} (browser_state export)
    pub fn load_file(&mut self, path: &str) -> Result<usize, String> {
        let data = std::fs::read_to_string(path).map_err(|e| format!("{e}"))?;
        let parsed: serde_json::Value = serde_json::from_str(&data).map_err(|e| format!("{e}"))?;
        // Accept both [{...}] array and {cookies: [{...}]} object
        let cookies: Vec<serde_json::Value> = if let Some(arr) = parsed.as_array() {
            arr.clone()
        } else if let Some(arr) = parsed["cookies"].as_array() {
            arr.clone()
        } else {
            return Err("Expected JSON array or object with 'cookies' key".to_string());
        };
        let mut count = 0;
        for c in cookies {
            let name = c["name"].as_str().unwrap_or_default();
            let value = c["value"].as_str().unwrap_or_default();
            let domain = c["domain"].as_str().unwrap_or_default().trim_start_matches('.');
            if !name.is_empty() && !domain.is_empty() {
                let cookies = self.cookies.entry(domain.to_string()).or_default();
                cookies.retain(|x| x.name != name);
                cookies.push(GhostCookie {
                    name: name.to_string(),
                    value: value.to_string(),
                    domain: domain.to_string(),
                    path: c["path"].as_str().unwrap_or("/").to_string(),
                    expires: c["expirationDate"].as_f64().or_else(|| c["expires"].as_f64()).map(|f| f as i64),
                    http_only: c["httpOnly"].as_bool().unwrap_or(false),
                    secure: c["secure"].as_bool().unwrap_or(false),
                });
                count += 1;
            }
        }
        Ok(count)
    }

    pub fn count(&self) -> usize {
        self.cookies.values().map(|v| v.len()).sum()
    }
}

// ─── Ghost Browser ───

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageResult {
    pub url: String,
    pub status: u16,
    pub title: String,
    pub text: String,        // visible text
    pub links: Vec<Link>,
    pub forms: Vec<Form>,
    pub inputs: Vec<Input>,
    pub buttons: Vec<Button>,
    pub apis: Vec<ApiEndpoint>,  // discovered API endpoints
    pub html_len: usize,
    pub is_spa: bool,
}

/// An API endpoint discovered by scanning JS bundles
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiEndpoint {
    pub url: String,           // full or partial URL
    pub method: String,        // GET, POST, PUT, DELETE, etc.
    pub context: String,       // surrounding code for understanding
    pub source: String,        // which JS file it came from
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Link {
    pub text: String,
    pub href: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Form {
    pub action: String,
    pub method: String,
    pub fields: Vec<FormField>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormField {
    pub name: String,
    pub field_type: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Input {
    pub name: String,
    pub input_type: String,
    pub placeholder: String,
    pub value: String,
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Button {
    pub text: String,
    pub button_type: String,
    pub name: String,
}

pub struct GhostBrowser {
    client: rquest::Client,
    pub cookies: CookieJar,
    pub last_url: String,
    pub last_html: String,
    pub history: Vec<String>,
    user_agent: String,
}

impl GhostBrowser {
    pub fn new() -> Self {
        let ua = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36";

        let client = crate::http_client::chrome136_with_cookies()
            .expect("Failed to build HTTP client");

        Self {
            client,
            cookies: CookieJar::new(),
            last_url: String::new(),
            last_html: String::new(),
            history: Vec::new(),
            user_agent: ua.to_string(),
        }
    }

    /// Get a reference to the HTTP client (for neorender)
    pub fn client_ref(&self) -> &rquest::Client { &self.client }

    /// Chrome-like headers — match a real Chrome 136 browser on macOS.
    pub fn chrome_headers(&self, url: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(USER_AGENT, HeaderValue::from_str(&self.user_agent).unwrap());
        h.insert(ACCEPT, HeaderValue::from_static("text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7"));
        h.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("es-ES,es;q=0.9,en;q=0.8"));
        h.insert(ACCEPT_ENCODING, HeaderValue::from_static("gzip, deflate, br, zstd"));
        h.insert(CACHE_CONTROL, HeaderValue::from_static("max-age=0"));
        h.insert(UPGRADE_INSECURE_REQUESTS, HeaderValue::from_static("1"));
        h.insert("Sec-Ch-Ua", HeaderValue::from_static("\"Chromium\";v=\"136\", \"Not_A Brand\";v=\"24\", \"Google Chrome\";v=\"136\""));
        h.insert("Sec-Ch-Ua-Mobile", HeaderValue::from_static("?0"));
        h.insert("Sec-Ch-Ua-Platform", HeaderValue::from_static("\"macOS\""));
        h.insert("Sec-Fetch-Dest", HeaderValue::from_static("document"));
        h.insert("Sec-Fetch-Mode", HeaderValue::from_static("navigate"));
        h.insert("Sec-Fetch-Site", HeaderValue::from_static("none"));
        h.insert("Sec-Fetch-User", HeaderValue::from_static("?1"));

        // Referer from last URL if same domain
        if !self.last_url.is_empty() {
            if let Ok(r) = HeaderValue::from_str(&self.last_url) {
                h.insert(REFERER, r);
            }
        }

        // Cookies
        if let Some(domain) = url::Url::parse(url).ok().and_then(|u| u.host_str().map(|s| s.to_string())) {
            if let Some(cookie_header) = self.cookies.header_for(&domain) {
                if let Ok(v) = HeaderValue::from_str(&cookie_header) {
                    eprintln!("[GHOST] Cookie header: {} chars for {}", v.len(), domain);
                    h.insert(COOKIE, v);
                }
            }
        }
        h
    }

    /// Navigate to a URL, return parsed page with API discovery
    pub async fn goto(&mut self, url: &str) -> Result<PageResult, String> {
        let headers = self.chrome_headers(url);
        let resp = match self.client.get(url)
            .headers(headers.clone())
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) if e.to_string().contains("Connect") || e.to_string().contains("connect") => {
                eprintln!("[GHOST] Connection failed, retrying in 2s: {}", e);
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                self.client.get(url)
                    .headers(headers)
                    .send()
                    .await
                    .map_err(|e| format!("HTTP error (retry): {e}"))?
            }
            Err(e) => return Err(format!("HTTP error: {e}")),
        };

        let status = resp.status().as_u16();
        let final_url = resp.url().to_string();

        // Store cookies from response
        if let Some(domain) = resp.url().host_str() {
            for cookie in resp.headers().get_all(SET_COOKIE) {
                if let Ok(s) = cookie.to_str() {
                    self.cookies.store_from_header(domain, s);
                }
            }
        }

        let html = resp.text().await.map_err(|e| format!("Body error: {e}"))?;
        let html_len = html.len();

        // Parse HTML
        let dom = parse_document(RcDom::default(), Default::default())
            .from_utf8()
            .read_from(&mut html.as_bytes())
            .map_err(|e| format!("Parse error: {e}"))?;

        let title = extract_title(&dom);
        let text = extract_text(&dom.document, 0);
        let links = extract_links(&dom.document, &final_url);
        let forms = extract_forms(&dom.document);
        let inputs = extract_inputs(&dom.document);
        let buttons = extract_buttons(&dom.document);

        // Detect SPA: big HTML shell but barely any visible content
        let is_spa = text.trim().len() < 200
            && html_len > 3000
            && inputs.is_empty()
            && buttons.is_empty();

        // AI's render: discover API endpoints from JS bundles
        let apis = if is_spa {
            eprintln!("[GHOST] SPA detected — scanning JS bundles for APIs...");
            let script_urls = extract_script_urls(&dom.document, &final_url);
            self.discover_apis(&script_urls, &final_url).await
        } else {
            // Even for SSR, scan inline scripts
            scan_js_for_apis(&html, "inline")
        };

        self.last_url = final_url.clone();
        self.last_html = html;
        self.history.push(final_url.clone());

        Ok(PageResult {
            url: final_url,
            status,
            title,
            text,
            links,
            forms,
            inputs,
            buttons,
            apis,
            html_len,
            is_spa,
        })
    }

    /// POST a form
    pub async fn submit_form(&mut self, url: &str, data: &HashMap<String, String>) -> Result<PageResult, String> {
        let headers = self.chrome_headers(url);
        let resp = match self.client.post(url)
            .headers(headers.clone())
            .form(data)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) if e.to_string().contains("Connect") || e.to_string().contains("connect") => {
                eprintln!("[GHOST] POST connection failed, retrying in 2s: {}", e);
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                self.client.post(url)
                    .headers(headers)
                    .form(data)
                    .send()
                    .await
                    .map_err(|e| format!("POST error (retry): {e}"))?
            }
            Err(e) => return Err(format!("POST error: {e}")),
        };

        let status = resp.status().as_u16();
        let final_url = resp.url().to_string();

        if let Some(domain) = resp.url().host_str() {
            for cookie in resp.headers().get_all(SET_COOKIE) {
                if let Ok(s) = cookie.to_str() {
                    self.cookies.store_from_header(domain, s);
                }
            }
        }

        let html = resp.text().await.map_err(|e| format!("Body error: {e}"))?;
        let html_len = html.len();

        let dom = parse_document(RcDom::default(), Default::default())
            .from_utf8()
            .read_from(&mut html.as_bytes())
            .map_err(|e| format!("Parse error: {e}"))?;

        let title = extract_title(&dom);
        let text = extract_text(&dom.document, 0);
        let links = extract_links(&dom.document, &final_url);
        let forms = extract_forms(&dom.document);
        let inputs = extract_inputs(&dom.document);
        let buttons = extract_buttons(&dom.document);

        self.last_url = final_url.clone();
        self.last_html = html;
        self.history.push(final_url.clone());

        Ok(PageResult {
            url: final_url,
            status,
            title,
            text,
            links,
            forms,
            inputs,
            buttons,
            apis: Vec::new(),
            html_len,
            is_spa: false,
        })
    }

    /// Fetch JSON API
    pub async fn fetch_json(&mut self, url: &str) -> Result<serde_json::Value, String> {
        let mut headers = self.chrome_headers(url);
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));

        let resp = match self.client.get(url)
            .headers(headers.clone())
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) if e.to_string().contains("Connect") || e.to_string().contains("connect") => {
                eprintln!("[GHOST] JSON fetch connection failed, retrying in 2s: {}", e);
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                self.client.get(url)
                    .headers(headers)
                    .send()
                    .await
                    .map_err(|e| format!("HTTP error (retry): {e}"))?
            }
            Err(e) => return Err(format!("HTTP error: {e}")),
        };

        if let Some(domain) = resp.url().host_str() {
            for cookie in resp.headers().get_all(SET_COOKIE) {
                if let Ok(s) = cookie.to_str() {
                    self.cookies.store_from_header(domain, s);
                }
            }
        }

        resp.json().await.map_err(|e| format!("JSON error: {e}"))
    }

    /// Load cookies from Chrome export file
    pub fn load_cookies(&mut self, path: &str) -> Result<usize, String> {
        self.cookies.load_file(path)
    }

    /// Call a discovered API endpoint directly — the AI's equivalent of "clicking"
    pub async fn call_api(&mut self, url: &str, method: &str, body: Option<&str>) -> Result<serde_json::Value, String> {
        self.call_api_with_headers(url, method, body, &[]).await
    }

    /// Call API with custom headers (e.g. Authorization: Bearer ...)
    pub async fn call_api_with_headers(&mut self, url: &str, method: &str, body: Option<&str>, extra: &[(String, String)]) -> Result<serde_json::Value, String> {
        let mut headers = self.chrome_headers(url);
        headers.insert(ACCEPT, HeaderValue::from_static("application/json, text/plain, */*"));
        headers.insert(
            rquest::header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        for (k, v) in extra {
            if let (Ok(name), Ok(val)) = (
                rquest::header::HeaderName::from_bytes(k.as_bytes()),
                HeaderValue::from_str(v),
            ) {
                headers.insert(name, val);
            }
        }

        let send_request = |headers: HeaderMap| {
            let client = &self.client;
            let body_str = body.unwrap_or("{}").to_string();
            async move {
                match method.to_uppercase().as_str() {
                    "POST" => client.post(url).headers(headers).body(body_str).send().await,
                    "PUT" => client.put(url).headers(headers).body(body_str).send().await,
                    "DELETE" => client.delete(url).headers(headers).send().await,
                    "PATCH" => client.patch(url).headers(headers).body(body_str).send().await,
                    _ => client.get(url).headers(headers).send().await,
                }
            }
        };

        let resp = match send_request(headers.clone()).await {
            Ok(r) => r,
            Err(e) if e.to_string().contains("Connect") || e.to_string().contains("connect") => {
                eprintln!("[GHOST] API call connection failed, retrying in 2s: {}", e);
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                send_request(headers).await
                    .map_err(|e| format!("API call error (retry): {e}"))?
            }
            Err(e) => return Err(format!("API call error: {e}")),
        };

        let status = resp.status().as_u16();

        // Store cookies
        if let Some(domain) = resp.url().host_str() {
            for cookie in resp.headers().get_all(SET_COOKIE) {
                if let Ok(s) = cookie.to_str() {
                    self.cookies.store_from_header(domain, s);
                }
            }
        }

        let text = resp.text().await.map_err(|e| format!("Body error: {e}"))?;

        // Try JSON first, fall back to text wrapper
        match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(json) => Ok(serde_json::json!({
                "status": status,
                "data": json,
            })),
            Err(_) => Ok(serde_json::json!({
                "status": status,
                "data": text,
            })),
        }
    }

    /// Discover API endpoints by fetching and scanning JS bundles
    async fn discover_apis(&self, script_urls: &[String], page_url: &str) -> Vec<ApiEndpoint> {
        let mut all_apis = Vec::new();
        let mut fetched_urls: Vec<String> = Vec::new();

        // Scan inline scripts from last_html
        all_apis.extend(scan_js_for_apis(&self.last_html, "inline"));

        // Fetch external JS bundles + follow ES module imports (depth 1)
        let mut urls_to_fetch: Vec<String> = script_urls.to_vec();

        for depth in 0..2 {  // depth 0 = direct scripts, depth 1 = imported modules
            let mut next_round = Vec::new();

            let mut handles = Vec::new();
            for script_url in urls_to_fetch.iter().take(10) {
                if fetched_urls.contains(script_url) { continue; }
                fetched_urls.push(script_url.clone());

                let client = self.client.clone();
                let url = script_url.clone();
                let base = page_url.to_string();
                let source = script_url.rsplit('/').next().unwrap_or("bundle.js").to_string();

                handles.push(tokio::spawn(async move {
                    match tokio::time::timeout(
                        std::time::Duration::from_secs(10),
                        client.get(&url).send()
                    ).await {
                        Ok(Ok(resp)) => {
                            if let Ok(js) = resp.text().await {
                                let apis = scan_js_for_apis(&js, &source);
                                // Extract ES module imports: import"./foo.js" or import("./foo.js")
                                let imports = extract_js_imports(&js, &url);
                                (apis, imports)
                            } else {
                                (Vec::new(), Vec::new())
                            }
                        }
                        _ => (Vec::new(), Vec::new()),
                    }
                }));
            }

            for handle in handles {
                if let Ok((apis, imports)) = handle.await {
                    all_apis.extend(apis);
                    if depth == 0 {
                        next_round.extend(imports);
                    }
                }
            }

            urls_to_fetch = next_round;
            if urls_to_fetch.is_empty() { break; }
        }

        // Deduplicate by URL
        all_apis.sort_by(|a, b| a.url.cmp(&b.url));
        all_apis.dedup_by(|a, b| a.url == b.url);

        eprintln!("[GHOST] Discovered {} API endpoints from {} scripts",
            all_apis.len(), fetched_urls.len());

        all_apis
    }
}

// ─── API Discovery — the AI's "render" ───

/// Scan JS source code for API endpoint patterns.
/// Two-pass strategy:
///   1. Find base URLs (API_URL, baseURL, etc.)
///   2. Find relative paths used with HTTP methods
///   3. Find absolute URLs
fn scan_js_for_apis(js: &str, source: &str) -> Vec<ApiEndpoint> {
    let mut apis = Vec::new();
    let mut base_urls = Vec::new();

    // ─── Pass 1: Find base URLs ───
    let base_patterns = &[
        r#"API_URL\s*[:=]\s*"([^"]+)""#,
        r#"api_?[Uu]rl\s*[:=]\s*"([^"]+)""#,
        r#"base_?[Uu][Rr][Ll]\s*[:=]\s*"([^"]+)""#,
        r#"BASE_URL\s*[:=]\s*"([^"]+)""#,
        r#"apiBase\s*[:=]\s*"([^"]+)""#,
    ];
    for pattern in base_patterns {
        if let Ok(re) = regex_lite::Regex::new(pattern) {
            for cap in re.captures_iter(js) {
                if let Some(url) = cap.get(1) {
                    let u = url.as_str().to_string();
                    if !u.is_empty() && u.len() < 200 {
                        base_urls.push(u.clone());
                        apis.push(ApiEndpoint {
                            url: u, method: "BASE".to_string(),
                            context: String::new(), source: source.to_string(),
                        });
                    }
                }
            }
        }
    }

    // ─── Pass 2: HTTP method calls with relative paths ───
    // .get("path"), .post("path"), etc. — the bread and butter of SPA API calls
    if let Ok(re) = regex_lite::Regex::new(r#"\.(get|post|put|delete|patch)\(\s*"([a-z][a-z0-9/_-]+[a-z0-9])""#) {
        for cap in re.captures_iter(js) {
            let method = cap.get(1).map(|m| m.as_str().to_uppercase()).unwrap_or_default();
            let path = cap.get(2).map(|m| m.as_str()).unwrap_or_default();
            if path.len() < 4 || path.len() > 100 { continue; }
            // Must contain a slash (API path) or be a known endpoint word
            if !path.contains('/') && !path.ends_with("-me") { continue; }
            // Skip non-API matches
            if is_noise_path(path) { continue; }

            let full = if let Some(base) = base_urls.first() {
                format!("{}/{}", base.trim_end_matches('/'), path)
            } else {
                path.to_string()
            };
            apis.push(ApiEndpoint {
                url: full, method,
                context: String::new(), source: source.to_string(),
            });
        }
    }

    // ─── Pass 3: Standalone path-like strings that look like API routes ───
    // "word/word" or "word/word/word" patterns (SPA relative endpoints)
    if let Ok(re) = regex_lite::Regex::new(r#""([a-z][a-z-]+/[a-z][a-z0-9/_-]+)""#) {
        for cap in re.captures_iter(js) {
            let path = cap.get(1).map(|m| m.as_str()).unwrap_or_default();
            if path.len() < 5 || path.len() > 80 { continue; }
            if is_noise_path(path) { continue; }
            // Must look like an API path (not a MIME type, not a URL fragment)
            if path.contains('.') { continue; } // MIME types have dots

            let full = if let Some(base) = base_urls.first() {
                format!("{}/{}", base.trim_end_matches('/'), path)
            } else {
                path.to_string()
            };
            // Avoid duplicates
            if apis.iter().any(|a| a.url == full) { continue; }
            apis.push(ApiEndpoint {
                url: full, method: "GET".to_string(),
                context: String::new(), source: source.to_string(),
            });
        }
    }

    // ─── Pass 4: Absolute URLs (fetch, full URLs) ───
    let abs_patterns = &[
        (r#"fetch\s*\(\s*"(https?://[^"]+)""#, "GET"),
        (r#""(https?://api[^"]+)""#, "GET"),
        (r#""(/api/v?\d*/[a-zA-Z][a-zA-Z0-9/_-]+)""#, "GET"),
        (r#""(/v[1-9]/[a-zA-Z][a-zA-Z0-9/_-]+)""#, "GET"),
        (r#""(/graphql[^"]*)""#, "POST"),
        (r#""(wss?://[^"]+)""#, "WS"),
    ];
    for &(pattern, method) in abs_patterns {
        if let Ok(re) = regex_lite::Regex::new(pattern) {
            for cap in re.captures_iter(js) {
                let url = cap.get(1).map(|m| m.as_str()).unwrap_or_default();
                if url.is_empty() || url.len() < 8 { continue; }
                if is_noise_url(url) { continue; }
                if apis.iter().any(|a| a.url == url) { continue; }
                apis.push(ApiEndpoint {
                    url: url.to_string(), method: method.to_string(),
                    context: String::new(), source: source.to_string(),
                });
            }
        }
    }

    apis
}

/// Filter out noise — paths that look like APIs but aren't
fn is_noise_path(path: &str) -> bool {
    // MIME types
    if path.starts_with("text/") || path.starts_with("image/") || path.starts_with("video/")
        || path.starts_with("audio/") || path.starts_with("application/")
        || path.starts_with("font/") || path.starts_with("multipart/") { return true; }
    // Common non-API patterns
    if path.starts_with("http") || path.starts_with("node_") || path.starts_with("@")
        || path.contains("sentry") || path.contains("webpack") || path.contains("babel")
        || path.contains("polyfill") || path.contains("chunk")
        || path.contains("source-map") || path.contains("localhost") { return true; }
    false
}

fn is_noise_url(url: &str) -> bool {
    url.ends_with(".js") || url.ends_with(".css") || url.ends_with(".png")
        || url.ends_with(".jpg") || url.ends_with(".svg") || url.ends_with(".woff")
        || url.ends_with(".map") || url.contains("webpack") || url.contains("polyfill")
        || url.contains("sentry.io") || url.contains("amplitude.com")
        || url.contains("localhost") || url.contains("example.com")
        || url.contains("w3.org") || url.contains("json-schema.org")
}

/// Extract ES module imports from JS source: import"./main.js", import("./chunk.js")
fn extract_js_imports(js: &str, script_url: &str) -> Vec<String> {
    let mut imports = Vec::new();
    // import"./path.js" or import "./path.js" or import("./path.js")
    let patterns = &[
        r#"import\s*"(\./[^"]+)""#,
        r#"import\s*'(\./[^']+)'"#,
        r#"import\(\s*"(\./[^"]+)""#,
        r#"import\(\s*'(\./[^']+)'"#,
    ];
    let base = if let Some(last_slash) = script_url.rfind('/') {
        &script_url[..=last_slash]
    } else {
        script_url
    };
    for pattern in patterns {
        if let Ok(re) = regex_lite::Regex::new(pattern) {
            for cap in re.captures_iter(js) {
                if let Some(path) = cap.get(1) {
                    let relative = path.as_str();
                    // Resolve relative to script URL
                    let full = if relative.starts_with("./") {
                        format!("{}{}", base, &relative[2..])
                    } else {
                        format!("{}{}", base, relative)
                    };
                    if !imports.contains(&full) {
                        imports.push(full);
                    }
                }
            }
        }
    }
    imports
}

/// Extract <script src="..."> URLs from parsed DOM
fn extract_script_urls(node: &markup5ever_rcdom::Handle, base_url: &str) -> Vec<String> {
    let mut urls = Vec::new();
    fn collect(node: &markup5ever_rcdom::Handle, base: &str, urls: &mut Vec<String>) {
        if let markup5ever_rcdom::NodeData::Element { name, attrs, .. } = &node.data {
            if name.local.as_ref() == "script" {
                let src = attrs.borrow().iter()
                    .find(|a| a.name.local.as_ref() == "src")
                    .map(|a| a.value.to_string());
                if let Some(src) = src {
                    let full = if src.starts_with("http") {
                        src
                    } else if src.starts_with("//") {
                        format!("https:{src}")
                    } else if let Ok(base_url) = url::Url::parse(base) {
                        base_url.join(&src).map(|u| u.to_string()).unwrap_or(src)
                    } else {
                        src
                    };
                    urls.push(full);
                }
            }
        }
        for child in node.children.borrow().iter() {
            collect(child, base, urls);
        }
    }
    collect(node, base_url, &mut urls);
    urls
}

// ─── DOM extraction helpers ───

use markup5ever_rcdom::{Handle, NodeData};
use markup5ever::local_name;

pub fn get_attr(node: &Handle, name: &str) -> String {
    match &node.data {
        NodeData::Element { attrs, .. } => {
            attrs.borrow().iter()
                .find(|a| a.name.local.as_ref() == name)
                .map(|a| a.value.to_string())
                .unwrap_or_default()
        }
        _ => String::new(),
    }
}

pub fn tag_name(node: &Handle) -> String {
    match &node.data {
        NodeData::Element { name, .. } => name.local.to_string(),
        _ => String::new(),
    }
}

fn extract_title(dom: &RcDom) -> String {
    fn find_title(node: &Handle) -> Option<String> {
        if tag_name(node) == "title" {
            let children = node.children.borrow();
            for child in children.iter() {
                if let NodeData::Text { contents } = &child.data {
                    return Some(contents.borrow().to_string());
                }
            }
        }
        for child in node.children.borrow().iter() {
            if let Some(t) = find_title(child) {
                return Some(t);
            }
        }
        None
    }
    find_title(&dom.document).unwrap_or_default()
}

pub fn extract_text(node: &Handle, depth: usize) -> String {
    if depth > 50 { return String::new(); }
    let tag = tag_name(node);
    // Skip invisible tags
    if matches!(tag.as_str(), "script" | "style" | "noscript" | "svg" | "head") {
        return String::new();
    }

    let mut text = String::new();
    match &node.data {
        NodeData::Text { contents } => {
            let t = contents.borrow().to_string();
            let t = t.trim();
            if !t.is_empty() {
                text.push_str(t);
                text.push(' ');
            }
        }
        _ => {}
    }

    for child in node.children.borrow().iter() {
        text.push_str(&extract_text(child, depth + 1));
    }

    // Add newline after block elements
    if matches!(tag.as_str(), "p" | "div" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "li" | "tr" | "br" | "hr") {
        text.push('\n');
    }

    text
}

fn extract_links(node: &Handle, base_url: &str) -> Vec<Link> {
    let mut links = Vec::new();
    fn collect(node: &Handle, base: &str, links: &mut Vec<Link>) {
        if tag_name(node) == "a" {
            let href = get_attr(node, "href");
            if !href.is_empty() && !href.starts_with('#') && !href.starts_with("javascript:") {
                let full = if href.starts_with("http") {
                    href.clone()
                } else if let Ok(base_url) = url::Url::parse(base) {
                    base_url.join(&href).map(|u| u.to_string()).unwrap_or(href.clone())
                } else {
                    href.clone()
                };
                let text = extract_text(node, 0).trim().to_string();
                links.push(Link { text, href: full });
            }
        }
        for child in node.children.borrow().iter() {
            collect(child, base, links);
        }
    }
    collect(node, base_url, &mut links);
    links
}

fn extract_forms(node: &Handle) -> Vec<Form> {
    let mut forms = Vec::new();
    fn collect(node: &Handle, forms: &mut Vec<Form>) {
        if tag_name(node) == "form" {
            let action = get_attr(node, "action");
            let method = get_attr(node, "method").to_uppercase();
            let method = if method.is_empty() { "GET".to_string() } else { method };
            let fields = collect_form_fields(node);
            forms.push(Form { action, method, fields });
        }
        for child in node.children.borrow().iter() {
            collect(child, forms);
        }
    }
    collect(node, &mut forms);
    forms
}

fn collect_form_fields(node: &Handle) -> Vec<FormField> {
    let mut fields = Vec::new();
    fn collect(node: &Handle, fields: &mut Vec<FormField>) {
        let tag = tag_name(node);
        if tag == "input" || tag == "select" || tag == "textarea" {
            let name = get_attr(node, "name");
            if !name.is_empty() {
                fields.push(FormField {
                    name,
                    field_type: get_attr(node, "type"),
                    value: get_attr(node, "value"),
                });
            }
        }
        for child in node.children.borrow().iter() {
            collect(child, fields);
        }
    }
    collect(node, &mut fields);
    fields
}

fn extract_inputs(node: &Handle) -> Vec<Input> {
    let mut inputs = Vec::new();
    fn collect(node: &Handle, inputs: &mut Vec<Input>) {
        if tag_name(node) == "input" {
            let name = get_attr(node, "name");
            let id = get_attr(node, "id");
            if !name.is_empty() || !id.is_empty() {
                inputs.push(Input {
                    name,
                    input_type: get_attr(node, "type"),
                    placeholder: get_attr(node, "placeholder"),
                    value: get_attr(node, "value"),
                    id,
                });
            }
        }
        for child in node.children.borrow().iter() {
            collect(child, inputs);
        }
    }
    collect(node, &mut inputs);
    inputs
}

fn extract_buttons(node: &Handle) -> Vec<Button> {
    let mut buttons = Vec::new();
    fn collect(node: &Handle, buttons: &mut Vec<Button>) {
        let tag = tag_name(node);
        if tag == "button" || (tag == "input" && matches!(get_attr(node, "type").as_str(), "submit" | "button")) {
            let text = if tag == "button" {
                extract_text(node, 0).trim().to_string()
            } else {
                get_attr(node, "value")
            };
            buttons.push(Button {
                text,
                button_type: get_attr(node, "type"),
                name: get_attr(node, "name"),
            });
        }
        for child in node.children.borrow().iter() {
            collect(child, buttons);
        }
    }
    collect(node, &mut buttons);
    buttons
}

// ─── Compact "see" format for AI ───

impl PageResult {
    /// Compact representation for AI consumption — like WOM but from HTTP
    pub fn to_see(&self, max_text: usize) -> String {
        let mut out = String::new();
        out.push_str(&format!("Page: {}\nURL: {}\nStatus: {}\n", self.title, self.url, self.status));

        // Text (truncated)
        let text = if self.text.len() > max_text {
            format!("{}...", &self.text[..max_text])
        } else {
            self.text.clone()
        };
        if !text.is_empty() {
            out.push_str(&format!("\n--- Text ---\n{}\n", text.trim()));
        }

        // Links
        if !self.links.is_empty() {
            out.push_str(&format!("\n--- Links ({}) ---\n", self.links.len()));
            for (i, link) in self.links.iter().take(50).enumerate() {
                let label = if link.text.is_empty() { "[no text]" } else { &link.text };
                out.push_str(&format!("[{}] {} → {}\n", i, label, link.href));
            }
        }

        // Forms
        if !self.forms.is_empty() {
            out.push_str(&format!("\n--- Forms ({}) ---\n", self.forms.len()));
            for form in &self.forms {
                out.push_str(&format!("FORM {} {} fields={:?}\n", form.method, form.action,
                    form.fields.iter().map(|f| format!("{}({})", f.name, f.field_type)).collect::<Vec<_>>()));
            }
        }

        // Inputs outside forms
        if !self.inputs.is_empty() {
            out.push_str(&format!("\n--- Inputs ({}) ---\n", self.inputs.len()));
            for inp in &self.inputs {
                out.push_str(&format!("INPUT {}({}) id={} placeholder={}\n",
                    inp.name, inp.input_type, inp.id, inp.placeholder));
            }
        }

        // Buttons
        if !self.buttons.is_empty() {
            out.push_str(&format!("\n--- Buttons ({}) ---\n", self.buttons.len()));
            for btn in &self.buttons {
                out.push_str(&format!("BTN \"{}\" type={}\n", btn.text, btn.button_type));
            }
        }

        // API Endpoints (AI's render of SPAs)
        if !self.apis.is_empty() {
            out.push_str(&format!("\n--- API Endpoints ({}) ---\n", self.apis.len()));
            for api in &self.apis {
                out.push_str(&format!("{} {} [{}]\n", api.method, api.url, api.source));
            }
        }

        if self.is_spa {
            out.push_str("\n[SPA] This page is a JS app. Use the API endpoints above to interact.\n");
        }

        out
    }
}
