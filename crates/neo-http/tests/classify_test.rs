use neo_http::{classify_url, should_skip, RequestKind};

#[test]
fn test_telemetry_urls_detected() {
    let telemetry_urls = [
        "https://www.google-analytics.com/collect?v=2",
        "https://sentry.io/api/123/envelope/",
        "https://api.segment.io/v1/track",
        "https://static.hotjar.com/c/hotjar-123.js",
        "https://bat.bing.com/action/0?ti=123",
        "https://www.facebook.com/tr?id=123&ev=PageView",
        "https://api.amplitude.com/2/httpapi",
        "https://cdn.mxpnl.com/libs/mixpanel.js",
        "https://rs.fullstory.com/rec/bundle",
        "https://js-agent.newrelic.com/nr-spa.js",
    ];
    for url in &telemetry_urls {
        assert_eq!(
            classify_url(url),
            RequestKind::Telemetry,
            "expected telemetry: {url}"
        );
        assert!(should_skip(url), "expected skip: {url}");
    }
}

#[test]
fn test_navigation_classified_correctly() {
    assert_eq!(
        classify_url("https://example.com/"),
        RequestKind::Navigation
    );
    assert_eq!(
        classify_url("https://github.com/rust-lang/rust"),
        RequestKind::Navigation
    );
    assert_eq!(
        classify_url("https://news.ycombinator.com/"),
        RequestKind::Navigation
    );
    assert!(!should_skip("https://example.com/"));
}
