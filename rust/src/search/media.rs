//! Image and video search, and the commands to fetch what was found.
//!
//! The download commands are *returned as text*, never executed. Running a shell command
//! built from strings a web page supplied would be a command-injection hole with extra steps,
//! so the caller decides — and the values are sanitised anyway, because they end up on
//! someone's terminal.

use serde_json::{json, Value};

use super::web::{by_field, google_url, merge_providers, quote_plus, Provider};
use crate::cdp::CdpClient;

/// Loaded from `js/search_images_extract.js`. See [`crate::js`] for why the snippets live
/// in real JavaScript files rather than Rust string literals.
fn image_extract_js() -> &'static str {
    include_str!("../../js/search_images_extract.js")
}

/// Loaded from `js/search_videos_extract.js`. See [`crate::js`] for why the snippets live
/// in real JavaScript files rather than Rust string literals.
fn video_extract_js() -> &'static str {
    include_str!("../../js/search_videos_extract.js")
}

// --- Google-free fallbacks (no /sorry/ wall on a clean profile) ---------------

/// Bing Images — results carry a JSON `m` attribute with the real media URL.
/// Loaded from `js/search_bing_images.js`. See [`crate::js`] for why the snippets live
/// in real JavaScript files rather than Rust string literals.
fn bing_images_js() -> &'static str {
    include_str!("../../js/search_bing_images.js")
}

/// YouTube results — genuine browser is not walled; gives real watch URLs + titles.
/// Loaded from `js/search_youtube_videos.js`. See [`crate::js`] for why the snippets live
/// in real JavaScript files rather than Rust string literals.
fn youtube_videos_js() -> &'static str {
    include_str!("../../js/search_youtube_videos.js")
}

pub(super) fn platform(url: &str) -> &'static str {
    for (host, name) in [
        ("youtube.com", "youtube"),
        ("youtu.be", "youtube"),
        ("instagram.com", "instagram"),
        ("tiktok.com", "tiktok"),
        ("vimeo.com", "vimeo"),
        ("twitter.com", "twitter"),
        ("x.com", "twitter"),
        ("facebook.com", "facebook"),
        ("dailymotion.com", "dailymotion"),
    ] {
        if url.contains(host) {
            return name;
        }
    }
    "other"
}

/// Keep filename-safe characters, replace everything else (incl. spaces) with '_'.
pub(super) fn sanitize(s: &str, max: usize) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .take(max)
        .collect()
}

pub(super) fn download_cmd_for_image(url: &str) -> String {
    if !url.starts_with("http") {
        return String::new();
    }
    let name = url
        .rsplit('/')
        .next()
        .unwrap_or("")
        .split('?')
        .next()
        .unwrap_or("");
    let name = if name.is_empty() { "image.jpg" } else { name };
    let safe = sanitize(name, 40);
    format!("curl -L -o \"{safe}\" \"{url}\"")
}

pub(super) fn download_cmd_for_video(url: &str, title: &str) -> String {
    const SUPPORTED: &[&str] = &[
        "youtube.com",
        "youtu.be",
        "vimeo.com",
        "instagram.com",
        "tiktok.com",
        "twitter.com",
        "x.com",
        "facebook.com",
        "dailymotion.com",
    ];
    if !SUPPORTED.iter().any(|p| url.contains(p)) {
        return String::new();
    }
    let safe = {
        let s = sanitize(title, 60);
        if s.is_empty() {
            "video".to_string()
        } else {
            s
        }
    };
    format!("yt-dlp -o \"{safe}.%(ext)s\" \"{url}\"")
}

/// `search_images` — merges Bing Images + Google Images (skips walled). Bing is
/// primary because it serves a genuine browser without a login wall.
pub async fn search_images(client: &CdpClient, query: &str, count: usize) -> String {
    let count = count.min(30);
    let providers = vec![
        Provider {
            name: "bing",
            url: format!("https://www.bing.com/images/search?q={}", quote_plus(query)),
            extract_js: bing_images_js().replace("COUNT", &count.to_string()),
            consent: true,
        },
        Provider {
            name: "google",
            url: google_url(query, 2),
            extract_js: image_extract_js().replace("COUNT", &count.to_string()),
            consent: true,
        },
    ];
    let (raw, engines) = merge_providers(client, providers, by_field("image_url"), count).await;
    let results: Vec<Value> = raw
        .into_iter()
        .map(|r| {
            let img = r.get("image_url").and_then(|v| v.as_str()).unwrap_or("");
            json!({
                "title": r.get("title").cloned().unwrap_or(Value::String(String::new())),
                "image_url": img,
                "source_url": r.get("source_url").cloned().unwrap_or(Value::String(String::new())),
                "source_host": r.get("source_host").cloned().unwrap_or(Value::String(String::new())),
                "description": r.get("description").cloned().unwrap_or(Value::String(String::new())),
                "download_cmd": download_cmd_for_image(img),
            })
        })
        .collect();
    json!({ "query": query, "count": results.len(), "results": results, "engines": engines })
        .to_string()
}

/// `search_videos` — merges YouTube + Google Videos (skips walled). YouTube is
/// primary because it gives real watch URLs to a genuine browser without a wall.
pub async fn search_videos(client: &CdpClient, query: &str, count: usize) -> String {
    let count = count.min(30);
    let providers = vec![
        Provider {
            name: "youtube",
            url: format!(
                "https://www.youtube.com/results?search_query={}",
                quote_plus(query)
            ),
            extract_js: youtube_videos_js().replace("COUNT", &count.to_string()),
            consent: true,
        },
        Provider {
            name: "google",
            url: google_url(query, 7),
            extract_js: video_extract_js().replace("COUNT", &count.to_string()),
            consent: true,
        },
    ];
    let (raw, engines) = merge_providers(client, providers, by_field("url"), count).await;
    let results: Vec<Value> = raw
        .into_iter()
        .map(|r| {
            let url = r.get("url").and_then(|v| v.as_str()).unwrap_or("");
            let title = r.get("title").and_then(|v| v.as_str()).unwrap_or("");
            json!({
                "title": title,
                "url": url,
                "channel": r.get("channel").cloned().unwrap_or(Value::String(String::new())),
                "duration": r.get("duration").cloned().unwrap_or(Value::String(String::new())),
                "description": r.get("description").cloned().unwrap_or(Value::String(String::new())),
                "platform": platform(url),
                "download_cmd": download_cmd_for_video(url, title),
            })
        })
        .collect();
    json!({ "query": query, "count": results.len(), "results": results, "engines": engines })
        .to_string()
}

/// `search_twitter_videos` — YouTube-hosted twitter/x clips + Google Videos scoped
/// to x.com/twitter.com, merged and filtered. No single walled source blocks it.
pub async fn search_twitter_videos(client: &CdpClient, query: &str, count: usize) -> String {
    let count = count.min(30);
    let providers = vec![
        Provider {
            name: "google_scoped",
            url: google_url(&format!("{query} (site:x.com OR site:twitter.com)"), 7),
            extract_js: video_extract_js().replace("COUNT", &(count * 3).to_string()),
            consent: true,
        },
        Provider {
            name: "youtube",
            url: format!(
                "https://www.youtube.com/results?search_query={}",
                quote_plus(&format!("{query} twitter"))
            ),
            extract_js: youtube_videos_js().replace("COUNT", &(count * 3).to_string()),
            consent: true,
        },
    ];
    // Gather generously, then keep only genuine twitter/x links.
    let (raw, engines) = merge_providers(client, providers, by_field("url"), count * 3).await;
    let results: Vec<Value> = raw
        .into_iter()
        .filter(|r| {
            r.get("url").and_then(|v| v.as_str())
                .map(|u| u.contains("x.com") || u.contains("twitter.com"))
                .unwrap_or(false)
        })
        .take(count)
        .map(|r| {
            let url = r.get("url").and_then(|v| v.as_str()).unwrap_or("");
            let title = r.get("title").and_then(|v| v.as_str()).unwrap_or("");
            json!({ "title": title, "url": url, "platform": "twitter", "download_cmd": download_cmd_for_video(url, title) })
        })
        .collect();
    json!({ "query": query, "count": results.len(), "results": results, "engines": engines })
        .to_string()
}
