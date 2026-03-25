//! Chrome header sets for navigation and fetch requests.
//! Must match the TLS emulation version in client.rs (wreq Chrome 145).
//! Captured from Chrome 146 real on macOS via httpbin.org/anything.

use std::collections::HashMap;

/// Chrome User-Agent string (macOS) — matches TLS emulation.
pub const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
     AppleWebKit/537.36 (KHTML, like Gecko) \
     Chrome/145.0.0.0 Safari/537.36";

/// Sec-Ch-Ua value — must match User-Agent version.
/// Format captured from Chrome 146; adapted to our emulated version.
/// Chrome 146 uses: "Chromium";v="146", "Not-A.Brand";v="24", "Google Chrome";v="146"
const SEC_CH_UA: &str =
    r#""Chromium";v="145", "Not-A.Brand";v="24", "Google Chrome";v="145""#;

/// Full navigation headers matching Chrome on macOS.
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
    h.insert("Sec-Ch-Ua".into(), SEC_CH_UA.into());
    h.insert("Sec-Ch-Ua-Mobile".into(), "?0".into());
    h.insert("Sec-Ch-Ua-Platform".into(), r#""macOS""#.into());
    h.insert("Sec-Fetch-Dest".into(), "document".into());
    h.insert("Sec-Fetch-Mode".into(), "navigate".into());
    h.insert("Sec-Fetch-Site".into(), "none".into());
    h.insert("Sec-Fetch-User".into(), "?1".into());
    h.insert("Upgrade-Insecure-Requests".into(), "1".into());
    h.insert("Priority".into(), "u=0, i".into());
    h
}

/// Fetch/XHR headers matching Chrome.
///
/// Used for API calls, sub-resources, and AJAX requests.
/// Includes Origin and Referer which Chrome always sends on same-origin fetch.
pub fn fetch_headers() -> HashMap<String, String> {
    let mut h = HashMap::new();
    h.insert("User-Agent".into(), USER_AGENT.into());
    h.insert("Accept".into(), "*/*".into());
    h.insert("Accept-Language".into(), "en-US,en;q=0.9".into());
    h.insert("Accept-Encoding".into(), "gzip, deflate, br, zstd".into());
    h.insert("Sec-Ch-Ua".into(), SEC_CH_UA.into());
    h.insert("Sec-Ch-Ua-Mobile".into(), "?0".into());
    h.insert("Sec-Ch-Ua-Platform".into(), r#""macOS""#.into());
    h.insert("Sec-Fetch-Dest".into(), "empty".into());
    h.insert("Sec-Fetch-Mode".into(), "cors".into());
    h.insert("Sec-Fetch-Site".into(), "same-origin".into());
    h.insert("Priority".into(), "u=1, i".into());
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
