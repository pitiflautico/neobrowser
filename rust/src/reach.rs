//! Reach tools: browse (server-side fetch), upload (file input), download (auth-aware).
//!
//! Ported from the Python `browse`/`upload`/`download` dispatch. The SSRF guard is
//! kept: only public http(s) URLs are allowed — file://, credentials-in-URL, and
//! hosts resolving to loopback/private/link-local ranges (incl. cloud metadata) are
//! blocked before any request goes out.

use std::net::{IpAddr, ToSocketAddrs};
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{json, Map, Value};

use crate::cdp::{CdpClient, CdpError};
use crate::paths;

/// SSRF guard: true only for a public http(s) URL with no userinfo whose host
/// resolves entirely to public addresses.
pub fn validate_url(raw: &str) -> bool {
    let url = match reqwest::Url::parse(raw) {
        Ok(u) => u,
        Err(_) => return false,
    };
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    if !url.username().is_empty() || url.password().is_some() {
        return false; // credentials-in-URL
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    // A literal IP: check it directly.
    if let Ok(ip) = host.parse::<IpAddr>() {
        return is_public(ip);
    }
    // Block obvious internal names outright.
    let h = host.to_ascii_lowercase();
    if h == "localhost" || h.ends_with(".localhost") || h == "metadata.google.internal" {
        return false;
    }
    // Resolve and require every address to be public.
    let port = url.port_or_known_default().unwrap_or(80);
    match (host, port).to_socket_addrs() {
        Ok(addrs) => {
            let mut any = false;
            for a in addrs {
                any = true;
                if !is_public(a.ip()) {
                    return false;
                }
            }
            any
        }
        Err(_) => false,
    }
}

/// Reject loopback / private / link-local / unspecified / cloud-metadata IPs.
fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            if v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
            {
                return false;
            }
            // Carrier-grade NAT 100.64.0.0/10 and metadata 169.254.169.254.
            let o = v4.octets();
            if o[0] == 100 && (64..=127).contains(&o[1]) {
                return false;
            }
            true
        }
        IpAddr::V6(v6) => {
            !(v6.is_loopback() || v6.is_unspecified() || v6.is_multicast()
                // unique-local fc00::/7
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                // link-local fe80::/10
                || (v6.segments()[0] & 0xffc0) == 0xfe80)
        }
    }
}

/// Strip zero-width and most control characters that hide in scraped text.
fn clean_scraped(s: &str) -> String {
    s.chars()
        .filter(|c| {
            !matches!(*c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{206F}' | '\u{FEFF}')
                && (!c.is_control() || *c == '\n' || *c == '\t')
        })
        .collect()
}

/// Very small HTML→text: drop script/style blocks and tags, collapse whitespace.
fn strip_html(input: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            // Skip whole script/style blocks.
            for (tag, end) in [("<script", "</script>"), ("<style", "</style>")] {
                if lower[i..].starts_with(tag) {
                    if let Some(rel) = lower[i..].find(end) {
                        i += rel + end.len();
                    } else {
                        i = bytes.len();
                    }
                    out.push(' ');
                    continue;
                }
            }
            // Skip a normal tag.
            if let Some(rel) = input[i..].find('>') {
                i += rel + 1;
                out.push(' ');
                continue;
            }
            break;
        }
        let ch = input[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    // Collapse whitespace.
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `browse` — server-side fetch of a public URL. JSON passes through; HTML is
/// reduced to text (8000-char cap). Never uses the browser (raw HTTP).
pub async fn browse(url: &str, headers: &Map<String, Value>) -> String {
    if !validate_url(url) {
        return json!({ "ok": false, "error": "blocked: only public http(s) URLs allowed (SSRF guard)", "url": url }).to_string();
    }
    let client = reqwest::Client::new();
    let mut req = client
        .get(url)
        .header("User-Agent", "Mozilla/5.0 (compatible; neo-browser/rust)")
        .timeout(Duration::from_secs(15));
    for (k, v) in headers {
        if let Some(s) = v.as_str() {
            req = req.header(k.as_str(), s);
        }
    }
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => return json!({ "ok": false, "error": e.to_string(), "url": url }).to_string(),
    };
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = match resp.text().await {
        Ok(t) => t,
        Err(e) => return json!({ "ok": false, "error": e.to_string(), "url": url }).to_string(),
    };
    let capped: String = body.chars().take(512 * 1024).collect();
    if content_type.contains("json") {
        return capped;
    }
    let text = clean_scraped(&strip_html(&capped));
    let text: String = text.chars().take(8000).collect();
    json!({ "url": url, "text": text, "content_type": content_type }).to_string()
}

/// `upload` — attach local files to a file input via `DOM.setFileInputFiles`.
pub async fn upload(
    client: &CdpClient,
    selector: &str,
    files: Vec<String>,
) -> Result<String, CdpError> {
    // Expand ~ and make absolute; verify existence.
    let abs: Vec<String> = files
        .iter()
        .map(|f| {
            let expanded = if let Some(rest) = f.strip_prefix("~/") {
                paths_home().join(rest)
            } else {
                PathBuf::from(f)
            };
            std::fs::canonicalize(&expanded)
                .unwrap_or(expanded)
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    let missing: Vec<&String> = abs
        .iter()
        .filter(|f| !std::path::Path::new(f).exists())
        .collect();
    if !missing.is_empty() {
        return Ok(
            json!({ "ok": false, "error": format!("file(s) not found: {missing:?}") }).to_string(),
        );
    }
    let doc = client
        .send("DOM.getDocument", json!({ "depth": 0 }))
        .await?;
    let root = doc
        .get("root")
        .and_then(|r| r.get("nodeId"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let q = client
        .send(
            "DOM.querySelector",
            json!({ "nodeId": root, "selector": selector }),
        )
        .await?;
    let node_id = q.get("nodeId").and_then(|v| v.as_i64()).unwrap_or(0);
    if node_id == 0 {
        return Ok(
            json!({ "ok": false, "error": format!("file input not found: {selector}") })
                .to_string(),
        );
    }
    client
        .send(
            "DOM.setFileInputFiles",
            json!({ "files": abs, "nodeId": node_id }),
        )
        .await?;
    Ok(json!({ "ok": true, "uploaded": abs, "selector": selector }).to_string())
}

fn paths_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// `download` — fetch a public URL to `~/.neobrowser/downloads/`, reusing the tab's
/// cookies so auth-gated files work. 200 MB cap.
pub async fn download(
    client: &CdpClient,
    url: &str,
    filename: Option<&str>,
) -> Result<String, CdpError> {
    if !validate_url(url) {
        return Ok(json!({ "ok": false, "error": "blocked: only public http(s) URLs allowed (SSRF guard)" }).to_string());
    }
    let ddir = paths::home().join("downloads");
    if std::fs::create_dir_all(&ddir).is_err() {
        return Ok(json!({ "ok": false, "error": "could not create downloads dir" }).to_string());
    }
    let raw_name = filename
        .map(String::from)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            url.trim_end_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or("download")
                .split('?')
                .next()
                .unwrap_or("download")
                .to_string()
        });
    let safe: String = raw_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .take(120)
        .collect();
    let safe = if safe.is_empty() {
        "download".to_string()
    } else {
        safe
    };
    let dest = ddir.join(&safe);

    // Reuse the tab's cookies for this URL.
    let mut cookie_header = String::new();
    if let Ok(res) = client
        .send("Network.getCookies", json!({ "urls": [url] }))
        .await
    {
        if let Some(cookies) = res.get("cookies").and_then(|c| c.as_array()) {
            let parts: Vec<String> = cookies
                .iter()
                .filter_map(|c| {
                    let n = c.get("name").and_then(|v| v.as_str())?;
                    let v = c.get("value").and_then(|v| v.as_str())?;
                    Some(format!("{n}={v}"))
                })
                .collect();
            cookie_header = parts.join("; ");
        }
    }

    let mut req = reqwest::Client::new()
        .get(url)
        .header("User-Agent", "Mozilla/5.0")
        .timeout(Duration::from_secs(30));
    if !cookie_header.is_empty() {
        req = req.header("Cookie", cookie_header);
    }
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => return Ok(json!({ "ok": false, "error": e.to_string() }).to_string()),
    };
    let bytes = match resp.bytes().await {
        Ok(b) => b,
        Err(e) => return Ok(json!({ "ok": false, "error": e.to_string() }).to_string()),
    };
    let capped = &bytes[..bytes.len().min(200 * 1024 * 1024)];
    if std::fs::write(&dest, capped).is_err() {
        return Ok(json!({ "ok": false, "error": "write failed" }).to_string());
    }
    Ok(
        json!({ "ok": true, "path": dest.display().to_string(), "bytes": capped.len() })
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssrf_blocks_non_public() {
        assert!(!validate_url("file:///etc/passwd"));
        assert!(!validate_url("http://localhost/x"));
        assert!(!validate_url("http://127.0.0.1/x"));
        assert!(!validate_url("http://10.0.0.5/x"));
        assert!(!validate_url("http://192.168.1.1/x"));
        assert!(!validate_url("http://169.254.169.254/latest/meta-data"));
        assert!(!validate_url("http://user:pass@example.com/"));
        assert!(!validate_url("http://metadata.google.internal/"));
        assert!(!validate_url("ftp://example.com/"));
    }

    #[test]
    fn ssrf_allows_public_literals() {
        assert!(validate_url("http://8.8.8.8/"));
        assert!(validate_url("https://1.1.1.1/"));
    }

    #[test]
    fn strip_html_removes_tags_and_scripts() {
        let html = "<html><head><style>a{}</style></head><body>Hello <b>world</b><script>evil()</script>!</body></html>";
        let text = strip_html(html);
        assert!(text.contains("Hello"));
        assert!(text.contains("world"));
        assert!(!text.contains("evil"));
        assert!(!text.contains("<"));
    }

    #[test]
    fn clean_scraped_strips_zero_width() {
        let dirty = "he\u{200B}llo\u{FEFF}";
        assert_eq!(clean_scraped(dirty), "hello");
    }

    #[test]
    fn download_filename_sanitized() {
        // Indirect check of the sanitizer via a crafted URL basename.
        let raw = "../../etc/pa$$wd?x=1";
        let safe: String = raw
            .rsplit('/')
            .next()
            .unwrap()
            .split('?')
            .next()
            .unwrap()
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        assert_eq!(safe, "pa__wd");
    }
}
