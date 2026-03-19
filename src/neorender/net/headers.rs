//! Sec-Fetch-* header generation per the Fetch Metadata Request Headers spec.
//! https://w3c.github.io/webappsec-fetch-metadata/

use super::{RequestMode, RequestDestination};

/// Determine the Sec-Fetch-Site value by comparing request origin to page origin.
pub fn sec_fetch_site(request_origin: &str, page_origin: &str) -> &'static str {
    if request_origin == page_origin {
        "same-origin"
    } else if same_site(request_origin, page_origin) {
        "same-site"
    } else {
        "cross-site"
    }
}

/// Map RequestMode to Sec-Fetch-Mode header value.
pub fn sec_fetch_mode(mode: &RequestMode) -> &'static str {
    match mode {
        RequestMode::Cors => "cors",
        RequestMode::Navigate => "navigate",
        RequestMode::NoCors => "no-cors",
        RequestMode::SameOrigin => "same-origin",
    }
}

/// Map RequestDestination to Sec-Fetch-Dest header value.
pub fn sec_fetch_dest(dest: &RequestDestination) -> &'static str {
    match dest {
        RequestDestination::Empty => "empty",
        RequestDestination::Document => "document",
        RequestDestination::Script => "script",
        RequestDestination::Style => "style",
        RequestDestination::Image => "image",
    }
}

/// Check if two origins are same-site (share the same registrable domain).
/// Simplified: compares the last 2 dot-separated parts of each hostname.
/// e.g. "https://api.example.com" and "https://www.example.com" are same-site.
fn same_site(a: &str, b: &str) -> bool {
    let reg_a = registrable_domain(a);
    let reg_b = registrable_domain(b);
    match (reg_a, reg_b) {
        (Some(da), Some(db)) => da == db,
        _ => false,
    }
}

/// Extract registrable domain from an origin string.
/// Simplified: takes the last 2 dot-separated parts of the hostname.
fn registrable_domain(origin: &str) -> Option<String> {
    let host = origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("http://"))
        .unwrap_or(origin);
    // Strip port if present
    let host = host.split(':').next().unwrap_or(host);
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() >= 2 {
        Some(format!("{}.{}", parts[parts.len() - 2], parts[parts.len() - 1]))
    } else {
        Some(host.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_same_origin() {
        assert_eq!(sec_fetch_site("https://example.com", "https://example.com"), "same-origin");
    }

    #[test]
    fn test_same_site() {
        assert_eq!(sec_fetch_site("https://api.example.com", "https://www.example.com"), "same-site");
    }

    #[test]
    fn test_cross_site() {
        assert_eq!(sec_fetch_site("https://evil.com", "https://example.com"), "cross-site");
    }

    #[test]
    fn test_sec_fetch_mode() {
        assert_eq!(sec_fetch_mode(&RequestMode::Cors), "cors");
        assert_eq!(sec_fetch_mode(&RequestMode::Navigate), "navigate");
    }

    #[test]
    fn test_sec_fetch_dest() {
        assert_eq!(sec_fetch_dest(&RequestDestination::Empty), "empty");
        assert_eq!(sec_fetch_dest(&RequestDestination::Document), "document");
    }
}
