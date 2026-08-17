//! Server-side fetching: redirect following, credential scoping, and HTML reduction.
//!
//! The credential-scoping rule lives here and is the reason this file exists separately:
//! a cookie or an auth header must not survive a redirect off the origin the caller
//! asked for, including an `https` → `http` downgrade on the same host.

use std::time::Duration;

use serde_json::{json, Map, Value};

use super::ssrf::resolve_public_url;

/// Max redirects followed manually (each hop re-runs the full SSRF guard).
const MAX_REDIRECTS: u32 = 5;

/// Caller-supplied headers that stay on a cross-origin redirect.
///
/// Deliberately an allowlist, not a blocklist of "sensitive" names. A blocklist
/// has to guess every way a secret can be spelled — `Authorization`, `Cookie`,
/// `X-Api-Key`, `X-Acme-Session`, … — and a header it has never heard of gets
/// forwarded to whatever host the redirect names. Reversing the default means an
/// unknown header is treated as a possible secret, which is what it usually is:
/// these headers are set by a model or a config file precisely to authenticate.
/// What remains here is content negotiation, which carries nothing private.
const CROSS_ORIGIN_SAFE_HEADERS: &[&str] =
    &["accept", "accept-charset", "accept-language", "user-agent"];

/// Scheme + host + port, the unit credentials are scoped to. Compared as a whole
/// so an `https -> http` downgrade on the *same* host still counts as a different
/// origin — otherwise a redirect could replay a secure-only cookie in plaintext.
fn origin_of(url: &reqwest::Url) -> (String, String, u16) {
    (
        url.scheme().to_ascii_lowercase(),
        url.host_str().unwrap_or_default().to_ascii_lowercase(),
        url.port_or_known_default().unwrap_or(0),
    )
}

/// Is `name` safe to forward once we have left the original origin?
pub(super) fn safe_cross_origin(name: &str) -> bool {
    let lower = name.trim().to_ascii_lowercase();
    CROSS_ORIGIN_SAFE_HEADERS.contains(&lower.as_str())
}

/// Tracks whether a redirect chain still sits on the origin the caller aimed at,
/// which is the only origin its credentials were meant for.
///
/// Split out of the fetch loop so the rule can be tested exhaustively without a
/// network: the SSRF guard rejects loopback, so a local test server could never
/// exercise this path. Leaving the origin is one-way — see [`Self::visit`].
#[derive(Debug)]
pub(super) struct CredentialScope {
    first: Option<(String, String, u16)>,
    on_first_origin: bool,
}

impl CredentialScope {
    pub(super) fn new(raw_url: &str) -> Self {
        Self {
            // An unparseable start URL yields None, which never equals a parsed
            // hop's origin, so credentials are withheld. Fail closed.
            first: reqwest::Url::parse(raw_url).ok().map(|u| origin_of(&u)),
            on_first_origin: true,
        }
    }

    /// Record arrival at `url`. Once the chain has left the caller's origin it
    /// stays "off origin" even if a later hop points back: by then the URL we
    /// would be returning to was chosen by the off-origin host, not the caller.
    pub(super) fn visit(&mut self, url: &reqwest::Url) {
        if self.on_first_origin && self.first.as_ref() != Some(&origin_of(url)) {
            self.on_first_origin = false;
        }
    }

    /// May credentials (cookies, auth headers) go out on the current hop?
    pub(super) fn allows_credentials(&self) -> bool {
        self.on_first_origin && self.first.is_some()
    }
}

/// GET `raw_url` with redirect::Policy::none, following redirects manually and
/// re-validating every hop (scheme + host + DNS + is_public) so a public URL
/// can't bounce us into `169.254.169.254` or a private range.
///
/// Credentials are scoped to the origin the caller aimed at: the `cookie` and any
/// caller header outside [`CROSS_ORIGIN_SAFE_HEADERS`] are sent only while the
/// chain has not left that origin. Leaving it is sticky — a chain that comes back
/// (`a.com -> evil.com -> a.com`) does not regain them, because by then the URL
/// we are "returning" to was chosen by evil.com, not by the caller.
///
/// Returns the response plus the names of any headers withheld, so the caller can
/// surface that rather than silently changing what it sent.
pub(super) async fn guarded_get(
    raw_url: &str,
    ua: &str,
    timeout: Duration,
    headers: &Map<String, Value>,
    cookie: Option<&str>,
) -> Result<(reqwest::Response, Vec<String>), String> {
    const BLOCKED: &str = "blocked: only public http(s) URLs allowed (SSRF guard)";
    let mut scope = CredentialScope::new(raw_url);
    let mut withheld: Vec<String> = Vec::new();
    let mut current = raw_url.to_string();
    for _ in 0..=MAX_REDIRECTS {
        let (url, addrs) = resolve_public_url(&current).ok_or_else(|| BLOCKED.to_string())?;
        scope.visit(&url);
        let on_first_origin = scope.allows_credentials();
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
                if on_first_origin || safe_cross_origin(k) {
                    req = req.header(k.as_str(), s);
                } else if !withheld.iter().any(|w| w.eq_ignore_ascii_case(k)) {
                    withheld.push(k.clone());
                }
            }
        }
        if let Some(c) = cookie {
            if on_first_origin {
                req = req.header("Cookie", c);
            } else if !withheld.iter().any(|w| w.eq_ignore_ascii_case("cookie")) {
                withheld.push("Cookie".to_string());
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
        return Ok((resp, withheld));
    }
    Err(format!("too many redirects (max {MAX_REDIRECTS})"))
}

/// Read a response body incrementally, stopping as soon as `cap` bytes are
/// accumulated — the old `resp.bytes().await` buffered the WHOLE body in
/// memory before truncating, so a multi-GB body could OOM the server.
pub(super) async fn read_capped(
    resp: reqwest::Response,
    cap: usize,
) -> Result<Vec<u8>, reqwest::Error> {
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
pub(super) fn clean_scraped(s: &str) -> String {
    s.chars()
        .filter(|c| {
            !matches!(*c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{206F}' | '\u{FEFF}')
                && (!c.is_control() || *c == '\n' || *c == '\t')
        })
        .collect()
}

/// Very small HTML→text: drop script/style blocks and tags, collapse whitespace.
pub(super) fn strip_html(input: &str) -> String {
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
    let (resp, withheld) = match guarded_get(
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
    // Logged as well as returned: the JSON passthrough below hands back the
    // upstream body verbatim, so there is no envelope of ours to carry the notice.
    if !withheld.is_empty() {
        tracing::warn!(
            headers = %withheld.join(", "),
            "withheld caller headers from a cross-origin redirect target"
        );
    }
    if content_type.contains("json") {
        return body;
    }
    let text = clean_scraped(&strip_html(&body));
    let text: String = text.chars().take(8000).collect();
    // Fenced and labelled: `browse` fetches arbitrary third-party HTML, which is the
    // most direct route for a page to try instructing the model.
    let wrapped = crate::untrusted::wrap(url, &text);
    let mut out = json!({
        "url": url,
        "content_type": content_type,
        "trust": wrapped["trust"].clone(),
        "text": wrapped["content"].clone(),
    });
    if let Some(inj) = wrapped.get("injection") {
        out["injection"] = inj.clone();
    }
    if let Some(w) = wrapped.get("warnings").and_then(Value::as_array) {
        out["warnings"] = json!(w.clone());
    }
    if !withheld.is_empty() {
        // Append rather than assign: an injection warning may already be here, and
        // overwriting it would hide the more serious of the two.
        let mut warns = out
            .get("warnings")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        warns.push(json!(format!(
            "redirect left the requested origin; these headers were not forwarded: {}",
            withheld.join(", ")
        )));
        out["warnings"] = Value::Array(warns);
    }
    out.to_string()
}
