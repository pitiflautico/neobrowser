//! The guarded GET: bounded redirects, bounded body, bounded time.
//!
//! Every bound here exists because the other side is untrusted. An unbounded body is a memory
//! exhaustion primitive handed to whatever page was asked for, and an unbounded redirect chain
//! is the same thing for time.

//! Server-side fetching: redirect following, credential scoping, and HTML reduction.
//!
//! The credential-scoping rule lives here and is the reason this file exists separately:
//! a cookie or an auth header must not survive a redirect off the origin the caller
//! asked for, including an `https` → `http` downgrade on the same host.

use std::time::Duration;

use serde_json::{Map, Value};

use super::super::ssrf::resolve_public_url;
use super::credentials::{safe_cross_origin, CredentialScope};
use super::MAX_REDIRECTS;

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
pub(in crate::reach) async fn guarded_get(
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
pub(in crate::reach) async fn read_capped(
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
