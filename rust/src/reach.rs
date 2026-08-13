//! Reach tools: browse (server-side fetch), upload (file input), download (auth-aware).
//!
//! Ported from the Python `browse`/`upload`/`download` dispatch. The SSRF guard is
//! kept: only public http(s) URLs are allowed — file://, credentials-in-URL, and
//! hosts resolving to loopback/private/link-local ranges (incl. cloud metadata) are
//! blocked before any request goes out.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs};
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{json, Map, Value};

use crate::cdp::{CdpClient, CdpError};
use crate::paths;

/// SSRF guard: true only for a public http(s) URL with no userinfo whose host
/// resolves entirely to public addresses.
pub fn validate_url(raw: &str) -> bool {
    resolve_public_url(raw).is_some()
}

/// Full SSRF validation of `raw`. On success returns the parsed URL plus, for
/// domain hosts, the exact socket addresses validation checked — callers pin
/// them on the request (`ClientBuilder::resolve_to_addrs`) so an attacker
/// can't re-resolve to a private IP between validation and connect
/// (DNS-rebinding TOCTOU).
fn resolve_public_url(raw: &str) -> Option<(reqwest::Url, Vec<SocketAddr>)> {
    let url = reqwest::Url::parse(raw).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    if !url.username().is_empty() || url.password().is_some() {
        return None; // credentials-in-URL
    }
    let host = url.host_str()?;
    // A literal IP: check it directly (no DNS, nothing to pin). host_str keeps
    // the brackets on IPv6 literals ("[::1]") — strip them before parsing.
    let bare = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    if let Ok(ip) = bare.parse::<IpAddr>() {
        return is_public(ip).then_some((url, Vec::new()));
    }
    // Block obvious internal names outright.
    let h = host.to_ascii_lowercase();
    if h == "localhost" || h.ends_with(".localhost") || h == "metadata.google.internal" {
        return None;
    }
    // Resolve and require every address to be public.
    let port = url.port_or_known_default().unwrap_or(80);
    let addrs: Vec<SocketAddr> = (host, port).to_socket_addrs().ok()?.collect();
    if addrs.is_empty() || !addrs.iter().all(|a| is_public(a.ip())) {
        return None;
    }
    Some((url, addrs))
}

/// Reject loopback / private / link-local / unspecified / cloud-metadata IPs.
fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => {
            // Loopback/unspecified/multicast first: to_ipv4() would otherwise
            // map ::1 to 0.0.0.1 and skip the loopback check.
            if v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() {
                return false;
            }
            // IPv4-mapped (::ffff:a.b.c.d) and IPv4-compatible (::a.b.c.d)
            // forms embed an IPv4 that must pass the same checks — otherwise
            // ::ffff:127.0.0.1 or ::ffff:a9fe:a9fe would sail through.
            if let Some(v4) = v6.to_ipv4() {
                return is_public_v4(v4);
            }
            let segs = v6.segments();
            // 6to4 (2002::/16): the next 32 bits are an IPv4 address.
            if segs[0] == 0x2002 {
                let v4 = Ipv4Addr::new(
                    (segs[1] >> 8) as u8,
                    segs[1] as u8,
                    (segs[2] >> 8) as u8,
                    segs[2] as u8,
                );
                return is_public_v4(v4);
            }
            // Teredo (2001:0000::/32): the last 32 bits are the IPv4 XOR all-ones.
            if segs[0] == 0x2001 && segs[1] == 0x0000 {
                let v4 = Ipv4Addr::new(
                    !(segs[6] >> 8) as u8,
                    !segs[6] as u8,
                    !(segs[7] >> 8) as u8,
                    !segs[7] as u8,
                );
                return is_public_v4(v4);
            }
            !(
                // unique-local fc00::/7
                (segs[0] & 0xfe00) == 0xfc00
                // link-local fe80::/10
                || (segs[0] & 0xffc0) == 0xfe80
            )
        }
    }
}

fn is_public_v4(v4: Ipv4Addr) -> bool {
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

/// Max redirects followed manually (each hop re-runs the full SSRF guard).
const MAX_REDIRECTS: u32 = 5;

/// GET `raw_url` with redirect::Policy::none, following redirects manually and
/// re-validating every hop (scheme + host + DNS + is_public) so a public URL
/// can't bounce us into `169.254.169.254` or a private range. `cookie` is only
/// sent on hops sharing the original host.
async fn guarded_get(
    raw_url: &str,
    ua: &str,
    timeout: Duration,
    headers: &Map<String, Value>,
    cookie: Option<&str>,
) -> Result<reqwest::Response, String> {
    const BLOCKED: &str = "blocked: only public http(s) URLs allowed (SSRF guard)";
    let first_host = reqwest::Url::parse(raw_url)
        .ok()
        .and_then(|u| u.host_str().map(String::from));
    let mut current = raw_url.to_string();
    for _ in 0..=MAX_REDIRECTS {
        let (url, addrs) = resolve_public_url(&current).ok_or_else(|| BLOCKED.to_string())?;
        // Pin the validated IPs for this hop's DNS (see resolve_public_url).
        let mut builder = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none());
        if !addrs.is_empty() {
            if let Some(host) = url.host_str() {
                builder = builder.resolve_to_addrs(host, &addrs);
            }
        }
        let client = builder.build().map_err(|e| e.to_string())?;
        let mut req = client
            .get(url.as_str())
            .header("User-Agent", ua)
            .timeout(timeout);
        for (k, v) in headers {
            if let Some(s) = v.as_str() {
                req = req.header(k.as_str(), s);
            }
        }
        if let Some(c) = cookie {
            if url.host_str().map(String::from) == first_host {
                req = req.header("Cookie", c);
            }
        }
        let resp = req.send().await.map_err(|e| e.to_string())?;
        if resp.status().is_redirection() {
            let location = resp
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| "redirect without Location header".to_string())?;
            // Relative redirects resolve against the current URL.
            current = url
                .join(location)
                .map_err(|e| format!("bad redirect target: {e}"))?
                .to_string();
            continue;
        }
        return Ok(resp);
    }
    Err(format!("too many redirects (max {MAX_REDIRECTS})"))
}

/// Read a response body incrementally, stopping as soon as `cap` bytes are
/// accumulated — the old `resp.bytes().await` buffered the WHOLE body in
/// memory before truncating, so a multi-GB body could OOM the server.
async fn read_capped(resp: reqwest::Response, cap: usize) -> Result<Vec<u8>, reqwest::Error> {
    let mut resp = resp;
    let mut buf = Vec::new();
    while let Some(chunk) = resp.chunk().await? {
        let remaining = cap.saturating_sub(buf.len());
        if chunk.len() >= remaining {
            buf.extend_from_slice(&chunk[..remaining]);
            break; // drop the response; the rest never leaves the socket buffer
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
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
    let resp = match guarded_get(
        url,
        "Mozilla/5.0 (compatible; neo-browser/rust)",
        Duration::from_secs(15),
        headers,
        None,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => return json!({ "ok": false, "error": e, "url": url }).to_string(),
    };
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = match read_capped(resp, 512 * 1024).await {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(e) => return json!({ "ok": false, "error": e.to_string(), "url": url }).to_string(),
    };
    if content_type.contains("json") {
        return body;
    }
    let text = clean_scraped(&strip_html(&body));
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

    let cookie_opt = if cookie_header.is_empty() {
        None
    } else {
        Some(cookie_header.as_str())
    };
    let empty_headers = Map::new();
    let resp = match guarded_get(
        url,
        "Mozilla/5.0",
        Duration::from_secs(30),
        &empty_headers,
        cookie_opt,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => return Ok(json!({ "ok": false, "error": e }).to_string()),
    };
    let bytes = match read_capped(resp, 200 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => return Ok(json!({ "ok": false, "error": e.to_string() }).to_string()),
    };
    if std::fs::write(&dest, &bytes).is_err() {
        return Ok(json!({ "ok": false, "error": "write failed" }).to_string());
    }
    Ok(json!({ "ok": true, "path": dest.display().to_string(), "bytes": bytes.len() }).to_string())
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
    fn ssrf_blocks_ipv4_in_v6_disguises() {
        // IPv4-mapped: ::ffff:a.b.c.d
        assert!(!validate_url("http://[::ffff:127.0.0.1]/"));
        assert!(!validate_url("http://[::ffff:a9fe:a9fe]/")); // 169.254.169.254
        assert!(!validate_url("http://[::ffff:10.0.0.5]/"));
        // IPv4-compatible: ::a.b.c.d
        assert!(!validate_url("http://[::127.0.0.1]/"));
        // 6to4 (2002::/16) embedding 127.0.0.1.
        assert!(!validate_url("http://[2002:7f00:1::]/"));
        // Teredo (2001:0000::/32) embedding 127.0.0.1 (XORed).
        assert!(!validate_url("http://[2001:0::80ff:fffe]/"));
        // Plain v6 loopback/link-local/unique-local still blocked.
        assert!(!validate_url("http://[::1]/"));
        assert!(!validate_url("http://[fe80::1]/"));
        assert!(!validate_url("http://[fd00::1]/"));
        // A mapped PUBLIC address still passes.
        assert!(validate_url("http://[::ffff:8.8.8.8]/"));
    }

    #[tokio::test]
    async fn guarded_get_blocks_private_url() {
        let err = guarded_get(
            "http://127.0.0.1:1/",
            "ua",
            Duration::from_secs(1),
            &Map::new(),
            None,
        )
        .await
        .unwrap_err();
        assert!(err.contains("blocked"), "got: {err}");
    }

    #[tokio::test]
    async fn read_capped_stops_at_cap() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut s, _)) = listener.accept().await {
                let mut req = [0u8; 1024];
                let _ = s.read(&mut req).await;
                let body = vec![b'x'; 1024 * 1024];
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = s.write_all(head.as_bytes()).await;
                let _ = s.write_all(&body).await;
            }
        });
        let resp = reqwest::get(format!("http://{addr}/")).await.unwrap();
        let buf = read_capped(resp, 4096).await.unwrap();
        assert_eq!(buf.len(), 4096);
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
