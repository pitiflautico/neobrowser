//! neo-http — HTTP layer for NeoRender AI browser engine.
//!
//! Handles all network requests with Chrome 136 TLS fingerprint,
//! URL classification, telemetry blocking, cookie management, and disk caching.

pub mod cache;
pub mod classify;
pub mod classify_request;
pub mod client;
pub mod cookies;
pub mod headers;
pub mod mock;
pub mod storage;

use neo_types::HttpResponse;
use std::collections::HashMap;

/// Errors that can occur in the HTTP layer.
#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    /// Network-level failure (DNS, TLS, connection refused, timeout).
    #[error("network error: {0}")]
    Network(String),
    /// Request was skipped because it matched a telemetry/analytics pattern.
    #[error("request skipped: {url}")]
    Skipped { url: String },
    /// Cookie store I/O error.
    #[error("cookie store error: {0}")]
    CookieStore(String),
    /// Cache I/O error.
    #[error("cache error: {0}")]
    Cache(String),
    /// URL parsing failed.
    #[error("invalid URL: {0}")]
    InvalidUrl(String),
    /// Response body decoding error.
    #[error("decode error: {0}")]
    Decode(String),
}

/// Classification of an HTTP request by purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestKind {
    Navigation,
    Subresource,
    Fetch,
    FormSubmit,
    Telemetry,
    Media,
    Api,
}

/// Context about who initiated a request and why.
#[derive(Debug, Clone)]
pub struct RequestContext {
    pub kind: RequestKind,
    pub initiator: String,
    pub referrer: Option<String>,
    pub frame_id: Option<String>,
    pub top_level_url: Option<String>,
}

/// A fully-specified HTTP request.
#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: HashMap<String, String>,
    pub body: Option<String>,
    pub context: RequestContext,
    pub timeout_ms: u64,
}

/// Result of a cache lookup.
#[derive(Debug, Clone)]
pub enum CacheDecision {
    Fresh(HttpResponse),
    Stale {
        response: HttpResponse,
        etag: Option<String>,
        last_modified: Option<String>,
    },
    Miss,
}

/// Trait for sending HTTP requests.
pub trait HttpClient: Send + Sync {
    fn request(&self, req: &HttpRequest) -> Result<HttpResponse, HttpError>;
}

/// Trait for cookie storage with SameSite awareness.
pub trait CookieStore: Send + Sync {
    fn get_for_request(&self, url: &str, top_level_url: Option<&str>, is_top_level: bool)
        -> String;
    fn store_set_cookie(&self, url: &str, set_cookie: &str);
    fn delete(&self, name: &str, domain: &str, path: &str);
    fn evict_expired(&self);
    fn clear_session(&self);
    fn list_for_domain(&self, domain: &str) -> Vec<neo_types::Cookie>;
    fn export(&self) -> Vec<neo_types::Cookie>;
    fn import(&self, cookies: &[neo_types::Cookie]);
    fn snapshot(&self) -> Vec<neo_types::Cookie>;
}

/// Web storage (localStorage, sessionStorage).
pub trait WebStorage: Send + Sync {
    fn get(&self, origin: &str, key: &str) -> Option<String>;
    fn set(&self, origin: &str, key: &str, value: &str);
    fn remove(&self, origin: &str, key: &str);
    fn clear(&self, origin: &str);
    fn keys(&self, origin: &str) -> Vec<String>;
    fn len(&self, origin: &str) -> usize;
}

/// Trait for HTTP response caching.
pub trait HttpCache: Send + Sync {
    fn lookup(&self, req: &HttpRequest) -> CacheDecision;
    fn store(&self, req: &HttpRequest, response: &HttpResponse);
    fn invalidate(&self, pattern: &str);
    fn is_fresh(&self, url: &str) -> bool;
}

// Re-exports for convenience.
pub use cache::DiskCache;
pub use classify::{classify_url, should_skip};
pub use classify_request::{classify_request, ClassificationOverrides, RequestCategory};
pub use client::RquestClient;
pub use cookies::SqliteCookieStore;
pub use mock::{InMemoryCookieStore, InMemoryWebStorage, MockHttpClient, NoopCache};
pub use storage::SqliteWebStorage;
