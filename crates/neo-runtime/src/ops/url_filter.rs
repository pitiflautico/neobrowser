//! URL skip logic for telemetry/analytics endpoints.

/// Check if a URL should be skipped (telemetry, analytics, tracking).
pub fn should_skip_url(url: &str) -> bool {
    // Only skip KNOWN tracking/analytics endpoints, not JS modules that
    // happen to have "analytics" in their filename.
    // Match on path patterns, not arbitrary substrings.
    const SKIP_EXACT_HOSTS: &[&str] = &[
        "googletagmanager.com",
        "google-analytics.com",
        "analytics.google.com",
        "doubleclick.net",
        "facebook.com/tr",
        "bat.bing.com",
        "hotjar.com",
        "sentry.io",
        "newrelic.com",
        "amplitude.com",
        "segment.com/v1",
        "segment.io",
    ];
    const SKIP_PATHS: &[&str] = &[
        "/collect",
        "/pixel",
        "/beacon",
        "/telemetry",
    ];
    // Check host-based skips
    if SKIP_EXACT_HOSTS.iter().any(|h| url.contains(h)) {
        return true;
    }
    // Check path-based skips (only match path portion, not filenames)
    if let Ok(parsed) = url::Url::parse(url) {
        let path = parsed.path();
        if SKIP_PATHS.iter().any(|p| path == *p || path.starts_with(&format!("{p}/"))) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skip_url_tracking_domains() {
        assert!(should_skip_url("https://www.google-analytics.com/collect"));
        assert!(should_skip_url("https://analytics.google.com/g/collect"));
        assert!(should_skip_url("https://sentry.io/api/123/envelope"));
        assert!(should_skip_url("https://api.amplitude.com/2/httpapi"));
        assert!(should_skip_url("https://bat.bing.com/action"));
    }

    #[test]
    fn skip_url_does_not_skip_app_modules() {
        assert!(!should_skip_url(
            "https://app.factorialhr.com/performance_analytics.eewhxaothy.js"
        ));
        assert!(!should_skip_url("https://example.com/analytics-dashboard.js"));
        assert!(!should_skip_url(
            "https://cdn.example.com/people-analytics.bundle.js"
        ));
    }

    #[test]
    fn skip_url_path_patterns() {
        assert!(should_skip_url("https://example.com/collect"));
        assert!(should_skip_url("https://example.com/pixel"));
        assert!(should_skip_url("https://example.com/beacon"));
        assert!(should_skip_url("https://example.com/telemetry"));
    }

    #[test]
    fn skip_url_does_not_skip_similar_paths() {
        assert!(!should_skip_url("https://example.com/api/collection"));
        assert!(!should_skip_url("https://example.com/pixels.js"));
        assert!(!should_skip_url("https://example.com/telemetry-dashboard"));
    }

    #[test]
    fn skip_url_normal_urls_pass_through() {
        assert!(!should_skip_url("https://app.factorialhr.com/vendor.js"));
        assert!(!should_skip_url("https://cdn.example.com/react.js"));
        assert!(!should_skip_url("https://example.com/api/users"));
        assert!(!should_skip_url("https://example.com/login"));
    }

    #[test]
    fn skip_url_analytics_in_filename_not_skipped() {
        assert!(!should_skip_url(
            "https://cdn.example.com/analytics-widget.js"
        ));
        assert!(!should_skip_url(
            "https://app.example.com/hr-analytics.bundle.js"
        ));
    }

    #[test]
    fn skip_url_tracking_in_path_not_skipped() {
        assert!(!should_skip_url(
            "https://example.com/order-tracking/12345"
        ));
        assert!(!should_skip_url(
            "https://example.com/shipment-tracking"
        ));
    }

    #[test]
    fn skip_url_sentry_in_domain_skipped() {
        assert!(should_skip_url("https://sentry.io/api/123/store/"));
        assert!(should_skip_url("https://sentry.io/api/456/envelope/"));
    }

    #[test]
    fn skip_url_sentry_in_path_not_skipped() {
        assert!(!should_skip_url(
            "https://example.com/docs/sentry-integration"
        ));
    }

    #[test]
    fn skip_url_empty() {
        assert!(!should_skip_url(""));
    }

    #[test]
    fn skip_url_malformed() {
        assert!(!should_skip_url("not-a-url"));
        assert!(!should_skip_url("://broken"));
    }

    #[test]
    fn skip_url_case_sensitivity() {
        assert!(!should_skip_url("https://SENTRY.IO/api/123"));
        assert!(should_skip_url("https://sentry.io/api/123"));
    }

    #[test]
    fn skip_url_collect_subpath() {
        assert!(should_skip_url("https://example.com/collect/data"));
    }

    #[test]
    fn skip_url_collect_with_query() {
        assert!(should_skip_url("https://example.com/collect?v=2"));
    }

    #[test]
    fn skip_url_pixel_exact() {
        assert!(should_skip_url("https://example.com/pixel"));
    }

    #[test]
    fn skip_url_pixel_with_extension_not_skipped() {
        assert!(!should_skip_url("https://example.com/pixels.js"));
    }

    #[test]
    fn skip_url_beacon_exact() {
        assert!(should_skip_url("https://example.com/beacon"));
    }

    #[test]
    fn skip_url_telemetry_subpath() {
        assert!(should_skip_url("https://example.com/telemetry/v1"));
    }

    #[test]
    fn skip_url_telemetry_dashboard_not_skipped() {
        assert!(!should_skip_url("https://example.com/telemetry-dashboard"));
    }
}
