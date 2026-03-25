//! Chrome CDP fallback for op_fetch — retries Cloudflare-blocked requests
//! through a real Chrome instance.
//!
//! When wreq gets a 403 with Cloudflare markers, the request is retried
//! through Chrome's native fetch() which has a matching TLS fingerprint.

use neo_chrome::fetch_proxy::{ChromeFetchProxy, ChromeFetchResult};
use std::collections::HashMap;
use std::sync::Arc;

/// Shared Chrome fallback stored in OpState.
///
/// Lazily initializes a ChromeFetchProxy on first use. Thread-safe via
/// tokio::sync::Mutex (async-aware, won't block the executor).
#[derive(Clone)]
pub struct SharedChromeFallback {
    proxy: Arc<tokio::sync::Mutex<Option<ChromeFetchProxy>>>,
}

impl SharedChromeFallback {
    /// Create a new (uninitialized) fallback. Chrome won't launch until
    /// the first Cloudflare-blocked request.
    pub fn new() -> Self {
        Self {
            proxy: Arc::new(tokio::sync::Mutex::new(None)),
        }
    }

    /// Execute a fetch through Chrome, lazily launching Chrome if needed.
    ///
    /// Initializes ChromeFetchProxy on first call. Injects cookies for
    /// the target domain if a cookie store is provided.
    pub async fn fetch_via_chrome(
        &self,
        url: &str,
        method: &str,
        body: Option<&str>,
        headers: &HashMap<String, String>,
        cookies: Option<&[(String, String, String)]>,
    ) -> Result<ChromeFetchResult, String> {
        let mut guard = self.proxy.lock().await;

        // Lazy init: launch Chrome on first use.
        if guard.is_none() {
            eprintln!("[neo-chrome-fallback] Launching Chrome for Cloudflare bypass...");
            let proxy = ChromeFetchProxy::new()
                .await
                .map_err(|e| format!("Chrome launch failed: {e}"))?;
            *guard = Some(proxy);
            eprintln!("[neo-chrome-fallback] Chrome ready.");
        }

        let proxy = guard.as_mut().unwrap();

        // Inject cookies for first-time domains (Chrome will find its own tab
        // or create one via ensure_tab_for_domain).
        if let Some(cookie_tuples) = cookies {
            if let Some(domain) = extract_domain(url) {
                if !proxy.is_domain_injected(&domain) {
                    let domain_cookies: Vec<_> = cookie_tuples
                        .iter()
                        .filter(|(_, _, d)| {
                            d == &domain
                                || d.ends_with(&format!(".{}", domain))
                                || domain.ends_with(d.trim_start_matches('.'))
                        })
                        .cloned()
                        .collect();
                    if !domain_cookies.is_empty() {
                        let _ = proxy.inject_cookies(&domain_cookies).await;
                    }
                    proxy.mark_domain_injected(&domain);
                }
            }
        }

        eprintln!(
            "[neo-chrome-fallback] Fetching via Chrome: {method} {url}"
        );
        let result = proxy
            .fetch(url, method, body, headers)
            .await
            .map_err(|e| format!("Chrome fetch failed: {e}"))?;

        eprintln!(
            "[neo-chrome-fallback] Chrome response: {} ({} bytes)",
            result.status,
            result.body.len()
        );
        Ok(result)
    }
}

/// Check if an HTTP response body contains Cloudflare block markers.
pub fn is_cloudflare_block(body: &str) -> bool {
    body.contains("Unusual activity")
        || body.contains("cf-error-details")
        || body.contains("Just a moment")
        || body.contains("cf-challenge-running")
        || body.contains("Checking if the site connection is secure")
}

/// Extract the domain from a URL string.
fn extract_domain(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
}

/// Extract the origin (scheme + host) from a URL.
fn extract_origin(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(u) => format!("{}://{}", u.scheme(), u.host_str().unwrap_or("")),
        Err(_) => url.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_cloudflare_block() {
        assert!(is_cloudflare_block("Unusual activity detected"));
        assert!(is_cloudflare_block("<div class=\"cf-error-details\">"));
        assert!(is_cloudflare_block("Just a moment..."));
        assert!(is_cloudflare_block("cf-challenge-running"));
        assert!(is_cloudflare_block("Checking if the site connection is secure"));
        assert!(!is_cloudflare_block("Hello, world!"));
        assert!(!is_cloudflare_block(r#"{"status":"ok"}"#));
    }

    #[test]
    fn test_extract_domain() {
        assert_eq!(
            extract_domain("https://chatgpt.com/api/foo"),
            Some("chatgpt.com".into())
        );
        assert_eq!(
            extract_domain("https://sub.example.com:8080/path"),
            Some("sub.example.com".into())
        );
        assert_eq!(extract_domain("not-a-url"), None);
    }

    #[test]
    fn test_shared_chrome_fallback_new() {
        // Just verify it can be created without panicking.
        let _fallback = SharedChromeFallback::new();
    }
}
