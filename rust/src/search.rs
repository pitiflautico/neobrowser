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

const DISMISS_CONSENT_JS: &str = r#"(function() {
    const btns = Array.from(document.querySelectorAll('button, [role="button"]'));
    const accept = btns.find(b => /accept all|aceptar todo|tout accepter|alle akzeptieren/i.test(b.innerText));
    if (accept) { accept.click(); return true; }
    return false;
})();"#;

async fn dismiss_consent(client: &CdpClient) {
    let _ = page::js(client, DISMISS_CONSENT_JS).await;
}

const GOOGLE_TEXT_JS: &str = r#"return JSON.stringify((function(limit){
    const out = [], seen = new Set();
    document.querySelectorAll('a h3').forEach(function(h3){
        if (out.length >= limit) return;
        const a = h3.closest('a[href]'); if (!a) return;
        let href = a.href || '';
        if (!href || href.indexOf('https://www.google.') === 0 || seen.has(href)) return;
        seen.add(href);
        let snip = '';
        const c = a.closest('div.g, div.MjjYud, div[data-hveid]');
        if (c) { const s = c.querySelector('.VwiC3b, div[data-sncf], span'); if (s) snip = s.textContent.slice(0,220); }
        out.push({title: h3.textContent.trim(), url: href, snippet: snip.trim()});
    });
    return out;
})(LIMIT))"#;

const DDG_TEXT_JS: &str = r#"return JSON.stringify((function(limit){
    const out = [], seen = new Set();
    document.querySelectorAll('.result__body, .result').forEach(function(r){
        if (out.length >= limit) return;
        if ((r.className || '').indexOf('result--ad') !== -1) return;
        const a = r.querySelector('.result__a'); if (!a) return;
        let href = a.href || '';
        if (href.indexOf('/y.js') !== -1 || href.indexOf('ad_domain') !== -1) return;
        try { const u = new URL(href); if (u.searchParams.get('uddg')) href = u.searchParams.get('uddg'); } catch(e){}
        if (!href || seen.has(href)) return;
        seen.add(href);
        const sn = r.querySelector('.result__snippet');
        out.push({title: a.textContent.trim(), url: href, snippet: sn ? sn.textContent.trim() : ''});
    });
    return out;
})(LIMIT))"#;

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
            extract_js: DDG_TEXT_JS.replace("LIMIT", &limit.to_string()),
            consent: false,
        },
        Provider {
            name: "google",
            url: format!(
                "https://www.google.com/search?q={}&hl=en&num=20",
                quote_plus(query)
            ),
            extract_js: GOOGLE_TEXT_JS.replace("LIMIT", &limit.to_string()),
            consent: true,
        },
    ];
    let (mut results, engines) = merge_providers(client, providers, by_field("url"), limit).await;
    results.truncate(limit);
    json!({ "query": query, "results": results, "engines": engines }).to_string()
}

const IMAGE_EXTRACT_JS: &str = r#"return (function(count) {
    const results = [];
    const seen = new Set();
    const scriptText = Array.from(document.querySelectorAll('script')).map(s => s.text).join('\n');
    const imgPattern = /https?:\/\/(?!encrypted-tbn)[\w.\-/%?=&+@#!:,;~]+\.(?:jpg|jpeg|png|webp)(?:[?&][^"'\s<>\\]{0,120})?/gi;
    const rawUrls = [...new Set(scriptText.match(imgPattern) || [])];
    const sourcePairs = Array.from(document.querySelectorAll('a[href^="http"]'))
        .filter(a => !a.href.includes('google.com'))
        .map(a => ({href: a.href, text: a.innerText?.trim() || ''}));
    const imgMeta = {};
    Array.from(document.querySelectorAll('img[alt], img[title]')).forEach(img => {
        const key = (img.src || '').split('?')[0];
        if (key) imgMeta[key] = img.alt || img.title || '';
    });
    const filtered = rawUrls
        .filter(u => !u.includes('gstatic.com') && !u.includes('google.com') && !u.includes('googleapis.com') && u.length > 30)
        .slice(0, count * 3);
    for (const imgUrl of filtered) {
        if (seen.has(imgUrl)) continue;
        seen.add(imgUrl);
        let host = '';
        try { host = new URL(imgUrl).hostname.replace(/^www\./, ''); } catch(e) {}
        const sourcePair = sourcePairs.find(p => {
            try { const ph = new URL(p.href).hostname.replace(/^www\./, ''); return ph.includes(host) || host.includes(ph); }
            catch { return false; }
        });
        results.push({
            image_url: imgUrl,
            source_url: sourcePair?.href || '',
            source_host: host,
            title: imgMeta[imgUrl.split('?')[0]] || sourcePair?.text?.split('\n')[0] || '',
            description: sourcePair?.text || '',
        });
        if (results.length >= count) break;
    }
    return JSON.stringify(results);
})(COUNT);"#;

const VIDEO_EXTRACT_JS: &str = r#"return (function(count) {
    const results = [];
    const seen = new Set();
    const durationRe = /^\d{1,2}:\d{2}(:\d{2})?$/;
    const headings = Array.from(document.querySelectorAll('h3'));
    for (const h3 of headings) {
        if (results.length >= count) break;
        const a = h3.closest('a') || h3.parentElement?.querySelector('a');
        const url = a?.href || '';
        if (!url || seen.has(url)) continue;
        seen.add(url);
        let card = h3.parentElement;
        for (let i = 0; i < 8; i++) { if (!card) break; if (card.innerText?.length > 60) break; card = card.parentElement; }
        const lines = (card?.innerText || '').split('\n').map(l => l.trim()).filter(l => l);
        let duration = '', description = '', channel = '';
        const bodyLines = lines.filter(l => l !== h3.innerText && !l.includes('www.') && !l.startsWith('›'));
        for (const line of bodyLines) {
            if (!duration && durationRe.test(line)) { duration = line; continue; }
            if (line.includes('·')) {
                const parts = line.split('·').map(p => p.trim());
                if (parts.length >= 2 && !channel) {
                    const candidate = parts[1] || parts[0] || '';
                    if (candidate.length < 60 && !/\d+\s*(year|month|day|view|ago)/i.test(candidate)) channel = candidate;
                }
                continue;
            }
            if (!description && line.length > 20) description = line;
        }
        results.push({ title: h3.innerText, url, channel, duration, description: description.slice(0, 300) });
    }
    return JSON.stringify(results);
})(COUNT);"#;

// --- Google-free fallbacks (no /sorry/ wall on a clean profile) ---------------

/// Bing Images — results carry a JSON `m` attribute with the real media URL.
const BING_IMAGES_JS: &str = r#"return JSON.stringify((function(count){
    const out = [], seen = new Set();
    document.querySelectorAll('a.iusc').forEach(function(a){
        if (out.length >= count) return;
        let m = {}; try { m = JSON.parse(a.getAttribute('m') || '{}'); } catch(e) {}
        const img = m.murl || ''; if (!img || seen.has(img)) return; seen.add(img);
        let host = ''; try { host = new URL(img).hostname.replace(/^www\./,''); } catch(e) {}
        out.push({ image_url: img, source_url: m.purl || '', source_host: host, title: m.t || '', description: m.desc || '' });
    });
    return out;
})(COUNT))"#;

/// YouTube results — genuine browser is not walled; gives real watch URLs + titles.
const YOUTUBE_VIDEOS_JS: &str = r#"return JSON.stringify((function(count){
    const out = [], seen = new Set();
    document.querySelectorAll('a#video-title, a#video-title-link, a.yt-simple-endpoint#video-title').forEach(function(a){
        if (out.length >= count) return;
        let href = a.href || ''; if (!href.includes('/watch') || seen.has(href)) return; seen.add(href);
        const title = (a.getAttribute('title') || a.textContent || '').trim();
        let channel = '';
        const card = a.closest('ytd-video-renderer, ytd-rich-item-renderer');
        if (card) { const ch = card.querySelector('ytd-channel-name a, #channel-name a'); if (ch) channel = ch.textContent.trim(); }
        out.push({ url: href, title: title, channel: channel, duration: '', description: '' });
    });
    return out;
})(COUNT))"#;

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
            extract_js: BING_IMAGES_JS.replace("COUNT", &count.to_string()),
            consent: true,
        },
        Provider {
            name: "google",
            url: google_url(query, 2),
            extract_js: IMAGE_EXTRACT_JS.replace("COUNT", &count.to_string()),
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
            extract_js: YOUTUBE_VIDEOS_JS.replace("COUNT", &count.to_string()),
            consent: true,
        },
        Provider {
            name: "google",
            url: google_url(query, 7),
            extract_js: VIDEO_EXTRACT_JS.replace("COUNT", &count.to_string()),
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
            extract_js: VIDEO_EXTRACT_JS.replace("COUNT", &(count * 3).to_string()),
            consent: true,
        },
        Provider {
            name: "youtube",
            url: format!(
                "https://www.youtube.com/results?search_query={}",
                quote_plus(&format!("{query} twitter"))
            ),
            extract_js: YOUTUBE_VIDEOS_JS.replace("COUNT", &(count * 3).to_string()),
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
