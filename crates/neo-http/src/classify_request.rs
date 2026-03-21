//! Rich request classification — multi-signal categorization.
//!
//! Unlike [`classify_url`](super::classify_url) which only looks at the URL,
//! [`classify_request`] considers URL, initiator, and content-type to
//! produce a fine-grained [`RequestCategory`].

use super::classify::is_telemetry_url;

/// Fine-grained category for an HTTP request.
///
/// More specific than [`RequestKind`](crate::RequestKind) — separates
/// scripts from modules, images from fonts, XHR from fetch, etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum RequestCategory {
    /// Top-level page load.
    Navigation,
    /// Iframe or frame document.
    Document,
    /// Classic `<script src>`.
    Script,
    /// `<script type=module>` or dynamic `import()`.
    Module,
    /// XMLHttpRequest.
    XHR,
    /// JS `fetch()` call.
    Fetch,
    /// `<img>`, CSS background-image, favicon.
    Image,
    /// `<link rel=stylesheet>`.
    Style,
    /// `@font-face` resource.
    Font,
    /// Analytics, tracking, ad pixels.
    Telemetry,
    /// `navigator.sendBeacon`.
    Beacon,
    /// Video, audio, HLS/DASH manifests.
    Media,
    /// Anything else.
    Other,
}

/// URL patterns that may override the block list.
#[derive(Debug, Clone, Default)]
pub struct ClassificationOverrides {
    /// URL substrings to always allow (bypass telemetry blocking).
    pub allow_patterns: Vec<String>,
    /// URL substrings to always block (treated as telemetry).
    pub block_patterns: Vec<String>,
}

impl ClassificationOverrides {
    /// Check if `url` matches any allow pattern.
    pub fn is_allowed(&self, url: &str) -> bool {
        let lower = url.to_lowercase();
        self.allow_patterns.iter().any(|p| lower.contains(p))
    }

    /// Check if `url` matches any block pattern.
    pub fn is_blocked(&self, url: &str) -> bool {
        let lower = url.to_lowercase();
        self.block_patterns.iter().any(|p| lower.contains(p))
    }
}

/// Image file extensions.
const IMAGE_EXTENSIONS: &[&str] = &[
    ".png", ".jpg", ".jpeg", ".gif", ".svg", ".webp", ".ico", ".avif", ".bmp",
];

/// Font file extensions.
const FONT_EXTENSIONS: &[&str] = &[".woff2", ".woff", ".ttf", ".otf", ".eot"];

/// Style extensions.
const STYLE_EXTENSIONS: &[&str] = &[".css"];

/// Media extensions (video/audio).
const MEDIA_EXTENSIONS: &[&str] = &[
    ".mp4", ".webm", ".m3u8", ".mp3", ".ogg", ".wav", ".flac", ".avi", ".mkv", ".m4a", ".aac",
    ".mpd",
];

/// Classify a request using URL, initiator hint, and content-type.
///
/// Priority: overrides > telemetry > content-type > initiator > URL extension.
pub fn classify_request(
    url: &str,
    initiator: Option<&str>,
    content_type: Option<&str>,
    overrides: Option<&ClassificationOverrides>,
) -> RequestCategory {
    let lower = url.to_lowercase();

    // 1. Override: forced block → Telemetry.
    if let Some(ov) = overrides {
        if ov.is_blocked(url) && !ov.is_allowed(url) {
            return RequestCategory::Telemetry;
        }
    }

    // 2. Telemetry (unless explicitly allowed).
    let allowed = overrides.is_some_and(|ov| ov.is_allowed(url));
    if !allowed && is_telemetry_url(&lower) {
        return RequestCategory::Telemetry;
    }

    // 3. Content-type signals.
    if let Some(ct) = content_type {
        let ct_lower = ct.to_lowercase();
        if ct_lower.contains("text/html") || ct_lower.contains("application/xhtml") {
            return match initiator {
                Some("iframe") | Some("frame") => RequestCategory::Document,
                _ => RequestCategory::Navigation,
            };
        }
        if ct_lower.contains("javascript") || ct_lower.contains("ecmascript") {
            return classify_script(&lower, initiator);
        }
        if ct_lower.contains("text/css") {
            return RequestCategory::Style;
        }
        if ct_lower.contains("image/") {
            return RequestCategory::Image;
        }
        if ct_lower.contains("font/") || ct_lower.contains("application/font") {
            return RequestCategory::Font;
        }
        if ct_lower.contains("video/") || ct_lower.contains("audio/") {
            return RequestCategory::Media;
        }
    }

    // 4. Initiator-based hints.
    if let Some(init) = initiator {
        match init {
            "beacon" | "sendbeacon" => return RequestCategory::Beacon,
            "xmlhttprequest" | "xhr" => return RequestCategory::XHR,
            "fetch" => return RequestCategory::Fetch,
            "script" => return classify_script_or_xhr(&lower),
            "iframe" | "frame" => return RequestCategory::Document,
            _ => {}
        }
    }

    // 5. URL extension fallback.
    classify_by_extension(&lower)
}

/// Distinguish Script vs Module from URL + initiator.
fn classify_script(lower: &str, initiator: Option<&str>) -> RequestCategory {
    if lower.contains("module") || lower.contains(".mjs") {
        return RequestCategory::Module;
    }
    if let Some(init) = initiator {
        if init == "module" || init == "import" {
            return RequestCategory::Module;
        }
    }
    RequestCategory::Script
}

/// When initiator is "script", decide between XHR/Fetch/Script.
fn classify_script_or_xhr(lower: &str) -> RequestCategory {
    if lower.ends_with(".js") || lower.ends_with(".mjs") {
        return RequestCategory::Script;
    }
    // API-like URLs initiated by script → Fetch.
    if lower.contains("/api/") || lower.contains("/graphql") {
        return RequestCategory::Fetch;
    }
    RequestCategory::XHR
}

/// Classify purely by URL file extension.
fn classify_by_extension(lower: &str) -> RequestCategory {
    if lower.ends_with(".js") {
        return RequestCategory::Script;
    }
    if lower.ends_with(".mjs") {
        return RequestCategory::Module;
    }
    if STYLE_EXTENSIONS.iter().any(|e| lower.ends_with(e)) {
        return RequestCategory::Style;
    }
    if IMAGE_EXTENSIONS.iter().any(|e| lower.ends_with(e)) {
        return RequestCategory::Image;
    }
    if FONT_EXTENSIONS.iter().any(|e| lower.ends_with(e)) {
        return RequestCategory::Font;
    }
    if MEDIA_EXTENSIONS.iter().any(|e| lower.ends_with(e)) {
        return RequestCategory::Media;
    }
    RequestCategory::Navigation
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_script_vs_module() {
        // .js with module initiator → Module.
        let cat = classify_request(
            "https://cdn.example.com/app.js",
            Some("module"),
            Some("application/javascript"),
            None,
        );
        assert_eq!(cat, RequestCategory::Module);

        // Plain .js → Script.
        let cat = classify_request(
            "https://cdn.example.com/app.js",
            None,
            Some("application/javascript"),
            None,
        );
        assert_eq!(cat, RequestCategory::Script);

        // .mjs → Module by extension.
        let cat = classify_request("https://cdn.example.com/chunk.mjs", None, None, None);
        assert_eq!(cat, RequestCategory::Module);
    }

    #[test]
    fn test_classify_xhr_by_initiator() {
        let cat = classify_request(
            "https://example.com/api/data",
            Some("xmlhttprequest"),
            None,
            None,
        );
        assert_eq!(cat, RequestCategory::XHR);

        let cat = classify_request("https://example.com/api/data", Some("fetch"), None, None);
        assert_eq!(cat, RequestCategory::Fetch);
    }

    #[test]
    fn test_classification_override() {
        let overrides = ClassificationOverrides {
            allow_patterns: vec!["my-analytics.internal".into()],
            block_patterns: vec!["evil-tracker.com".into()],
        };

        // Telemetry URL allowed by override.
        let cat = classify_request(
            "https://my-analytics.internal/collect",
            None,
            None,
            Some(&overrides),
        );
        assert_ne!(cat, RequestCategory::Telemetry);

        // Normal URL blocked by override.
        let cat = classify_request(
            "https://evil-tracker.com/pixel",
            None,
            None,
            Some(&overrides),
        );
        assert_eq!(cat, RequestCategory::Telemetry);
    }

    #[test]
    fn test_classify_image_font_style_media_beacon() {
        assert_eq!(classify_request("https://cdn.example.com/logo.png", None, None, None), RequestCategory::Image);
        assert_eq!(classify_request("https://cdn.example.com/font.woff2", None, None, None), RequestCategory::Font);
        assert_eq!(classify_request("https://cdn.example.com/main.css", None, None, None), RequestCategory::Style);
        assert_eq!(classify_request("https://cdn.example.com/video.mp4", None, None, None), RequestCategory::Media);
        assert_eq!(classify_request("https://example.com/log", Some("beacon"), None, None), RequestCategory::Beacon);
    }

    #[test]
    fn test_classify_document_and_telemetry() {
        assert_eq!(classify_request("https://example.com/embed", Some("iframe"), Some("text/html"), None), RequestCategory::Document);
        assert_eq!(classify_request("https://www.google-analytics.com/collect", None, None, None), RequestCategory::Telemetry);
    }
}
