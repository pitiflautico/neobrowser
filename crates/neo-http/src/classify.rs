//! URL classification — determines request kind and whether to skip telemetry.
//!
//! Combines domain-based telemetry lists with generic tracking patterns,
//! site-specific heuristics (ChatGPT, Google), and heavy-script detection.
//! All patterns ported from NeoRender V1 plus additional coverage.

use crate::RequestKind;

/// Telemetry/analytics URL patterns to block automatically.
///
/// Domain-level patterns — matched as substrings of the lowercased URL.
const TELEMETRY_PATTERNS: &[&str] = &[
    // --- Google ecosystem ---
    "google-analytics.com",
    "googletagmanager.com",
    "googlesyndication.com",
    "googleadservices.com",
    "doubleclick.net",
    "google.com/pagead",
    "google.com/ads",
    "analytics.google.com",
    // --- Datadog ---
    "datadoghq.com",
    "datadoghq.eu",
    "browser-intake-datadoghq",
    // --- Sentry (domain-based only) ---
    "sentry.io",
    "sentry-cdn.com",
    // --- Amplitude (covered by generic keyword above) ---
    // --- Segment ---
    "segment.io",
    "segment.com",
    "cdn.segment.com",
    "api.segment.io",
    // --- Hotjar (covered by generic keyword above) ---
    // --- Facebook ---
    "facebook.com/tr",
    "connect.facebook.net",
    "pixel.facebook.com",
    // --- Bing ---
    "bat.bing.com",
    // --- Microsoft Clarity ---
    "clarity.ms",
    // --- Mixpanel ---
    "mixpanel.com",
    "api.mixpanel.com",
    "cdn.mxpnl.com",
    // --- New Relic (domain-based only) ---
    "newrelic.com",
    "nr-data.net",
    // --- Optimizely ---
    "optimizely.com",
    "logx.optimizely.com",
    "cdn.optimizely.com",
    // --- FullStory (covered by generic keyword above) ---
    // --- Intercom ---
    "intercom.io",
    "widget.intercom.io",
    "api-iam.intercom.io",
    // --- Heap ---
    "heap.io",
    "heapanalytics.com",
    "cdn.heapanalytics.com",
    // --- Session replay / heatmaps ---
    "mouseflow.com",
    "crazyegg.com",
    "script.crazyegg.com",
    // --- Ad networks ---
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
    // --- Reviews / compliance ---
    "trustpilot.com/tp/",
    // --- Error tracking ---
    "bugsnag.com",
    "notify.bugsnag.com",
    // --- Feature flags (covered by generic keyword above) ---
    // --- Cookie consent ---
    "cdn.cookielaw.org",
    "cookiebot.com",
    // --- LinkedIn tracking ---
    "linkedin.com/li/track",
    // --- Statsig (covered by generic keyword above) ---
    // --- Generic keyword patterns (domain-based to avoid false positives) ---
    "amplitude.com",
    "hotjar.com",
    "fullstory.com",
    "rs.fullstory.com",
    "launchdarkly.com",
    "statsig.com",
    // --- Generic tracking path patterns (from V1) ---
    "/log?",
    "/pixel",
    "/collect",
    "/events",
    "/gen_204",
    "/client_204",
    "/rgstr",
    // "/ces/" — REMOVED: ChatGPT Experience Settings API is NOT telemetry
    "adservice",
    "adserver",
    "/api/v1/events",
    "apfc",
    "browser-intake",
];

/// ChatGPT-specific telemetry patterns.
///
/// These patterns target OpenAI's internal analytics and experimentation
/// endpoints that fire during ChatGPT sessions.
const CHATGPT_PATTERNS: &[&str] = &[
    "ab.chatgpt",
    "oai/log",
    "featuregates",
    // ChatGPT telemetry endpoint — short path, only relevant in OAI context
    "chatgpt.com/v1/m",
];

/// Google-specific telemetry and RPC patterns.
///
/// These cover Google services beyond the ad/analytics domains already
/// in `TELEMETRY_PATTERNS`.
const GOOGLE_PATTERNS: &[&str] = &[
    "ogads-pa.",
    "play.google.com/log",
    "google.com/$rpc",
    "googleads",
];

/// Media file extensions.
const MEDIA_EXTENSIONS: &[&str] = &[
    ".mp4", ".webm", ".m3u8", ".mp3", ".ogg", ".wav", ".flac", ".avi", ".mkv", ".m4a", ".aac",
    ".ts", ".mpd",
];

/// URL patterns for heavy framework scripts that are candidates for stubbing.
///
/// Matching scripts can be replaced with no-op stubs to save bandwidth
/// and CPU during data extraction.
const HEAVY_SCRIPT_PATTERNS: &[&str] = &[
    "webpack-runtime",
    "vendor.",
    "polyfills.",
    "chunk-vendors",
];

/// Classify a URL into a [`RequestKind`].
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

/// Returns true if the URL points to a heavy framework script that could
/// be replaced with a no-op stub to reduce processing cost.
///
/// This does NOT mean the script should be blocked — only that it is a
/// candidate for lightweight stubbing during data-extraction mode.
pub fn is_heavy_script(url: &str) -> bool {
    let lower = url.to_lowercase();
    HEAVY_SCRIPT_PATTERNS.iter().any(|p| lower.contains(p))
}

/// Path-based telemetry patterns — matched against the URL path only
/// (not filenames). A path must equal the pattern or start with pattern + "/".
const TELEMETRY_PATH_PATTERNS: &[&str] = &[
    "/telemetry",
    "/analytics",
    "/tracking",
    "/beacon",
];

/// Check if a lowercased URL matches known telemetry patterns.
pub fn is_telemetry_url(lower: &str) -> bool {
    // Domain-based substring matching (safe — these are domain fragments)
    if TELEMETRY_PATTERNS.iter().any(|p| lower.contains(p))
        || CHATGPT_PATTERNS.iter().any(|p| lower.contains(p))
        || GOOGLE_PATTERNS.iter().any(|p| lower.contains(p))
    {
        return true;
    }
    // Path-based matching: only match exact path segments, not substrings
    // of filenames like "performance_analytics.js"
    if let Some(path_start) = lower.find("://").and_then(|i| lower[i + 3..].find('/').map(|j| i + 3 + j)) {
        let path = &lower[path_start..];
        // Strip query string for matching
        let path_no_query = path.split('?').next().unwrap_or(path);
        for p in TELEMETRY_PATH_PATTERNS {
            if path_no_query == *p || path_no_query.starts_with(&format!("{p}/")) {
                return true;
            }
        }
    }
    false
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

    // ---- Existing tests (preserved) ----

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

    // ---- False positive prevention ----

    #[test]
    fn test_analytics_in_filename_not_skipped() {
        // JS modules containing "analytics" in filename should NOT be telemetry
        assert!(!should_skip("https://app.factorialhr.com/performance_analytics.eewhxaothy.js"));
        assert!(!should_skip("https://cdn.example.com/analytics-dashboard.js"));
        assert!(!should_skip("https://cdn.example.com/people-analytics.bundle.js"));
    }

    #[test]
    fn test_tracking_in_content_path_not_skipped() {
        // "tracking" in a content path (e.g. order tracking) should NOT be telemetry
        assert!(!should_skip("https://example.com/order-tracking/12345"));
        assert!(!should_skip("https://example.com/shipment-tracking"));
    }

    #[test]
    fn test_sentry_in_path_not_skipped() {
        // "sentry" in a content path should NOT be telemetry (only sentry.io domain)
        assert!(!should_skip("https://example.com/docs/sentry-integration"));
        assert!(!should_skip("https://example.com/sentry/capture"));
    }

    #[test]
    fn test_analytics_domain_still_skipped() {
        // Actual analytics domains must still be detected
        assert!(should_skip("https://www.google-analytics.com/collect"));
        assert!(should_skip("https://analytics.google.com/g/collect"));
    }

    #[test]
    fn test_telemetry_path_still_skipped() {
        // Exact /telemetry path is still telemetry
        assert!(should_skip("https://example.com/telemetry"));
        assert!(should_skip("https://example.com/telemetry/v1"));
        // But telemetry-dashboard is NOT (it's a different path segment)
        assert!(!should_skip("https://example.com/telemetry-dashboard"));
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

    // ---- R8d: V1 pattern coverage ----

    /// Every skip pattern from V1 ops.rs lines 25-47 must be matched by V2.
    /// Updated: generic keyword patterns now use domain-based matching to
    /// avoid false positives on filenames like "performance_analytics.js".
    #[test]
    fn test_v1_patterns_covered() {
        let v1_urls = [
            // Path-based patterns (exact path match)
            "https://example.com/telemetry/v1",
            "https://example.com/tracking/pixel",
            "https://example.com/beacon/fire",
            // Domain-based patterns
            "https://sentry.io/api/1/capture",
            "https://newrelic.com/agent/v1",
            "https://api.amplitude.com/event",
            "https://api.segment.io/track",
            "https://static.hotjar.com/hj.js",
            "https://www.googletagmanager.com/gtm.js",
            "https://stats.g.doubleclick.net/dc.js",
            // V1-specific inline patterns
            "https://example.com/apfc/data",
            "https://example.com/rgstr/v1",
            "https://ab.chatgpt.com/config",
            "https://chatgpt.com/v1/m",
            "https://api.statsig.com/v1",
            "https://example.com/featuregates/check",
            "https://browser-intake-datadoghq.com/v2",
            "https://chatgpt.com/backend-api/oai/log",
            "https://cdn.mxpnl.com/libs/mixpanel.js",
            "https://sentry.io/api/1/store/",
            "https://rs.fullstory.com/rec/bundle",
            "https://app.launchdarkly.com/sdk/evalx",
            // Google-specific
            "https://ogads-pa.clients6.google.com/rpc",
            "https://play.google.com/log?format=json",
            "https://people-pa.google.com/$rpc/method",
            "https://www.googleads.g.doubleclick.net/pagead",
            "https://www.google.com/gen_204?atyp=i",
            "https://www.google.com/client_204?atyp=i",
            // Common ad/tracking paths
            "https://example.com/log?type=event",
            "https://example.com/pixel.gif",
            "https://example.com/collect?v=1",
            "https://example.com/events/track",
            "https://example.com/adservice/v1",
            "https://ad.adserver.net/ad",
            "https://www.facebook.com/tr?id=123",
            "https://bat.bing.com/action/0?ti=567",
            "https://www.linkedin.com/li/track?id=abc",
            "https://example.com/api/v1/events",
            // Media (V1 blocked these inline)
            "https://cdn.example.com/video.mp4",
            "https://cdn.example.com/clip.webm",
            "https://stream.example.com/live.m3u8",
        ];

        for url in &v1_urls {
            assert!(
                should_skip(url) || is_media(&url.to_lowercase()),
                "V1 pattern NOT covered in V2: {url}"
            );
        }
    }

    /// ChatGPT-specific telemetry endpoints are blocked.
    #[test]
    fn test_chatgpt_telemetry_skipped() {
        assert!(should_skip("https://ab.chatgpt.com/v1/config"));
        assert!(should_skip("https://chatgpt.com/v1/m"));
        assert!(should_skip("https://chatgpt.com/backend-api/oai/log"));
        assert!(should_skip("https://api.statsig.com/v1/initialize"));
        assert!(should_skip(
            "https://chatgpt.com/ces/featuregates/v1/check"
        ));
    }

    /// Google-specific telemetry endpoints are blocked.
    #[test]
    fn test_google_telemetry_skipped() {
        assert!(should_skip(
            "https://ogads-pa.clients6.google.com/batch"
        ));
        assert!(should_skip(
            "https://play.google.com/log?format=json&hasfast=true"
        ));
        assert!(should_skip(
            "https://people-pa.google.com/$rpc/some.Method"
        ));
        assert!(should_skip(
            "https://www.googleads.g.doubleclick.net/pagead/id"
        ));
        assert!(should_skip("https://www.google.com/gen_204?atyp=i"));
        assert!(should_skip("https://www.google.com/client_204?atyp=i"));
    }

    /// Media URLs are detected (may be blocked or classified as Media).
    #[test]
    fn test_media_skipped() {
        assert!(is_media(&"https://cdn.example.com/video.mp4".to_lowercase()));
        assert!(is_media(
            &"https://cdn.example.com/clip.webm".to_lowercase()
        ));
        assert!(is_media(
            &"https://stream.example.com/live.m3u8".to_lowercase()
        ));
        assert!(is_media(
            &"https://cdn.example.com/audio.mp3".to_lowercase()
        ));
        assert!(is_media(
            &"https://cdn.example.com/podcast.ogg".to_lowercase()
        ));
        // Not media
        assert!(!is_media(
            &"https://example.com/data.json".to_lowercase()
        ));
    }

    /// Heavy framework scripts are detected as stub candidates.
    #[test]
    fn test_heavy_script_detected() {
        assert!(is_heavy_script(
            "https://cdn.example.com/js/webpack-runtime.abc123.js"
        ));
        assert!(is_heavy_script(
            "https://cdn.example.com/vendor.bundle.js"
        ));
        assert!(is_heavy_script(
            "https://cdn.example.com/polyfills.es5.js"
        ));
        assert!(is_heavy_script(
            "https://cdn.example.com/chunk-vendors.js"
        ));
        // Not heavy
        assert!(!is_heavy_script("https://cdn.example.com/app.js"));
        assert!(!is_heavy_script("https://cdn.example.com/main.js"));
    }

    /// Legitimate URLs must NOT be skipped as telemetry.
    #[test]
    fn test_legitimate_not_skipped() {
        // Normal API endpoints
        assert!(!should_skip("https://api.example.com/data"));
        assert!(!should_skip("https://example.com/users/123"));
        assert!(!should_skip("https://github.com/rust-lang/rust"));
        assert!(!should_skip("https://docs.rs/tokio/latest/"));
        // Pages with "log" in path but not "/log?" pattern
        assert!(!should_skip("https://example.com/blog/my-post"));
        assert!(!should_skip("https://example.com/catalog/items"));
        // API with /v1/ path — classified as Api, not Telemetry
        // (unless it also matches a telemetry pattern)
        assert!(!should_skip("https://api.example.com/v1/users"));
        assert!(!should_skip("https://example.com/v1/documents"));
    }
}
