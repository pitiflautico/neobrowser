//! Browser-driven search: text (Google → DuckDuckGo fallback), images, videos.
//!
//! Ported from the Python `_search_google`/`_search_duckduckgo` and
//! `google_search.py`. Search runs through the real stealth browser because a raw
//! HTTP fetch to Google/DDG gets bot-blocked. The Google image/video extraction
//! blobs are Google-DOM-specific and ported verbatim; they (like the Python
//! originals) may need selector updates when Google changes its markup.
//!
//! Split into [`web`] (multi-provider text search) and [`media`] (images, video, and the
//! commands to fetch them).

pub mod media;
pub mod web;

pub use media::{search_images, search_twitter_videos, search_videos};
pub use web::search;

#[cfg(test)]
mod tests {
    use super::media::{download_cmd_for_image, download_cmd_for_video, platform};
    use super::web::{google_url, quote_plus};

    #[test]
    fn quote_plus_encodes() {
        assert_eq!(quote_plus("hello world"), "hello+world");
        assert_eq!(quote_plus("a&b=c"), "a%26b%3Dc");
        assert_eq!(quote_plus("café"), "caf%C3%A9");
    }

    #[test]
    fn google_url_has_udm_and_num() {
        let u = google_url("cats", 2);
        assert!(u.contains("q=cats"));
        assert!(u.contains("udm=2"));
        assert!(u.contains("num=30"));
    }

    #[test]
    fn platform_detection() {
        assert_eq!(platform("https://youtu.be/abc"), "youtube");
        assert_eq!(platform("https://x.com/user/status/1"), "twitter");
        assert_eq!(platform("https://example.com/v"), "other");
    }

    #[test]
    fn download_cmds() {
        assert!(download_cmd_for_image("https://a.com/pic.jpg").starts_with("curl -L -o"));
        assert_eq!(download_cmd_for_image("data:x"), "");
        assert!(download_cmd_for_video("https://youtu.be/x", "My Video!").contains("yt-dlp"));
        assert_eq!(download_cmd_for_video("https://example.com/v", "t"), "");
    }
}
