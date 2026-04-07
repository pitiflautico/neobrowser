//! Referrer policy computation per the W3C Referrer Policy spec.
//! https://w3c.github.io/webappsec-referrer-policy/

use super::ReferrerPolicy;

/// Compute the Referer header value based on the referrer policy.
pub fn compute_referrer(
    page_url: &str,
    target_url: &str,
    policy: &ReferrerPolicy,
) -> Option<String> {
    match policy {
        ReferrerPolicy::NoReferrer => None,
        ReferrerPolicy::Origin => Some(origin_of(page_url)),
        ReferrerPolicy::SameOrigin => {
            if same_origin(page_url, target_url) {
                Some(page_url.to_string())
            } else {
                None
            }
        }
        ReferrerPolicy::StrictOriginWhenCrossOrigin => {
            if same_origin(page_url, target_url) {
                // Same origin: send full URL
                Some(page_url.to_string())
            } else if is_https(page_url) && is_https(target_url) {
                // Cross-origin HTTPS→HTTPS: send origin only
                Some(origin_of(page_url))
            } else {
                // Downgrade (HTTPS→HTTP) or non-HTTPS: no referrer
                None
            }
        }
    }
}

/// Extract origin from a URL string (scheme + host + port).
fn origin_of(url: &str) -> String {
    url::Url::parse(url)
        .map(|u| u.origin().ascii_serialization())
        .unwrap_or_default()
}

/// Check if two URLs have the same origin (scheme + host + port).
fn same_origin(a: &str, b: &str) -> bool {
    let oa = url::Url::parse(a).ok().map(|u| u.origin().ascii_serialization());
    let ob = url::Url::parse(b).ok().map(|u| u.origin().ascii_serialization());
    match (oa, ob) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// Check if a URL uses HTTPS.
fn is_https(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("https:")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_referrer() {
        assert_eq!(
            compute_referrer("https://a.com/page", "https://b.com/", &ReferrerPolicy::NoReferrer),
            None
        );
    }

    #[test]
    fn test_origin_policy() {
        assert_eq!(
            compute_referrer("https://a.com/page?q=1", "https://b.com/", &ReferrerPolicy::Origin),
            Some("https://a.com".to_string())
        );
    }

    #[test]
    fn test_same_origin_policy_same() {
        assert_eq!(
            compute_referrer("https://a.com/page", "https://a.com/other", &ReferrerPolicy::SameOrigin),
            Some("https://a.com/page".to_string())
        );
    }

    #[test]
    fn test_same_origin_policy_cross() {
        assert_eq!(
            compute_referrer("https://a.com/page", "https://b.com/other", &ReferrerPolicy::SameOrigin),
            None
        );
    }

    #[test]
    fn test_strict_same_origin() {
        assert_eq!(
            compute_referrer(
                "https://a.com/page?q=1",
                "https://a.com/other",
                &ReferrerPolicy::StrictOriginWhenCrossOrigin
            ),
            Some("https://a.com/page?q=1".to_string())
        );
    }

    #[test]
    fn test_strict_cross_origin_https() {
        assert_eq!(
            compute_referrer(
                "https://a.com/page?q=1",
                "https://b.com/other",
                &ReferrerPolicy::StrictOriginWhenCrossOrigin
            ),
            Some("https://a.com".to_string())
        );
    }

    #[test]
    fn test_strict_downgrade() {
        assert_eq!(
            compute_referrer(
                "https://a.com/page",
                "http://b.com/other",
                &ReferrerPolicy::StrictOriginWhenCrossOrigin
            ),
            None
        );
    }
}
