//! BrowserNetwork — Fetch Standard networking with automatic browser headers.
//!
//! Replaces manual header hacks in ops.rs with a proper implementation of:
//! - Sec-Fetch-* headers (Fetch Metadata Request Headers spec)
//! - Referrer policy (W3C Referrer Policy spec)
//! - Origin header (only sent when spec requires it)

use std::sync::Arc;

pub mod headers;
pub mod referrer;

/// Browser-like networking with automatic Fetch Standard headers.
pub struct BrowserNetwork {
    client: Arc<rquest::Client>,
    origin: String,          // Current page origin (e.g. "https://chatgpt.com")
    url: String,             // Current page URL
    referrer_policy: ReferrerPolicy,
}

#[derive(Clone, Debug)]
pub enum ReferrerPolicy {
    StrictOriginWhenCrossOrigin, // default
    NoReferrer,
    Origin,
    SameOrigin,
}

#[derive(Debug)]
pub enum RequestMode {
    Cors,       // fetch() from JS
    Navigate,   // page navigation
    NoCors,     // img, script tags
    SameOrigin,
}

#[derive(Debug)]
pub enum RequestDestination {
    Empty,      // fetch()
    Document,   // navigation
    Script,     // <script>
    Style,      // <link rel=stylesheet>
    Image,      // <img>
}

pub struct FetchResponse {
    pub status: u16,
    pub body: String,
    pub headers: std::collections::HashMap<String, String>,
}

const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36";

/// Build a full set of Chrome 136 navigation headers (for goto/navigate).
/// These match what a real Chrome sends on document navigations.
pub fn navigation_headers() -> rquest::header::HeaderMap {
    let mut h = rquest::header::HeaderMap::new();
    h.insert(rquest::header::ACCEPT,
        rquest::header::HeaderValue::from_static("text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7"));
    h.insert(rquest::header::ACCEPT_LANGUAGE,
        rquest::header::HeaderValue::from_static("es-ES,es;q=0.9,en;q=0.8"));
    h.insert(rquest::header::ACCEPT_ENCODING,
        rquest::header::HeaderValue::from_static("gzip, deflate, br, zstd"));
    h.insert(rquest::header::UPGRADE_INSECURE_REQUESTS,
        rquest::header::HeaderValue::from_static("1"));
    h.insert(rquest::header::CACHE_CONTROL,
        rquest::header::HeaderValue::from_static("max-age=0"));
    h.insert("Sec-Ch-Ua",
        rquest::header::HeaderValue::from_static("\"Chromium\";v=\"136\", \"Not_A Brand\";v=\"24\", \"Google Chrome\";v=\"136\""));
    h.insert("Sec-Ch-Ua-Mobile",
        rquest::header::HeaderValue::from_static("?0"));
    h.insert("Sec-Ch-Ua-Platform",
        rquest::header::HeaderValue::from_static("\"macOS\""));
    h.insert("Sec-Fetch-Dest",
        rquest::header::HeaderValue::from_static("document"));
    h.insert("Sec-Fetch-Mode",
        rquest::header::HeaderValue::from_static("navigate"));
    h.insert("Sec-Fetch-Site",
        rquest::header::HeaderValue::from_static("none"));
    h.insert("Sec-Fetch-User",
        rquest::header::HeaderValue::from_static("?1"));
    h
}

/// Build headers for XHR/fetch requests (from JS).
/// Lighter than navigation — no Upgrade-Insecure-Requests, Accept is JSON-friendly.
pub fn fetch_headers() -> rquest::header::HeaderMap {
    let mut h = rquest::header::HeaderMap::new();
    h.insert(rquest::header::ACCEPT,
        rquest::header::HeaderValue::from_static("application/json, text/plain, */*"));
    h.insert(rquest::header::ACCEPT_LANGUAGE,
        rquest::header::HeaderValue::from_static("es-ES,es;q=0.9,en;q=0.8"));
    h.insert(rquest::header::ACCEPT_ENCODING,
        rquest::header::HeaderValue::from_static("gzip, deflate, br, zstd"));
    h.insert("Sec-Ch-Ua",
        rquest::header::HeaderValue::from_static("\"Chromium\";v=\"136\", \"Not_A Brand\";v=\"24\", \"Google Chrome\";v=\"136\""));
    h.insert("Sec-Ch-Ua-Mobile",
        rquest::header::HeaderValue::from_static("?0"));
    h.insert("Sec-Ch-Ua-Platform",
        rquest::header::HeaderValue::from_static("\"macOS\""));
    h
}

/// Send+Sync snapshot of BrowserNetwork state for V8's OpState.
/// OpState requires Send+Sync; BrowserNetwork itself lives in NeoSession (not Send).
/// This handle is stored in OpState and read by op_neorender_fetch.
#[derive(Clone)]
pub struct BrowserNetworkHandle {
    pub client: Arc<rquest::Client>,
    pub origin: String,
    pub url: String,
    pub referrer_policy: ReferrerPolicy,
}

impl BrowserNetwork {
    pub fn new(client: Arc<rquest::Client>) -> Self {
        Self {
            client,
            origin: String::new(),
            url: String::new(),
            referrer_policy: ReferrerPolicy::StrictOriginWhenCrossOrigin,
        }
    }

    /// Reconstruct from parts (used by op_neorender_fetch on the worker thread).
    pub fn from_parts(
        client: Arc<rquest::Client>,
        origin: &str,
        url: &str,
        referrer_policy: ReferrerPolicy,
    ) -> Self {
        Self {
            client,
            origin: origin.to_string(),
            url: url.to_string(),
            referrer_policy,
        }
    }

    /// Create a Send+Sync handle for storing in V8's OpState.
    pub fn to_handle(&self) -> BrowserNetworkHandle {
        BrowserNetworkHandle {
            client: self.client.clone(),
            origin: self.origin.clone(),
            url: self.url.clone(),
            referrer_policy: self.referrer_policy.clone(),
        }
    }

    /// Update page context on navigation. Called by NeoSession::goto().
    pub fn set_page(&mut self, url: &str) {
        self.url = url.to_string();
        self.origin = url::Url::parse(url)
            .ok()
            .map(|u| u.origin().ascii_serialization())
            .unwrap_or_default();
    }

    /// Current page origin (for OpState backward compat).
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// Current page URL (for OpState backward compat).
    pub fn page_url(&self) -> &str {
        &self.url
    }

    /// Reference to the underlying HTTP client.
    pub fn client(&self) -> &Arc<rquest::Client> {
        &self.client
    }

    /// Standard fetch with all browser headers automatically applied.
    pub async fn fetch(
        &self,
        url: &str,
        method: &str,
        body: Option<&str>,
        custom_headers: Option<&str>,
        mode: RequestMode,
        destination: RequestDestination,
    ) -> Result<FetchResponse, String> {
        let req = match method {
            "POST" => self.client.post(url),
            "PUT" => self.client.put(url),
            "DELETE" => self.client.delete(url),
            "PATCH" => self.client.patch(url),
            _ => self.client.get(url),
        };

        // Apply base fetch headers (Sec-Ch-Ua, Accept-Language, Accept-Encoding, Accept)
        let base_headers = fetch_headers();
        let mut req = req.headers(base_headers);

        // Sec-Fetch-* headers (only if we have a page context)
        if !self.origin.is_empty() {
            let target_origin = url::Url::parse(url)
                .ok()
                .map(|u| u.origin().ascii_serialization())
                .unwrap_or_default();

            // Sec-Fetch-Site
            req = req.header("Sec-Fetch-Site", headers::sec_fetch_site(&target_origin, &self.origin));

            // Sec-Fetch-Mode
            req = req.header("Sec-Fetch-Mode", headers::sec_fetch_mode(&mode));

            // Sec-Fetch-Dest
            req = req.header("Sec-Fetch-Dest", headers::sec_fetch_dest(&destination));

            // Origin header: sent for CORS requests and non-GET/HEAD
            match mode {
                RequestMode::Cors => {
                    req = req.header("Origin", &self.origin);
                }
                RequestMode::Navigate => {
                    // Navigation: Origin sent only for POST
                    if method == "POST" {
                        req = req.header("Origin", &self.origin);
                    }
                }
                _ => {
                    // NoCors/SameOrigin: Origin for non-GET/HEAD
                    if method != "GET" && method != "HEAD" {
                        req = req.header("Origin", &self.origin);
                    }
                }
            }

            // Referer header per policy
            if let Some(referer) = referrer::compute_referrer(&self.url, url, &self.referrer_policy) {
                req = req.header("Referer", referer);
            }

            // Sec-Fetch-User: only for user-activated navigations (we don't track this, omit)
        }

        // Custom headers from JS (override auto-generated ones)
        if let Some(json) = custom_headers {
            if !json.is_empty() {
                if let Ok(hdrs) = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(json) {
                    for (key, val) in hdrs {
                        if let Some(v) = val.as_str() {
                            if let Ok(hname) = rquest::header::HeaderName::from_bytes(key.as_bytes()) {
                                if let Ok(hval) = rquest::header::HeaderValue::from_str(v) {
                                    req = req.header(hname, hval);
                                }
                            }
                        }
                    }
                }
            }
        }

        // Body
        let resp = req
            .body(body.unwrap_or("").to_string())
            .send()
            .await
            .map_err(|e| format!("Fetch: {e}"))?;

        let status = resp.status().as_u16();

        let mut resp_headers = std::collections::HashMap::new();
        for (name, val) in resp.headers() {
            if let Ok(v) = val.to_str() {
                resp_headers.insert(name.as_str().to_string(), v.to_string());
            }
        }

        let resp_body = resp.text().await.unwrap_or_default();

        Ok(FetchResponse {
            status,
            body: resp_body,
            headers: resp_headers,
        })
    }
}
