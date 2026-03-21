//! Chrome 136 header sets for navigation and fetch requests.

use std::collections::HashMap;

/// Chrome 136 User-Agent string (macOS).
pub const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
     AppleWebKit/537.36 (KHTML, like Gecko) \
     Chrome/136.0.0.0 Safari/537.36";

/// Full navigation headers matching Chrome 136 on macOS.
///
/// Includes all Sec-Ch-Ua, Sec-Fetch, Accept, and encoding headers
/// that a real Chrome browser sends on a top-level navigation.
pub fn navigation_headers() -> HashMap<String, String> {
    let mut h = HashMap::new();
    h.insert("User-Agent".into(), USER_AGENT.into());
    h.insert(
        "Accept".into(),
        "text/html,application/xhtml+xml,application/xml;\
         q=0.9,image/avif,image/webp,image/apng,*/*;\
         q=0.8,application/signed-exchange;v=b3;q=0.7"
            .into(),
    );
    h.insert("Accept-Language".into(), "en-US,en;q=0.9".into());
    h.insert("Accept-Encoding".into(), "gzip, deflate, br, zstd".into());
    h.insert("Cache-Control".into(), "max-age=0".into());
    h.insert(
        "Sec-Ch-Ua".into(),
        r#""Chromium";v="136", "Google Chrome";v="136", "Not.A/Brand";v="99""#.into(),
    );
    h.insert("Sec-Ch-Ua-Mobile".into(), "?0".into());
    h.insert("Sec-Ch-Ua-Platform".into(), r#""macOS""#.into());
    h.insert("Sec-Fetch-Dest".into(), "document".into());
    h.insert("Sec-Fetch-Mode".into(), "navigate".into());
    h.insert("Sec-Fetch-Site".into(), "none".into());
    h.insert("Sec-Fetch-User".into(), "?1".into());
    h.insert("Upgrade-Insecure-Requests".into(), "1".into());
    h.insert("Connection".into(), "keep-alive".into());
    h
}

/// Lighter fetch/XHR headers matching Chrome 136.
///
/// Used for API calls, sub-resources, and AJAX requests.
/// Omits navigation-specific headers like Upgrade-Insecure-Requests.
pub fn fetch_headers() -> HashMap<String, String> {
    let mut h = HashMap::new();
    h.insert("User-Agent".into(), USER_AGENT.into());
    h.insert("Accept".into(), "*/*".into());
    h.insert("Accept-Language".into(), "en-US,en;q=0.9".into());
    h.insert("Accept-Encoding".into(), "gzip, deflate, br, zstd".into());
    h.insert(
        "Sec-Ch-Ua".into(),
        r#""Chromium";v="136", "Google Chrome";v="136", "Not.A/Brand";v="99""#.into(),
    );
    h.insert("Sec-Ch-Ua-Mobile".into(), "?0".into());
    h.insert("Sec-Ch-Ua-Platform".into(), r#""macOS""#.into());
    h.insert("Sec-Fetch-Dest".into(), "empty".into());
    h.insert("Sec-Fetch-Mode".into(), "cors".into());
    h.insert("Sec-Fetch-Site".into(), "same-origin".into());
    h.insert("Connection".into(), "keep-alive".into());
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_navigation_headers_complete() {
        let h = navigation_headers();
        assert!(h.contains_key("User-Agent"));
        assert!(h.contains_key("Sec-Ch-Ua"));
        assert!(h.contains_key("Sec-Fetch-Dest"));
        assert_eq!(h["Sec-Fetch-Dest"], "document");
        assert_eq!(h["Sec-Fetch-Mode"], "navigate");
    }

    #[test]
    fn test_fetch_headers_lighter() {
        let h = fetch_headers();
        assert_eq!(h["Sec-Fetch-Dest"], "empty");
        assert_eq!(h["Sec-Fetch-Mode"], "cors");
        assert!(!h.contains_key("Upgrade-Insecure-Requests"));
    }
}
