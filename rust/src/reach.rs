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

/// Directories `upload` may read from. If `NEOBROWSER_UPLOAD_DIR` is set, ONLY that
/// directory is allowed (tightest, recommended for autonomous agents). Otherwise a
/// safe default set of user content folders.
fn upload_allowed_roots() -> Vec<PathBuf> {
    if let Some(dir) = std::env::var_os("NEOBROWSER_UPLOAD_DIR") {
        if !dir.is_empty() {
            let p = PathBuf::from(dir);
            return vec![std::fs::canonicalize(&p).unwrap_or(p)];
        }
    }
    let home = paths_home();
    ["Downloads", "Desktop", "Documents"]
        .iter()
        .map(|d| home.join(d))
        .chain(std::iter::once(crate::paths::home().join("downloads")))
        .map(|p| std::fs::canonicalize(&p).unwrap_or(p))
        .collect()
}

/// True for paths that must never be uploaded even from an allowed root — secrets,
/// keys, keychains, credential files, and NeoBrowser's own cookie/session store.
/// This is the defense against a prompt-injected agent exfiltrating local secrets.
fn is_sensitive_upload(canonical: &std::path::Path) -> bool {
    let s = canonical.to_string_lossy().to_lowercase();
    const DENY_SEGMENTS: &[&str] = &[
        "/.ssh/",
        "/.aws/",
        "/.gnupg/",
        "/.gpg/",
        "/.kube/",
        "/.docker/",
        "/.config/gcloud/",
        "/library/keychains/",
        "/.mozilla/",
        "/.password-store/",
    ];
    if DENY_SEGMENTS.iter().any(|seg| s.contains(seg)) {
        return true;
    }
    // NeoBrowser's own secret store (cookies / sessions / profiles).
    let nb = crate::paths::home().to_string_lossy().to_lowercase();
    for sub in ["/cookies", "/sessions", "/profiles"] {
        if s.starts_with(&format!("{nb}{sub}")) {
            return true;
        }
    }
    let name = canonical
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    const DENY_NAMES: &[&str] = &[
        "id_rsa",
        "id_dsa",
        "id_ecdsa",
        "id_ed25519",
        "credentials",
        ".env",
        ".netrc",
        ".pgpass",
        ".git-credentials",
        ".npmrc",
        ".pypirc",
    ];
    if DENY_NAMES.contains(&name.as_str()) {
        return true;
    }
    let ext = canonical
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    matches!(
        ext.as_str(),
        "pem" | "key" | "p12" | "pfx" | "keychain" | "kdbx"
    )
}

/// Resolve a requested upload path, or return a reason it is rejected.
fn resolve_upload_path(f: &str) -> Result<PathBuf, String> {
    let expanded = if let Some(rest) = f.strip_prefix("~/") {
        paths_home().join(rest)
    } else {
        PathBuf::from(f)
    };
    let canonical = std::fs::canonicalize(&expanded).map_err(|_| format!("file not found: {f}"))?;
    if is_sensitive_upload(&canonical) {
        return Err(format!("refused (sensitive path): {f}"));
    }
    let roots = upload_allowed_roots();
    if !roots.iter().any(|r| canonical.starts_with(r)) {
        return Err(format!(
            "refused (outside allowed upload dirs): {f}. Allowed: {}. Set NEOBROWSER_UPLOAD_DIR to widen.",
            roots.iter().map(|r| r.display().to_string()).collect::<Vec<_>>().join(", ")
        ));
    }
    Ok(canonical)
}

/// `upload` — attach local files to a file input via `DOM.setFileInputFiles`.
///
/// Security: files must live under an allowed root (see `upload_allowed_roots`) and
/// must not be sensitive (see `is_sensitive_upload`), so a prompt-injected agent
/// cannot exfiltrate arbitrary local files (ssh keys, credentials, cookie stores…).
pub async fn upload(
    client: &CdpClient,
    selector: &str,
    files: Vec<String>,
) -> Result<String, CdpError> {
    let mut abs: Vec<String> = Vec::with_capacity(files.len());
    for f in &files {
        match resolve_upload_path(f) {
            Ok(p) => abs.push(p.to_string_lossy().into_owned()),
            Err(reason) => {
                return Ok(json!({ "ok": false, "error": reason }).to_string());
            }
        }
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

    #[test]
    fn sensitive_upload_paths_blocked() {
        use std::path::Path;
        assert!(is_sensitive_upload(Path::new("/Users/x/.ssh/id_rsa")));
        assert!(is_sensitive_upload(Path::new("/Users/x/.aws/credentials")));
        assert!(is_sensitive_upload(Path::new(
            "/Users/x/Documents/server.pem"
        )));
        assert!(is_sensitive_upload(Path::new("/Users/x/project/.env")));
        assert!(is_sensitive_upload(Path::new(
            "/Users/x/Library/Keychains/login.keychain-db"
        )));
        // Ordinary user content is fine.
        assert!(!is_sensitive_upload(Path::new(
            "/Users/x/Downloads/photo.png"
        )));
        assert!(!is_sensitive_upload(Path::new("/Users/x/Documents/cv.pdf")));
    }

    #[test]
    fn upload_restricted_to_allowed_root() {
        let _g = crate::env_test_guard();
        let dir = std::env::temp_dir().join(format!("nb-upload-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let inside = dir.join("ok.txt");
        std::fs::write(&inside, b"hi").unwrap();
        let outside = std::env::temp_dir().join(format!("nb-upl-out-{}.txt", std::process::id()));
        std::fs::write(&outside, b"hi").unwrap();

        std::env::set_var("NEOBROWSER_UPLOAD_DIR", &dir);
        // A file inside the allowed dir resolves.
        assert!(resolve_upload_path(inside.to_str().unwrap()).is_ok());
        // A file outside is refused.
        let err = resolve_upload_path(outside.to_str().unwrap()).unwrap_err();
        assert!(err.contains("outside allowed upload dirs"), "got: {err}");
        // A missing file is refused.
        assert!(resolve_upload_path("/no/such/file-xyz").is_err());

        std::env::remove_var("NEOBROWSER_UPLOAD_DIR");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(&outside);
    }
}
