//! URL classification — determines request kind and whether to skip telemetry.

use crate::RequestKind;

/// Telemetry/analytics URL patterns to block automatically.
const TELEMETRY_PATTERNS: &[&str] = &[
    "google-analytics.com",
    "googletagmanager.com",
    "googlesyndication.com",
    "googleadservices.com",
    "doubleclick.net",
    "google.com/pagead",
    "google.com/ads",
    "analytics.google.com",
    "datadoghq.com",
    "datadoghq.eu",
    "browser-intake-datadoghq",
    "sentry.io",
    "sentry-cdn.com",
    "amplitude.com",
    "api.amplitude.com",
    "cdn.amplitude.com",
    "segment.io",
    "segment.com",
    "cdn.segment.com",
    "api.segment.io",
    "hotjar.com",
    "static.hotjar.com",
    "script.hotjar.com",
    "facebook.com/tr",
    "connect.facebook.net",
    "pixel.facebook.com",
    "bat.bing.com",
    "clarity.ms",
    "mixpanel.com",
    "api.mixpanel.com",
    "cdn.mxpnl.com",
    "newrelic.com",
    "nr-data.net",
    "js-agent.newrelic.com",
    "bam.nr-data.net",
    "optimizely.com",
    "logx.optimizely.com",
    "cdn.optimizely.com",
    "fullstory.com",
    "rs.fullstory.com",
    "intercom.io",
    "widget.intercom.io",
    "api-iam.intercom.io",
    "heap.io",
    "heapanalytics.com",
    "cdn.heapanalytics.com",
    "mouseflow.com",
    "crazyegg.com",
    "script.crazyegg.com",
    "quantserve.com",
    "scorecardresearch.com",
    "sb.scorecardresearch.com",
    "adnxs.com",
    "adsrvr.org",
    "taboola.com",
    "outbrain.com",
    "criteo.com",
    "static.criteo.net",
    "bidswitch.net",
    "rubiconproject.com",
    "pubmatic.com",
    "casalemedia.com",
    "openx.net",
    "indexexchange.com",
    "trustpilot.com/tp/",
    "bugsnag.com",
    "notify.bugsnag.com",
    "app.launchdarkly.com",
    "events.launchdarkly.com",
    "cdn.cookielaw.org",
    "cookiebot.com",
];

/// Media file extensions.
const MEDIA_EXTENSIONS: &[&str] = &[
    ".mp4", ".webm", ".m3u8", ".mp3", ".ogg", ".wav", ".flac", ".avi", ".mkv", ".m4a", ".aac",
    ".ts", ".mpd",
];

/// Classify a URL into a `RequestKind`.
///
/// Uses pattern matching on the URL string to determine the type.
/// Navigation is the default for unrecognized URLs.
pub fn classify_url(url: &str) -> RequestKind {
    let lower = url.to_lowercase();

    if is_telemetry(&lower) {
        return RequestKind::Telemetry;
    }
    if is_media(&lower) {
        return RequestKind::Media;
    }
    if is_api(&lower) {
        return RequestKind::Api;
    }
    if is_subresource(&lower) {
        return RequestKind::Subresource;
    }
    RequestKind::Navigation
}

/// Returns true if the URL should be skipped entirely (telemetry/tracking).
///
/// Skipped requests never hit the network; they return `HttpError::Skipped`.
pub fn should_skip(url: &str) -> bool {
    is_telemetry(&url.to_lowercase())
}

/// Check if a lowercased URL matches known telemetry patterns.
pub(crate) fn is_telemetry_url(lower: &str) -> bool {
    TELEMETRY_PATTERNS.iter().any(|p| lower.contains(p))
}

fn is_telemetry(lower: &str) -> bool {
    is_telemetry_url(lower)
}

fn is_media(lower: &str) -> bool {
    MEDIA_EXTENSIONS.iter().any(|ext| lower.ends_with(ext))
        || lower.contains("/media/")
        || lower.contains("/stream/")
}

fn is_api(lower: &str) -> bool {
    lower.contains("/api/")
        || lower.contains("/graphql")
        || lower.contains("/rest/")
        || lower.contains("/v1/")
        || lower.contains("/v2/")
        || lower.contains("/v3/")
}

fn is_subresource(lower: &str) -> bool {
    lower.ends_with(".js")
        || lower.ends_with(".css")
        || lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".gif")
        || lower.ends_with(".svg")
        || lower.ends_with(".woff2")
        || lower.ends_with(".woff")
        || lower.ends_with(".ttf")
        || lower.ends_with(".ico")
        || lower.ends_with(".webp")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_telemetry_detected() {
        assert_eq!(
            classify_url("https://www.google-analytics.com/collect"),
            RequestKind::Telemetry
        );
        assert_eq!(
            classify_url("https://sentry.io/api/123/envelope/"),
            RequestKind::Telemetry
        );
        assert_eq!(
            classify_url("https://api.segment.io/v1/t"),
            RequestKind::Telemetry
        );
        assert_eq!(
            classify_url("https://static.hotjar.com/c/hotjar.js"),
            RequestKind::Telemetry
        );
        assert!(should_skip("https://bat.bing.com/action/0?ti=123"));
        assert!(should_skip("https://www.facebook.com/tr?id=123"));
    }

    #[test]
    fn test_navigation_classified() {
        assert_eq!(
            classify_url("https://example.com/"),
            RequestKind::Navigation
        );
        assert_eq!(
            classify_url("https://github.com/rust-lang/rust"),
            RequestKind::Navigation
        );
    }

    #[test]
    fn test_media_classified() {
        assert_eq!(
            classify_url("https://cdn.example.com/video.mp4"),
            RequestKind::Media
        );
        assert_eq!(
            classify_url("https://cdn.example.com/live.m3u8"),
            RequestKind::Media
        );
    }

    #[test]
    fn test_api_classified() {
        assert_eq!(
            classify_url("https://example.com/api/users"),
            RequestKind::Api
        );
        assert_eq!(
            classify_url("https://example.com/graphql"),
            RequestKind::Api
        );
    }

    #[test]
    fn test_subresource_classified() {
        assert_eq!(
            classify_url("https://cdn.example.com/app.js"),
            RequestKind::Subresource
        );
        assert_eq!(
            classify_url("https://cdn.example.com/style.css"),
            RequestKind::Subresource
        );
    }

    #[test]
    fn test_non_telemetry_not_skipped() {
        assert!(!should_skip("https://example.com/"));
        assert!(!should_skip("https://github.com/"));
    }
}
