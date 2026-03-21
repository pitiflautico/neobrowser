//! Mock implementations for testing without network or disk I/O.

use crate::{CacheDecision, CookieStore, HttpCache, HttpClient, HttpError, HttpRequest, WebStorage};
use neo_types::{Cookie, HttpResponse};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug)]
pub struct MockHttpClient {
    rules: Mutex<Vec<MockRule>>,
    recorded: Mutex<Vec<HttpRequest>>,
}

#[derive(Debug, Clone)]
struct MockRule { url_pattern: String, response: HttpResponse }

#[derive(Debug)]
pub struct MockRuleBuilder<'a> { client: &'a MockHttpClient, pattern: String }

impl MockHttpClient {
    pub fn new() -> Self { Self { rules: Mutex::new(Vec::new()), recorded: Mutex::new(Vec::new()) } }
    pub fn when_url(&self, pattern: &str) -> MockRuleBuilder<'_> { MockRuleBuilder { client: self, pattern: pattern.to_string() } }
    pub fn requests(&self) -> Vec<HttpRequest> { self.recorded.lock().expect("lock poisoned").clone() }
}
impl Default for MockHttpClient { fn default() -> Self { Self::new() } }
impl<'a> MockRuleBuilder<'a> {
    pub fn returns(self, response: HttpResponse) {
        self.client.rules.lock().expect("lock poisoned").push(MockRule { url_pattern: self.pattern, response });
    }
}
impl HttpClient for MockHttpClient {
    fn request(&self, req: &HttpRequest) -> Result<HttpResponse, HttpError> {
        self.recorded.lock().expect("lock poisoned").push(req.clone());
        let rules = self.rules.lock().expect("lock poisoned");
        for rule in rules.iter() { if req.url.contains(&rule.url_pattern) { return Ok(rule.response.clone()); } }
        Err(HttpError::Network(format!("no mock rule for URL: {}", req.url)))
    }
}

#[derive(Debug, Default)]
pub struct InMemoryCookieStore { cookies: Mutex<HashMap<String, Cookie>> }
impl InMemoryCookieStore { pub fn new() -> Self { Self::default() } }
impl CookieStore for InMemoryCookieStore {
    fn get_for_request(&self, url: &str, _top_level_url: Option<&str>, _is_top_level: bool) -> String {
        let host = url::Url::parse(url).map(|u| u.host_str().unwrap_or("").to_string()).unwrap_or_default();
        let store = self.cookies.lock().expect("lock poisoned");
        store.values().filter(|c| host.contains(&c.domain) || c.domain.contains(&host)).map(|c| format!("{}={}", c.name, c.value)).collect::<Vec<_>>().join("; ")
    }
    fn store_set_cookie(&self, url: &str, set_cookie: &str) {
        let host = url::Url::parse(url).map(|u| u.host_str().unwrap_or("").to_string()).unwrap_or_default();
        let first = set_cookie.split(';').next().unwrap_or("");
        let (name, value) = first.split_once('=').unwrap_or((first, ""));
        let cookie = Cookie { name: name.trim().to_string(), value: value.trim().to_string(), domain: host, path: "/".to_string(), expires: None, http_only: false, secure: false, same_site: None };
        let key = format!("{}:{}", cookie.domain, cookie.name);
        self.cookies.lock().expect("lock poisoned").insert(key, cookie);
    }
    fn delete(&self, name: &str, domain: &str, _path: &str) { self.cookies.lock().expect("lock poisoned").remove(&format!("{domain}:{name}")); }
    fn evict_expired(&self) {}
    fn clear_session(&self) { self.cookies.lock().expect("lock poisoned").clear(); }
    fn list_for_domain(&self, domain: &str) -> Vec<Cookie> { self.cookies.lock().expect("lock poisoned").values().filter(|c| c.domain == domain).cloned().collect() }
    fn export(&self) -> Vec<Cookie> { self.cookies.lock().expect("lock poisoned").values().cloned().collect() }
    fn import(&self, cookies: &[Cookie]) { let mut store = self.cookies.lock().expect("lock poisoned"); for c in cookies { store.insert(format!("{}:{}", c.domain, c.name), c.clone()); } }
    fn snapshot(&self) -> Vec<Cookie> { self.export() }
}

#[derive(Debug, Default)]
pub struct NoopCache;
impl NoopCache { pub fn new() -> Self { Self } }
impl HttpCache for NoopCache {
    fn lookup(&self, _req: &HttpRequest) -> CacheDecision { CacheDecision::Miss }
    fn store(&self, _req: &HttpRequest, _response: &HttpResponse) {}
    fn invalidate(&self, _pattern: &str) {}
    fn is_fresh(&self, _url: &str) -> bool { false }
}

#[derive(Debug, Default)]
pub struct InMemoryWebStorage { data: Mutex<HashMap<String, HashMap<String, String>>> }
impl InMemoryWebStorage { pub fn new() -> Self { Self::default() } }
impl WebStorage for InMemoryWebStorage {
    fn get(&self, origin: &str, key: &str) -> Option<String> { self.data.lock().expect("lock poisoned").get(origin).and_then(|m| m.get(key)).cloned() }
    fn set(&self, origin: &str, key: &str, value: &str) { self.data.lock().expect("lock poisoned").entry(origin.to_string()).or_default().insert(key.to_string(), value.to_string()); }
    fn remove(&self, origin: &str, key: &str) { if let Some(m) = self.data.lock().expect("lock poisoned").get_mut(origin) { m.remove(key); } }
    fn clear(&self, origin: &str) { self.data.lock().expect("lock poisoned").remove(origin); }
    fn keys(&self, origin: &str) -> Vec<String> { self.data.lock().expect("lock poisoned").get(origin).map(|m| { let mut k: Vec<String> = m.keys().cloned().collect(); k.sort(); k }).unwrap_or_default() }
    fn len(&self, origin: &str) -> usize { self.data.lock().expect("lock poisoned").get(origin).map(|m| m.len()).unwrap_or(0) }
}
