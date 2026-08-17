//! Deciding whether a request's `Origin` may talk to this server.
//!
//! This is the check that keeps a web page in the user's browser from driving their browser.
//! A page cannot set most headers cross-origin, but it *can* send a simple request to
//! localhost, so the Origin comparison is the boundary — and it compares the parsed host
//! exactly. An earlier version used `starts_with("http://localhost")`, which happily accepted
//! `http://localhost.evil.test`: a domain an attacker can register, that resolves wherever
//! they like, and whose Origin passes a prefix test.

//! MCP over Streamable HTTP, with authentication, origin validation and per-session
//! isolation.
//!
//! stdio remains the primary local transport and nothing here changes it. This exists
//! for the cases stdio cannot serve: a container, a remote dev box, several clients
//! against one host.
//!
//! It is off unless a port is configured, and when on it enforces three things the MCP
//! specification is explicit about, each of which is a real attack rather than a
//! checkbox:
//!
//! 1. **Authentication.** A bearer token on every request. Without it, anything that can
//!    reach the port drives a browser holding the user's sessions.
//! 2. **Origin validation.** A browser page can POST to `127.0.0.1` — and with DNS
//!    rebinding, a remote page can reach a LAN-bound port too. The `Origin` header is
//!    what distinguishes a real client from a page, and an unexpected one is rejected.
//! 3. **Session isolation.** Each `Mcp-Session-Id` gets its own [`Browser`], hence its
//!    own Chrome profile and its own cookies. Sharing one browser between callers would
//!    hand session A's logged-in state to session B.
//!
//! Binding defaults to loopback. A non-loopback bind is possible but requires an
//! explicit opt-in, because exposing this on a LAN interface is a materially different
//! decision from running it locally.

/// Is this `Origin` acceptable?
///
/// Absent is fine: a native MCP client is not a browser and sends none. A present one
/// must be loopback — that is what stops a web page (or a DNS-rebound remote page) from
/// driving the transport, since a page cannot forge its own Origin.
pub fn origin_allowed(origin: Option<&str>) -> bool {
    let Some(origin) = origin else { return true };
    let o = origin.trim().to_ascii_lowercase();
    if o == "null" {
        // A `null` origin comes from a sandboxed iframe or a `file://` page. Not a
        // legitimate MCP client, and exactly the shape an attacker would arrive with.
        return false;
    }
    // Parsed and compared by exact host, not by prefix. A prefix check accepts
    // `http://localhost.evil.test`, which is an attacker-controlled domain that merely
    // begins with the right characters — the same near-miss the policy engine's suffix
    // matching has to defend against.
    let Ok(url) = reqwest::Url::parse(&o) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    matches!(
        url.host_str(),
        Some("127.0.0.1") | Some("localhost") | Some("[::1]") | Some("::1")
    )
}
