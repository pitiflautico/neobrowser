//! Mock implementations for testing without network or disk I/O.

use crate::{CacheDecision, CookieStore, HttpCache, HttpClient, HttpError, HttpRequest};
use neo_types::{Cookie, HttpResponse};
use std::collections::HashMap;
use std::sync::Mutex;

/// Mock HTTP client with configurable per-URL responses.
///
/// Records all requests for later inspection. Responses are matched
/// by substring against the request URL.
#[derive(Debug)]
pub struct MockHttpClient {
    rules: Mutex<Vec<MockRule>>,
    recorded: Mutex<Vec<HttpRequest>>,
}

/// A pattern-to-response mapping rule.
#[derive(Debug, Clone)]
struct MockRule {
    url_pattern: String,
    response: HttpResponse,
}

/// Builder for adding response rules.
#[derive(Debug)]
pub struct MockRuleBuilder<'a> {
    client: &'a MockHttpClient,
    pattern: String,
}

impl MockHttpClient {
    /// Create a new mock client with no rules.
    pub fn new() -> Self {
        Self {
            rules: Mutex::new(Vec::new()),
            recorded: Mutex::new(Vec::new()),
        }
    }

    /// Start building a rule: when the URL contains `pattern`...
    pub fn when_url(&self, pattern: &str) -> MockRuleBuilder<'_> {
        MockRuleBuilder {
            client: self,
            pattern: pattern.to_string(),
        }
    }

    /// Get all recorded requests.
    pub fn requests(&self) -> Vec<HttpRequest> {
        self.recorded.lock().expect("lock poisoned").clone()
    }
}

impl Default for MockHttpClient {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> MockRuleBuilder<'a> {
    /// Complete the rule: return this response when matched.
    pub fn returns(self, response: HttpResponse) {
        let rule = MockRule {
            url_pattern: self.pattern,
            response,
        };
        self.client.rules.lock().expect("lock poisoned").push(rule);
    }
}

impl HttpClient for MockHttpClient {
    /// Match the URL against rules and return the configured response.
    ///
    /// Returns `HttpError::Network` if no rule matches.
    fn request(&self, req: &HttpRequest) -> Result<HttpResponse, HttpError> {
        self.recorded
            .lock()
            .expect("lock poisoned")
            .push(req.clone());

        let rules = self.rules.lock().expect("lock poisoned");
        for rule in rules.iter() {
            if req.url.contains(&rule.url_pattern) {
                return Ok(rule.response.clone());
            }
        }
        Err(HttpError::Network(format!(
            "no mock rule for URL: {}",
            req.url
        )))
    }
}

/// In-memory cookie store backed by a HashMap (no SQLite).
///
/// Useful for unit tests that don't need persistence.
#[derive(Debug, Default)]
pub struct InMemoryCookieStore {
    cookies: Mutex<HashMap<String, Cookie>>,
}

impl InMemoryCookieStore {
    /// Create an empty in-memory cookie store.
    pub fn new() -> Self {
        Self::default()
    }
}

impl CookieStore for InMemoryCookieStore {
    /// Get cookies for a request (simplified: returns all cookies for domain).
    fn get_for_request(
        &self,
        url: &str,
        _top_level_url: Option<&str>,
        _is_top_level: bool,
    ) -> String {
        let host = url::Url::parse(url)
            .map(|u| u.host_str().unwrap_or("").to_string())
            .unwrap_or_default();
        let store = self.cookies.lock().expect("lock poisoned");
        store
            .values()
            .filter(|c| host.contains(&c.domain) || c.domain.contains(&host))
            .map(|c| format!("{}={}", c.name, c.value))
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// Store a cookie from a Set-Cookie header (simplified parsing).
    fn store_set_cookie(&self, url: &str, set_cookie: &str) {
        let host = url::Url::parse(url)
            .map(|u| u.host_str().unwrap_or("").to_string())
            .unwrap_or_default();
        let first = set_cookie.split(';').next().unwrap_or("");
        let (name, value) = first.split_once('=').unwrap_or((first, ""));
        let cookie = Cookie {
            name: name.trim().to_string(),
            value: value.trim().to_string(),
            domain: host,
            path: "/".to_string(),
            expires: None,
            http_only: false,
            secure: false,
            same_site: None,
        };
        let key = format!("{}:{}", cookie.domain, cookie.name);
        self.cookies
            .lock()
            .expect("lock poisoned")
            .insert(key, cookie);
    }

    fn delete(&self, name: &str, domain: &str, _path: &str) {
        let key = format!("{domain}:{name}");
        self.cookies.lock().expect("lock poisoned").remove(&key);
    }

    fn evict_expired(&self) {}
    fn clear_session(&self) {
        self.cookies.lock().expect("lock poisoned").clear();
    }

    fn list_for_domain(&self, domain: &str) -> Vec<Cookie> {
        self.cookies
            .lock()
            .expect("lock poisoned")
            .values()
            .filter(|c| c.domain == domain)
            .cloned()
            .collect()
    }

    fn export(&self) -> Vec<Cookie> {
        self.cookies
            .lock()
            .expect("lock poisoned")
            .values()
            .cloned()
            .collect()
    }

    fn import(&self, cookies: &[Cookie]) {
        let mut store = self.cookies.lock().expect("lock poisoned");
        for c in cookies {
            let key = format!("{}:{}", c.domain, c.name);
            store.insert(key, c.clone());
        }
    }

    fn snapshot(&self) -> Vec<Cookie> {
        self.export()
    }
}

/// Cache that always returns Miss (no caching).
///
/// Useful for tests that don't need cache behavior.
#[derive(Debug, Default)]
pub struct NoopCache;

impl NoopCache {
    /// Create a new no-op cache.
    pub fn new() -> Self {
        Self
    }
}

impl HttpCache for NoopCache {
    fn lookup(&self, _req: &HttpRequest) -> CacheDecision {
        CacheDecision::Miss
    }
    fn store(&self, _req: &HttpRequest, _response: &HttpResponse) {}
    fn invalidate(&self, _pattern: &str) {}
    fn is_fresh(&self, _url: &str) -> bool {
        false
    }
}
