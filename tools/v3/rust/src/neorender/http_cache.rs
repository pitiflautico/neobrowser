//! HTTP response cache for NeoRender.
//!
//! Caches GET responses to avoid re-fetching static resources.
//! Respects Cache-Control, ETag, Last-Modified, Expires headers.
//! LRU eviction when over max size.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Cached HTTP response returned to callers.
pub struct CachedResponse {
    pub body: String,
    pub status: u16,
    pub headers: HashMap<String, String>,
}

/// HTTP response cache with LRU eviction.
pub struct HttpCache {
    entries: HashMap<String, CacheEntry>,
    max_size: usize,   // bytes
    current_size: usize,
}

struct CacheEntry {
    body: String,
    status: u16,
    headers: HashMap<String, String>,
    inserted: Instant,
    last_accessed: Instant,
    max_age: Duration,
    etag: Option<String>,
    last_modified: Option<String>,
}

impl HttpCache {
    /// Create a new cache with the given max size in megabytes.
    pub fn new(max_size_mb: usize) -> Self {
        Self {
            entries: HashMap::new(),
            max_size: max_size_mb * 1024 * 1024,
            current_size: 0,
        }
    }

    /// Check if URL is cached and not expired. Returns None if miss or expired.
    pub fn get(&mut self, url: &str) -> Option<CachedResponse> {
        let now = Instant::now();
        let entry = self.entries.get_mut(url)?;

        // Check expiry
        if now.duration_since(entry.inserted) > entry.max_age {
            // Expired — don't remove yet (conditional headers may still be useful)
            return None;
        }

        entry.last_accessed = now;

        Some(CachedResponse {
            body: entry.body.clone(),
            status: entry.status,
            headers: entry.headers.clone(),
        })
    }

    /// Store a response in cache. Only caches GET 200 responses that are cacheable.
    pub fn store(&mut self, url: &str, status: u16, headers: &HashMap<String, String>, body: &str) {
        if !Self::is_cacheable(status, headers) {
            return;
        }

        let max_age = Self::parse_max_age(headers)
            .or_else(|| Self::parse_expires(headers))
            .unwrap_or(Duration::from_secs(300)); // 5 min default for cacheable responses

        let etag = headers.get("etag").or_else(|| headers.get("ETag")).cloned();
        let last_modified = headers.get("last-modified")
            .or_else(|| headers.get("Last-Modified"))
            .cloned();

        let body_size = body.len();

        // Remove old entry if exists
        if let Some(old) = self.entries.remove(url) {
            self.current_size = self.current_size.saturating_sub(old.body.len());
        }

        // Evict if needed
        while self.current_size + body_size > self.max_size && !self.entries.is_empty() {
            self.evict();
        }

        // Don't cache if single entry exceeds max size
        if body_size > self.max_size {
            return;
        }

        let now = Instant::now();
        self.entries.insert(url.to_string(), CacheEntry {
            body: body.to_string(),
            status,
            headers: headers.clone(),
            inserted: now,
            last_accessed: now,
            max_age,
            etag,
            last_modified,
        });
        self.current_size += body_size;
    }

    /// Check Cache-Control header to decide cacheability.
    /// Only cache GET 200 without no-store, no-cache, or Set-Cookie.
    fn is_cacheable(status: u16, headers: &HashMap<String, String>) -> bool {
        if status != 200 {
            return false;
        }

        // Don't cache responses with Set-Cookie
        if headers.contains_key("set-cookie") || headers.contains_key("Set-Cookie") {
            return false;
        }

        let cc = headers.get("cache-control")
            .or_else(|| headers.get("Cache-Control"))
            .map(|v| v.to_lowercase())
            .unwrap_or_default();

        if cc.contains("no-store") || cc.contains("no-cache") || cc.contains("private") {
            return false;
        }

        true
    }

    /// Parse max-age from Cache-Control header.
    fn parse_max_age(headers: &HashMap<String, String>) -> Option<Duration> {
        let cc = headers.get("cache-control")
            .or_else(|| headers.get("Cache-Control"))?;
        let cc_lower = cc.to_lowercase();

        for directive in cc_lower.split(',') {
            let directive = directive.trim();
            if let Some(val) = directive.strip_prefix("max-age=") {
                if let Ok(secs) = val.trim().parse::<u64>() {
                    return Some(Duration::from_secs(secs));
                }
            }
        }
        None
    }

    /// Parse Expires header into a Duration from now.
    fn parse_expires(headers: &HashMap<String, String>) -> Option<Duration> {
        let expires_str = headers.get("expires")
            .or_else(|| headers.get("Expires"))?;

        // Parse HTTP date format: "Thu, 01 Dec 2025 16:00:00 GMT"
        // Use a simple approach: try to parse with chrono-like manual parsing
        // For robustness, we just check if it looks valid and give a conservative TTL
        if expires_str.trim() == "0" || expires_str.trim() == "-1" {
            return None; // Already expired
        }

        // Try parsing RFC 2822 / HTTP date
        if let Ok(ts) = httpdate::parse_http_date(expires_str) {
            let now = std::time::SystemTime::now();
            if let Ok(dur) = ts.duration_since(now) {
                return Some(dur);
            }
            return None; // Already in the past
        }

        None
    }

    /// Evict the least recently accessed entry.
    fn evict(&mut self) {
        let lru_url = self.entries.iter()
            .min_by_key(|(_, e)| e.last_accessed)
            .map(|(url, _)| url.clone());

        if let Some(url) = lru_url {
            if let Some(entry) = self.entries.remove(&url) {
                self.current_size = self.current_size.saturating_sub(entry.body.len());
            }
        }
    }

    /// Get conditional request headers for a URL (If-None-Match, If-Modified-Since).
    /// Returns empty map if URL not in cache.
    pub fn conditional_headers(&self, url: &str) -> HashMap<String, String> {
        let mut h = HashMap::new();
        if let Some(entry) = self.entries.get(url) {
            if let Some(ref etag) = entry.etag {
                h.insert("If-None-Match".to_string(), etag.clone());
            }
            if let Some(ref lm) = entry.last_modified {
                h.insert("If-Modified-Since".to_string(), lm.clone());
            }
        }
        h
    }

    /// Update the last_accessed time on a cache hit (for 304 responses).
    /// Also refreshes the inserted time so the entry doesn't expire prematurely.
    pub fn touch(&mut self, url: &str) {
        if let Some(entry) = self.entries.get_mut(url) {
            let now = Instant::now();
            entry.last_accessed = now;
            entry.inserted = now;
        }
    }

    /// Clear all cached entries.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.current_size = 0;
    }

    /// Current cache size in bytes.
    pub fn size(&self) -> usize {
        self.current_size
    }

    /// Number of cached entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Is the cache empty?
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_headers(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn test_store_and_get() {
        let mut cache = HttpCache::new(1); // 1MB
        let headers = make_headers(&[("cache-control", "max-age=3600")]);
        cache.store("https://example.com/style.css", 200, &headers, "body{color:red}");
        assert_eq!(cache.len(), 1);

        let resp = cache.get("https://example.com/style.css").unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body, "body{color:red}");
    }

    #[test]
    fn test_no_store_not_cached() {
        let mut cache = HttpCache::new(1);
        let headers = make_headers(&[("cache-control", "no-store")]);
        cache.store("https://example.com/api", 200, &headers, "data");
        assert!(cache.is_empty());
    }

    #[test]
    fn test_no_cache_not_cached() {
        let mut cache = HttpCache::new(1);
        let headers = make_headers(&[("cache-control", "no-cache")]);
        cache.store("https://example.com/api", 200, &headers, "data");
        assert!(cache.is_empty());
    }

    #[test]
    fn test_set_cookie_not_cached() {
        let mut cache = HttpCache::new(1);
        let headers = make_headers(&[
            ("cache-control", "max-age=3600"),
            ("set-cookie", "session=abc"),
        ]);
        cache.store("https://example.com/login", 200, &headers, "ok");
        assert!(cache.is_empty());
    }

    #[test]
    fn test_non_200_not_cached() {
        let mut cache = HttpCache::new(1);
        let headers = make_headers(&[("cache-control", "max-age=3600")]);
        cache.store("https://example.com/404", 404, &headers, "not found");
        assert!(cache.is_empty());
    }

    #[test]
    fn test_conditional_headers() {
        let mut cache = HttpCache::new(1);
        let headers = make_headers(&[
            ("cache-control", "max-age=3600"),
            ("etag", "\"abc123\""),
            ("last-modified", "Wed, 21 Oct 2025 07:28:00 GMT"),
        ]);
        cache.store("https://example.com/page", 200, &headers, "content");

        let cond = cache.conditional_headers("https://example.com/page");
        assert_eq!(cond.get("If-None-Match").unwrap(), "\"abc123\"");
        assert_eq!(cond.get("If-Modified-Since").unwrap(), "Wed, 21 Oct 2025 07:28:00 GMT");
    }

    #[test]
    fn test_conditional_headers_empty_for_unknown() {
        let cache = HttpCache::new(1);
        let cond = cache.conditional_headers("https://example.com/unknown");
        assert!(cond.is_empty());
    }

    #[test]
    fn test_eviction_by_size() {
        // 1 byte max — forces eviction on every store
        let mut cache = HttpCache {
            entries: HashMap::new(),
            max_size: 100,
            current_size: 0,
        };
        let headers = make_headers(&[("cache-control", "max-age=3600")]);

        cache.store("https://a.com/1", 200, &headers, &"x".repeat(60));
        assert_eq!(cache.len(), 1);

        cache.store("https://a.com/2", 200, &headers, &"y".repeat(60));
        // Should have evicted the first to fit the second
        assert_eq!(cache.len(), 1);
        assert!(cache.get("https://a.com/2").is_some());
    }

    #[test]
    fn test_clear() {
        let mut cache = HttpCache::new(1);
        let headers = make_headers(&[("cache-control", "max-age=3600")]);
        cache.store("https://example.com/a", 200, &headers, "a");
        cache.store("https://example.com/b", 200, &headers, "b");
        assert_eq!(cache.len(), 2);

        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(cache.size(), 0);
    }

    #[test]
    fn test_miss_returns_none() {
        let mut cache = HttpCache::new(1);
        assert!(cache.get("https://example.com/nothing").is_none());
    }

    #[test]
    fn test_private_not_cached() {
        let mut cache = HttpCache::new(1);
        let headers = make_headers(&[("cache-control", "private, max-age=3600")]);
        cache.store("https://example.com/private", 200, &headers, "secret");
        assert!(cache.is_empty());
    }

    #[test]
    fn test_touch_refreshes_entry() {
        let mut cache = HttpCache::new(1);
        let headers = make_headers(&[("cache-control", "max-age=3600")]);
        cache.store("https://example.com/x", 200, &headers, "data");
        cache.touch("https://example.com/x");
        assert!(cache.get("https://example.com/x").is_some());
    }
}
