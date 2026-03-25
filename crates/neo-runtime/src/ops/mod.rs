//! V8 operations — bridge between JavaScript and Rust.
//!
//! Fetch ops use async I/O on the existing tokio runtime (Chromium-style:
//! single event loop, shared connection pool, HTTP/2 multiplexing).
//! Timers use thread::sleep.

pub mod console;
pub mod cookies;
pub mod fetch;
pub mod headers;
pub mod misc;
pub mod navigation;
pub mod storage;
pub mod timers;
pub mod url_filter;

// Re-export all ops so the extension! macro can reference them as ops::op_*.
pub use console::*;
pub use cookies::*;
pub use fetch::*;
pub use misc::*;
pub use navigation::op_navigation_request;
pub use storage::*;
pub use timers::*;

use neo_http::{CookieStore, HttpClient, WebStorage};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

// ─── Shared state structs (stored in OpState) ───

/// Shared HTTP client stored in OpState for fetch ops.
pub struct SharedHttpClient(pub Arc<dyn HttpClient>);

/// Shared cookie store for auto-injecting cookies into fetch requests.
///
/// Wraps `Option<Arc<dyn CookieStore>>` so it can be absent (no cookie store attached).
pub struct SharedCookieStore(pub Option<Arc<dyn CookieStore>>);

/// Console log buffer — captures JS console output.
#[derive(Default, Clone)]
pub struct ConsoleBuffer {
    /// Captured log messages.
    pub messages: Arc<std::sync::Mutex<Vec<String>>>,
}

/// Web storage state: wraps a `WebStorage` trait object + current origin.
///
/// Falls back to an in-memory HashMap when no `WebStorage` backend is provided
/// (preserves backward compatibility with code that used `StorageState::default()`).
#[derive(Clone)]
pub struct StorageState {
    /// Backend (SQLite, in-memory mock, etc.).
    pub backend: Arc<dyn WebStorage>,
    /// Current storage origin (set on navigation, e.g. "https://example.com").
    pub origin: String,
}

impl Default for StorageState {
    fn default() -> Self {
        Self {
            backend: Arc::new(neo_http::InMemoryWebStorage::new()),
            origin: String::new(),
        }
    }
}

/// Shared scheduler config values accessible from ops.
#[derive(Clone)]
pub struct OpsSchedulerConfig {
    /// Max ticks per setInterval (exposed to JS).
    pub interval_max_ticks: u32,
}

impl Default for OpsSchedulerConfig {
    fn default() -> Self {
        Self {
            interval_max_ticks: 20,
        }
    }
}

// ─── Streaming Fetch (G2) ───

/// Store for active streaming HTTP responses.
///
/// Keeps `wreq::Response` objects alive between `op_fetch_start` and
/// subsequent `op_fetch_read_chunk` calls. Each stream gets a unique u32 id.
pub struct StreamStore {
    pub(crate) streams: HashMap<u32, ActiveStream>,
    pub(crate) next_id: u32,
}

pub(crate) struct ActiveStream {
    pub(crate) response: Option<wreq::Response>,
    /// Tracked for future TTL-based cleanup of abandoned streams.
    #[allow(dead_code)]
    pub(crate) created_at: std::time::Instant,
    /// Pre-filled body from Chrome fallback (single-chunk stream).
    pub(crate) prefilled_body: Option<String>,
}

impl Default for StreamStore {
    fn default() -> Self {
        Self {
            streams: HashMap::new(),
            next_id: 1,
        }
    }
}

impl StreamStore {
    /// Insert a new stream, returns its unique id.
    pub fn insert(&mut self, response: wreq::Response) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.streams.insert(id, ActiveStream {
            response: Some(response),
            created_at: std::time::Instant::now(),
            prefilled_body: None,
        });
        id
    }

    /// Take the response out of a stream (for reading).
    pub fn take_response(&mut self, id: u32) -> Option<wreq::Response> {
        self.streams.get_mut(&id).and_then(|s| s.response.take())
    }

    /// Put a response back after reading a chunk.
    pub fn put_response(&mut self, id: u32, response: wreq::Response) {
        if let Some(stream) = self.streams.get_mut(&id) {
            stream.response = Some(response);
        }
    }

    /// Remove a stream entirely.
    pub fn remove(&mut self, id: u32) {
        self.streams.remove(&id);
    }

    /// Number of active streams.
    pub fn len(&self) -> usize {
        self.streams.len()
    }

    /// Whether there are no active streams.
    pub fn is_empty(&self) -> bool {
        self.streams.is_empty()
    }
}

/// Shared raw wreq client for streaming fetch ops.
///
/// Stored separately from `SharedHttpClient` because streaming needs the raw
/// `wreq::Client` to get an `wreq::Response` without reading the body.
pub struct SharedRquestClient(pub Arc<wreq::Client>);

/// Shared tokio runtime for fetch ops — Chromium-style single network thread.
pub struct SharedFetchRuntime(pub Arc<tokio::runtime::Runtime>);

/// Shared impit client for Cloudflare TLS bypass.
///
/// Used as a lightweight fallback before launching Chrome. impit uses
/// patched rustls with Chrome 142 fingerprint to pass JA3/JA4 checks.
#[derive(Clone)]
pub struct SharedImpitClient(pub Arc<neo_http::ImpitClient>);

// ─── Browser Shim Ops ───

/// Queue of navigation requests from JS (form.submit, location.href, window.open).
///
/// The engine drains this queue after every interaction to handle
/// client-side navigation attempts.
#[derive(Default, Clone)]
pub struct NavigationQueue {
    requests: Arc<Mutex<Vec<String>>>,
}

impl NavigationQueue {
    /// Push a new navigation request (called from JS via op_navigation_request).
    pub fn push(&self, req: String) {
        if let Ok(mut q) = self.requests.lock() {
            q.push(req);
        }
    }

    /// Drain all pending navigation requests. Returns empty vec if none.
    pub fn drain(&self) -> Vec<String> {
        if let Ok(mut q) = self.requests.lock() {
            q.drain(..).collect()
        } else {
            vec![]
        }
    }

    /// Check if there are pending navigation requests.
    pub fn has_pending(&self) -> bool {
        if let Ok(q) = self.requests.lock() {
            !q.is_empty()
        } else {
            false
        }
    }
}

/// Cookie state for `document.cookie` access, backed by a simple in-process store.
///
/// Cookies are stored per-origin. The origin is set when the page navigates.
#[derive(Clone)]
pub struct CookieState {
    cookies: Arc<Mutex<HashMap<String, String>>>,
    origin: String,
}

impl CookieState {
    /// Create a new cookie state for the given origin.
    pub fn new(origin: &str) -> Self {
        Self {
            cookies: Arc::new(Mutex::new(HashMap::new())),
            origin: origin.to_string(),
        }
    }

    /// Get the cookie string for `document.cookie` getter.
    pub fn get_cookie_string(&self) -> String {
        if let Ok(cookies) = self.cookies.lock() {
            cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; ")
        } else {
            String::new()
        }
    }

    /// Parse and store a `Set-Cookie`-style string from `document.cookie` setter.
    ///
    /// Only the name=value part is stored; attributes (Path, Domain, etc.)
    /// are ignored since we operate in a single-origin context.
    pub fn set_from_string(&self, cookie_str: &str) {
        let name_value = cookie_str.split(';').next().unwrap_or("");
        if let Some((name, value)) = name_value.split_once('=') {
            let name = name.trim().to_string();
            let value = value.trim().to_string();
            if !name.is_empty() {
                if let Ok(mut cookies) = self.cookies.lock() {
                    cookies.insert(name, value);
                }
            }
        }
    }

    /// Set the origin (called on navigation).
    pub fn set_origin(&mut self, origin: &str) {
        self.origin = origin.to_string();
    }

    /// Get the current origin.
    pub fn origin(&self) -> &str {
        &self.origin
    }
}

impl Default for CookieState {
    fn default() -> Self {
        Self {
            cookies: Arc::new(Mutex::new(HashMap::new())),
            origin: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── StreamStore tests ───

    #[test]
    fn stream_store_default_starts_at_1() {
        let store = StreamStore::default();
        assert_eq!(store.next_id, 1);
        assert!(store.is_empty());
    }

    #[test]
    fn stream_store_remove_nonexistent() {
        let mut store = StreamStore::default();
        store.remove(999); // should not panic
        assert!(store.is_empty());
    }

    #[test]
    fn stream_store_len() {
        let mut store = StreamStore::default();
        assert_eq!(store.len(), 0);
        // Insert a prefilled stream (no real response needed)
        let id = store.next_id;
        store.next_id += 1;
        store.streams.insert(id, ActiveStream {
            response: None,
            created_at: std::time::Instant::now(),
            prefilled_body: Some("test".to_string()),
        });
        assert_eq!(store.len(), 1);
        store.remove(id);
        assert_eq!(store.len(), 0);
    }

    // ─── NavigationQueue tests ───

    #[test]
    fn navigation_queue_push_and_drain() {
        let q = NavigationQueue::default();
        assert!(!q.has_pending());
        q.push("nav1".to_string());
        q.push("nav2".to_string());
        assert!(q.has_pending());
        let drained = q.drain();
        assert_eq!(drained, vec!["nav1", "nav2"]);
        assert!(!q.has_pending());
    }

    #[test]
    fn navigation_queue_drain_empty() {
        let q = NavigationQueue::default();
        let drained = q.drain();
        assert!(drained.is_empty());
    }

    // ─── CookieState tests ───

    #[test]
    fn cookie_state_set_and_get() {
        let cs = CookieState::new("https://example.com");
        cs.set_from_string("foo=bar; Path=/; Domain=example.com");
        let s = cs.get_cookie_string();
        assert!(s.contains("foo=bar"));
    }

    #[test]
    fn cookie_state_multiple_cookies() {
        let cs = CookieState::new("https://example.com");
        cs.set_from_string("a=1");
        cs.set_from_string("b=2");
        let s = cs.get_cookie_string();
        assert!(s.contains("a=1"));
        assert!(s.contains("b=2"));
    }

    #[test]
    fn cookie_state_overwrite() {
        let cs = CookieState::new("https://example.com");
        cs.set_from_string("foo=old");
        cs.set_from_string("foo=new");
        assert_eq!(cs.get_cookie_string(), "foo=new");
    }

    #[test]
    fn cookie_state_empty_name_ignored() {
        let cs = CookieState::new("https://example.com");
        cs.set_from_string("=value");
        assert!(cs.get_cookie_string().is_empty());
    }

    #[test]
    fn cookie_state_origin() {
        let mut cs = CookieState::new("https://a.com");
        assert_eq!(cs.origin(), "https://a.com");
        cs.set_origin("https://b.com");
        assert_eq!(cs.origin(), "https://b.com");
    }
}
