//! Edge-case tests for URL classification and telemetry detection.
//!
//! After the fix: classify.rs uses domain-based matching for analytics/tracking/sentry
//! patterns, and path-based matching for /telemetry, /analytics, /tracking, /beacon.
//! This eliminates false positives where legitimate JS filenames were being blocked.

use neo_http::classify::{is_telemetry_url, is_heavy_script};
use neo_http::{classify_url, should_skip, RequestKind};

// ─── False-positive prevention (the fix) ───

#[test]
fn analytics_in_filename_not_telemetry() {
    // Fixed: "analytics" in a filename should NOT trigger telemetry detection
    assert!(
        !is_telemetry_url("https://app.example.com/performance_analytics.js"),
        "substring 'analytics' in filename should NOT trigger telemetry"
    );
}

#[test]
fn analytics_in_domain_is_telemetry() {
    assert!(is_telemetry_url("https://analytics.google.com/g/collect"));
}

#[test]
fn tracking_in_path_segment_is_telemetry() {
    // /tracking/pixel matches the path pattern /tracking
    assert!(is_telemetry_url("https://example.com/tracking/pixel.gif"));
}

#[test]
fn tracking_in_filename_not_telemetry() {
    // Fixed: "tracking" in a filename should NOT trigger telemetry
    assert!(!is_telemetry_url("https://cdn.example.com/user-tracking.js"));
}

#[test]
fn sentry_in_domain_is_telemetry() {
    assert!(is_telemetry_url("https://sentry.io/api/123/envelope/"));
}

#[test]
fn sentry_in_path_not_telemetry() {
    // Fixed: "sentry" in a non-sentry.io URL should NOT be telemetry
    assert!(!is_telemetry_url("https://example.com/sentry/capture"));
}

#[test]
fn sentry_in_unrelated_word_not_telemetry() {
    // Fixed: "sentry" in filenames should NOT match
    assert!(!is_telemetry_url("https://cdn.example.com/sentry-guard-widget.js"));
}

#[test]
fn empty_url_not_telemetry() {
    assert!(!is_telemetry_url(""));
}

#[test]
fn malformed_url_not_telemetry() {
    assert!(!is_telemetry_url("not-a-url"));
    assert!(!is_telemetry_url("://broken"));
}

#[test]
fn case_insensitivity_classify_url() {
    // classify_url lowercases internally
    assert_eq!(
        classify_url("https://www.Google-Analytics.com/collect"),
        RequestKind::Telemetry
    );
    assert_eq!(
        classify_url("https://WWW.GOOGLETAGMANAGER.COM/gtm.js"),
        RequestKind::Telemetry
    );
}

#[test]
fn case_insensitivity_should_skip() {
    assert!(should_skip("https://SENTRY.IO/api/123/envelope"));
    assert!(should_skip("https://bat.BING.com/action/0"));
}

// ─── Path-based telemetry patterns ───

#[test]
fn beacon_in_path_is_telemetry() {
    assert!(is_telemetry_url("https://example.com/beacon/fire"));
}

#[test]
fn beacon_in_filename_not_telemetry() {
    // Fixed: "beacon" in a filename should NOT match
    assert!(!is_telemetry_url("https://example.com/beacon-widget.js"));
}

#[test]
fn events_path_is_telemetry() {
    assert!(is_telemetry_url("https://example.com/events/track"));
}

// ─── should_skip ───

#[test]
fn should_skip_google_analytics() {
    assert!(should_skip("https://www.google-analytics.com/collect?v=2"));
}

#[test]
fn should_skip_segment() {
    assert!(should_skip("https://api.segment.io/v1/track"));
}

#[test]
fn should_skip_hotjar() {
    assert!(should_skip("https://static.hotjar.com/c/hotjar-123.js"));
}

#[test]
fn should_not_skip_normal_page() {
    assert!(!should_skip("https://example.com/"));
    assert!(!should_skip("https://github.com/rust-lang/rust"));
    assert!(!should_skip("https://docs.rs/tokio/latest/"));
}

#[test]
fn should_not_skip_api_endpoint() {
    assert!(!should_skip("https://api.example.com/v1/users"));
    assert!(!should_skip("https://example.com/api/data"));
}

// ─── Heavy script detection ───

#[test]
fn heavy_script_vendor_bundle() {
    assert!(is_heavy_script("https://cdn.example.com/vendor.abc123.js"));
}

#[test]
fn heavy_script_webpack_runtime() {
    assert!(is_heavy_script("https://cdn.example.com/webpack-runtime.js"));
}

#[test]
fn heavy_script_polyfills() {
    assert!(is_heavy_script("https://cdn.example.com/polyfills.es2015.js"));
}

#[test]
fn not_heavy_script_app() {
    assert!(!is_heavy_script("https://cdn.example.com/app.js"));
    assert!(!is_heavy_script("https://cdn.example.com/main.js"));
    assert!(!is_heavy_script("https://cdn.example.com/react.production.min.js"));
}

// ─── Request classification ───

#[test]
fn classify_js_as_subresource() {
    assert_eq!(
        classify_url("https://cdn.example.com/app.js"),
        RequestKind::Subresource
    );
}

#[test]
fn classify_css_as_subresource() {
    assert_eq!(
        classify_url("https://cdn.example.com/style.css"),
        RequestKind::Subresource
    );
}

#[test]
fn classify_font_as_subresource() {
    assert_eq!(
        classify_url("https://fonts.example.com/font.woff2"),
        RequestKind::Subresource
    );
}

#[test]
fn classify_video_as_media() {
    assert_eq!(
        classify_url("https://cdn.example.com/video.mp4"),
        RequestKind::Media
    );
}

#[test]
fn classify_hls_as_media() {
    assert_eq!(
        classify_url("https://stream.example.com/live.m3u8"),
        RequestKind::Media
    );
}

#[test]
fn classify_graphql_as_api() {
    assert_eq!(
        classify_url("https://example.com/graphql"),
        RequestKind::Api
    );
}

#[test]
fn classify_rest_api_as_api() {
    assert_eq!(
        classify_url("https://example.com/api/users"),
        RequestKind::Api
    );
}

#[test]
fn classify_plain_page_as_navigation() {
    assert_eq!(
        classify_url("https://example.com/about"),
        RequestKind::Navigation
    );
}

// ─── Telemetry takes priority over other classifications ───

#[test]
fn telemetry_beats_subresource() {
    // A .js URL that's also telemetry should be Telemetry, not Subresource
    assert_eq!(
        classify_url("https://www.google-analytics.com/analytics.js"),
        RequestKind::Telemetry
    );
}

#[test]
fn telemetry_beats_api() {
    // An API path that's also telemetry should be Telemetry
    assert_eq!(
        classify_url("https://api.segment.io/v1/track"),
        RequestKind::Telemetry
    );
}
