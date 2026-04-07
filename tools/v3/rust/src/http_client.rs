//! Centralized HTTP client factory — single place for TLS fingerprint config.
//!
//! All HTTP clients in neobrowser should come from here. This ensures:
//! - Consistent Chrome 136 TLS fingerprint across all requests
//! - Proper browser headers for stealth
//! - Single place to update when Chrome version changes
//! - No scattered Client::builder() calls

use rquest::header::{
    HeaderMap, HeaderValue, ACCEPT, ACCEPT_ENCODING, ACCEPT_LANGUAGE, CACHE_CONTROL, DNT,
    UPGRADE_INSECURE_REQUESTS,
};
use std::time::Duration;

/// Chrome 136 client with full browser emulation.
/// - Chrome 136 TLS fingerprint (BoringSSL)
/// - cookie_store(false) — cookies handled externally via UnifiedCookieJar
/// - Follows up to 10 redirects
/// - 30s timeout
/// - Full Chrome browser headers
pub fn chrome136() -> Result<rquest::Client, rquest::Error> {
    rquest::Client::builder()
        .emulation(rquest_util::Emulation::Chrome136)
        .cookie_store(false)
        .redirect(rquest::redirect::Policy::limited(10))
        .timeout(Duration::from_secs(30))
        .default_headers(chrome136_headers())
        .build()
}

/// Chrome 136 client with rquest's internal cookie store enabled.
/// Used where cookies need to be handled per-redirect chain (e.g. GhostBrowser, neorender).
pub fn chrome136_with_cookies() -> Result<rquest::Client, rquest::Error> {
    rquest::Client::builder()
        .emulation(rquest_util::Emulation::Chrome136)
        .cookie_store(true)
        .redirect(rquest::redirect::Policy::limited(10))
        .timeout(Duration::from_secs(30))
        .default_headers(chrome136_headers())
        .build()
}

/// Lightweight client for local HTTP (CDP discovery, CORS proxy).
/// No TLS fingerprint needed — these are localhost requests.
pub fn local(timeout_secs: u64) -> Result<rquest::Client, rquest::Error> {
    rquest::Client::builder()
        .redirect(rquest::redirect::Policy::limited(10))
        .timeout(Duration::from_secs(timeout_secs))
        .build()
}

/// Light-mode client for quick HTML fetches (no cookie jar, no emulation).
/// Uses Chrome UA string + compression + basic Accept headers.
pub fn light() -> Result<rquest::Client, rquest::Error> {
    let mut headers = HeaderMap::new();
    headers.insert(
        ACCEPT,
        HeaderValue::from_static(
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        ),
    );
    headers.insert(
        ACCEPT_ENCODING,
        HeaderValue::from_static("gzip, deflate, br"),
    );

    rquest::Client::builder()
        .user_agent(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
             AppleWebKit/537.36 (KHTML, like Gecko) \
             Chrome/136.0.0.0 Safari/537.36",
        )
        .default_headers(headers)
        .cookie_store(false)
        .gzip(true)
        .brotli(true)
        .redirect(rquest::redirect::Policy::limited(5))
        .build()
}

/// Chrome 136 client for quick fetches (module loader, sync fetch in V8).
/// Short timeout to avoid blocking.
pub fn chrome136_quick(timeout_secs: u64) -> Result<rquest::Client, rquest::Error> {
    rquest::Client::builder()
        .emulation(rquest_util::Emulation::Chrome136)
        .cookie_store(false)
        .timeout(Duration::from_secs(timeout_secs))
        .default_headers(chrome136_headers())
        .build()
}

fn chrome136_headers() -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert(
        ACCEPT,
        HeaderValue::from_static(
            "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8",
        ),
    );
    h.insert(
        ACCEPT_LANGUAGE,
        HeaderValue::from_static("es-ES,es;q=0.9,en;q=0.8"),
    );
    h.insert(
        ACCEPT_ENCODING,
        HeaderValue::from_static("gzip, deflate, br, zstd"),
    );
    h.insert(DNT, HeaderValue::from_static("1"));
    h.insert(UPGRADE_INSECURE_REQUESTS, HeaderValue::from_static("1"));
    h.insert(CACHE_CONTROL, HeaderValue::from_static("max-age=0"));
    h
}
