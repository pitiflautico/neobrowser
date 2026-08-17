//! Browser-driven search: text (Google → DuckDuckGo fallback), images, videos.
//!
//! Ported from the Python `_search_google`/`_search_duckduckgo` and
//! `google_search.py`. Search runs through the real stealth browser because a raw
//! HTTP fetch to Google/DDG gets bot-blocked. The Google image/video extraction
//! blobs are Google-DOM-specific and ported verbatim; they (like the Python
//! originals) may need selector updates when Google changes its markup.

use serde_json::{json, Value};

use crate::cdp::CdpClient;
use crate::page;

/// Percent-encode a query the `quote_plus` way (space → '+').
fn quote_plus(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn google_url(query: &str, udm: u8) -> String {
    format!(
        "https://www.google.com/search?q={}&udm={}&num=30",
        quote_plus(query),
        udm
    )
}

/// Loaded from `js/dismiss_consent.js`. See [`crate::js`] for why the snippets live
/// in real JavaScript files rather than Rust string literals.
fn dismiss_consent_js() -> &'static str {
    include_str!("../js/dismiss_consent.js")
}

async fn dismiss_consent(client: &CdpClient) {
    let _ = page::js(client, dismiss_consent_js()).await;
}

/// Loaded from `js/search_google_text.js`. See [`crate::js`] for why the snippets live
/// in real JavaScript files rather than Rust string literals.
fn google_text_js() -> &'static str {
    include_str!("../js/search_google_text.js")
}

/// Loaded from `js/search_ddg_text.js`. See [`crate::js`] for why the snippets live
/// in real JavaScript files rather than Rust string literals.
fn ddg_text_js() -> &'static str {
    include_str!("../js/search_ddg_text.js")
}

async fn js_array(client: &CdpClient, code: &str) -> Vec<Value> {
    match page::js(client, code).await {
        Ok(Value::String(s)) => serde_json::from_str(&s).unwrap_or_default(),
        Ok(Value::Array(a)) => a,
        _ => Vec::new(),
    }
}

/// One search source: a URL to open plus JS that returns an array of results.
struct Provider {
    name: &'static str,
    url: String,
    extract_js: String,
    consent: bool,
}

/// Run providers in order: navigate, dismiss consent, skip if the page is walled
/// (bot wall / captcha / etc.), extract, and merge deduped results until `count`.
///
/// This is the general answer to "we'll hit this on many more sites": no single
/// source is a hard dependency — a walled or empty provider is transparently
/// skipped and the next one fills in. Returns (results, per-engine trace).
async fn merge_providers(
    client: &CdpClient,
    providers: Vec<Provider>,
    key: impl Fn(&Value) -> Option<String>,
    count: usize,
) -> (Vec<Value>, Vec<Value>) {
    let mut out: Vec<Value> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut trace: Vec<Value> = Vec::new();
    for p in providers {
        if out.len() >= count {
            break;
        }
        if page::navigate(client, &p.url, 3.0).await.is_err() {
            trace.push(json!({ "engine": p.name, "error": "navigate failed" }));
            continue;
        }
        if p.consent {
            dismiss_consent(client).await;
        }
        if let Some(w) = crate::walls::detect(client).await {
            trace.push(json!({ "engine": p.name, "walled": w.as_str() }));
            continue;
        }
        let before = out.len();
        for item in js_array(client, &p.extract_js).await {
            if let Some(k) = key(&item) {
                if seen.insert(k) {
                    out.push(item);
                    if out.len() >= count {
                        break;
                    }
                }
            }
        }
        trace.push(json!({ "engine": p.name, "got": out.len() - before }));
    }
    (out, trace)
}

fn by_field(field: &'static str) -> impl Fn(&Value) -> Option<String> {
    move |v: &Value| {
        v.get(field)
            .and_then(|x| x.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from)
    }
}

/// `search` — merges DuckDuckGo + Google (skips whichever is walled).
pub async fn search(client: &CdpClient, query: &str, limit: usize) -> String {
    let providers = vec![
        Provider {
            name: "duckduckgo",
            url: format!("https://html.duckduckgo.com/html/?q={}", quote_plus(query)),
            extract_js: ddg_text_js().replace("LIMIT", &limit.to_string()),
            consent: false,
        },
        Provider {
            name: "google",
            url: format!(
                "https://www.google.com/search?q={}&hl=en&num=20",
                quote_plus(query)
            ),
            extract_js: google_text_js().replace("LIMIT", &limit.to_string()),
            consent: true,
        },
    ];
    let (mut results, engines) = merge_providers(client, providers, by_field("url"), limit).await;
    results.truncate(limit);
    json!({ "query": query, "results": results, "engines": engines }).to_string()
}

/// Loaded from `js/search_images_extract.js`. See [`crate::js`] for why the snippets live
/// in real JavaScript files rather than Rust string literals.
fn image_extract_js() -> &'static str {
    include_str!("../js/search_images_extract.js")
}

/// Loaded from `js/search_videos_extract.js`. See [`crate::js`] for why the snippets live
/// in real JavaScript files rather than Rust string literals.
fn video_extract_js() -> &'static str {
    include_str!("../js/search_videos_extract.js")
}

// --- Google-free fallbacks (no /sorry/ wall on a clean profile) ---------------

/// Bing Images — results carry a JSON `m` attribute with the real media URL.
/// Loaded from `js/search_bing_images.js`. See [`crate::js`] for why the snippets live
/// in real JavaScript files rather than Rust string literals.
fn bing_images_js() -> &'static str {
    include_str!("../js/search_bing_images.js")
}

/// YouTube results — genuine browser is not walled; gives real watch URLs + titles.
/// Loaded from `js/search_youtube_videos.js`. See [`crate::js`] for why the snippets live
/// in real JavaScript files rather than Rust string literals.
fn youtube_videos_js() -> &'static str {
    include_str!("../js/search_youtube_videos.js")
}

fn platform(url: &str) -> &'static str {
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
fn sanitize(s: &str, max: usize) -> String {
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

fn download_cmd_for_image(url: &str) -> String {
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

fn download_cmd_for_video(url: &str, title: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

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
