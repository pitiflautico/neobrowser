//! neo-http — HTTP layer for NeoRender AI browser engine.
//!
//! Handles all network requests with Chrome 136 TLS fingerprint,
//! URL classification, telemetry blocking, cookie management, and disk caching.

pub mod cache;
pub mod classify;
pub mod client;
pub mod cookies;
pub mod headers;
pub mod mock;

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
    /// Top-level page navigation.
    Navigation,
    /// Sub-resource loaded by the page (CSS, JS, images).
    Subresource,
    /// XHR/fetch API call.
    Fetch,
    /// Form submission.
    FormSubmit,
    /// Telemetry, analytics, or tracking beacon.
    Telemetry,
    /// Media content (video, audio, streaming).
    Media,
    /// REST/GraphQL API endpoint.
    Api,
}

/// Context about who initiated a request and why.
#[derive(Debug, Clone)]
pub struct RequestContext {
    /// What kind of request this is.
    pub kind: RequestKind,
    /// Who started the request (e.g. "parser", "script", "user").
    pub initiator: String,
    /// Referrer URL, if any.
    pub referrer: Option<String>,
    /// Frame ID within the page.
    pub frame_id: Option<String>,
    /// The top-level document URL (for SameSite cookie decisions).
    pub top_level_url: Option<String>,
}

/// A fully-specified HTTP request.
#[derive(Debug, Clone)]
pub struct HttpRequest {
    /// HTTP method (GET, POST, etc.).
    pub method: String,
    /// Target URL.
    pub url: String,
    /// Request headers.
    pub headers: HashMap<String, String>,
    /// Optional request body.
    pub body: Option<String>,
    /// Request context with classification metadata.
    pub context: RequestContext,
    /// Timeout in milliseconds.
    pub timeout_ms: u64,
}

/// Result of a cache lookup.
#[derive(Debug, Clone)]
pub enum CacheDecision {
    /// Cached response is still fresh; use it directly.
    Fresh(HttpResponse),
    /// Cached response is stale; revalidate with server.
    Stale {
        /// The stale cached response.
        response: HttpResponse,
        /// ETag for conditional request.
        etag: Option<String>,
        /// Last-Modified for conditional request.
        last_modified: Option<String>,
    },
    /// No cache entry found.
    Miss,
}

/// Trait for sending HTTP requests.
pub trait HttpClient: Send + Sync {
    /// Send an HTTP request and return the response.
    fn request(&self, req: &HttpRequest) -> Result<HttpResponse, HttpError>;
}

/// Trait for cookie storage with SameSite awareness.
pub trait CookieStore: Send + Sync {
    /// Get the Cookie header value for a request.
    fn get_for_request(&self, url: &str, top_level_url: Option<&str>, is_top_level: bool)
        -> String;
    /// Store a Set-Cookie header from a response.
    fn store_set_cookie(&self, url: &str, set_cookie: &str);
    /// Delete a specific cookie.
    fn delete(&self, name: &str, domain: &str, path: &str);
    /// Remove all expired cookies.
    fn evict_expired(&self);
    /// Remove all session cookies (those without an expiry).
    fn clear_session(&self);
    /// List all cookies for a domain.
    fn list_for_domain(&self, domain: &str) -> Vec<neo_types::Cookie>;
    /// Export all cookies.
    fn export(&self) -> Vec<neo_types::Cookie>;
    /// Import cookies.
    fn import(&self, cookies: &[neo_types::Cookie]);
    /// Snapshot all cookies (alias for export).
    fn snapshot(&self) -> Vec<neo_types::Cookie>;
}

/// Trait for HTTP response caching.
pub trait HttpCache: Send + Sync {
    /// Look up a cached response for a request.
    fn lookup(&self, req: &HttpRequest) -> CacheDecision;
    /// Store a response in the cache.
    fn store(&self, req: &HttpRequest, response: &HttpResponse);
    /// Invalidate cached entries matching a URL pattern.
    fn invalidate(&self, pattern: &str);
    /// Check if a cached entry for the URL is still fresh.
    fn is_fresh(&self, url: &str) -> bool;
}

// Re-exports for convenience.
pub use cache::DiskCache;
pub use classify::{classify_url, should_skip};
pub use client::RquestClient;
pub use cookies::SqliteCookieStore;
pub use mock::{InMemoryCookieStore, MockHttpClient, NoopCache};
