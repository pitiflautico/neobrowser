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

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

use crate::tools::{Registry, ToolCtx};

/// Maximum request body. A JSON-RPC call is small; unbounded reads are how a client
/// makes the server allocate until it dies.
const MAX_BODY: usize = 8 * 1024 * 1024;

/// Sessions idle longer than this are reaped, releasing their Chrome.
///
/// Necessary rather than tidy: every session owns a browser, so a client that
/// disconnects without cleanup would otherwise leak a Chrome per connection until the
/// host runs out of memory.
const SESSION_IDLE_SECS: u64 = 900;

/// One client session: its own browser, trace and policy.
struct Session {
    ctx: ToolCtx,
    last_seen: std::time::Instant,
}

pub struct HttpTransport {
    bind: String,
    port: u16,
    token: String,
    registry: Arc<Registry>,
    sessions: Mutex<HashMap<String, Session>>,
}

impl HttpTransport {
    pub fn new(bind: String, port: u16, registry: Arc<Registry>) -> Arc<Self> {
        Arc::new(Self {
            bind,
            port,
            token: crate::vault::random_token_hex(),
            registry,
            sessions: Mutex::new(HashMap::new()),
        })
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    /// Constant-time bearer comparison. Same reasoning as the bridge: a variable-time
    /// compare on a secret is a latent timing oracle.
    fn authorized(&self, header: Option<&str>) -> bool {
        let Some(value) = header else { return false };
        let presented = value
            .trim()
            .strip_prefix("Bearer ")
            .or_else(|| value.trim().strip_prefix("bearer "))
            .unwrap_or(value.trim());
        let (a, b) = (self.token.as_bytes(), presented.as_bytes());
        if a.len() != b.len() {
            return false;
        }
        a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
    }

    /// Get or create the session for `id`, and reap idle ones.
    async fn session_ctx(&self, id: &str) -> ToolCtx {
        let mut sessions = self.sessions.lock().await;

        // Reap first, so a long-running server does not accumulate browsers. Collected
        // then dropped outside the retain so each browser's shutdown is awaited.
        let stale: Vec<String> = sessions
            .iter()
            .filter(|(_, s)| s.last_seen.elapsed().as_secs() > SESSION_IDLE_SECS)
            .map(|(k, _)| k.clone())
            .collect();
        for key in stale {
            if let Some(s) = sessions.remove(&key) {
                tracing::info!(session = %key, "reaping an idle HTTP session");
                s.ctx.browser.shutdown().await;
            }
        }

        if let Some(s) = sessions.get_mut(id) {
            s.last_seen = std::time::Instant::now();
            return s.ctx.clone();
        }

        // A fresh session: its own browser on its own profile, so nothing is shared
        // with any other caller.
        let ctx = ToolCtx {
            browser: Arc::new(crate::browser::Browser::with_profile(id)),
            registry: self.registry.clone(),
            policy: Arc::new(crate::policy::Policy::from_env()),
            trace: Arc::new(crate::trace::Trace::new(format!("trace_http_{id}"))),
            // The bridge drives the user's own browser and is a single-user local
            // affair; exposing it through a network transport would mean one HTTP
            // client could drive another user's tabs.
            bridge: None,
        };
        tracing::info!(session = %id, "new HTTP session with an isolated profile");
        sessions.insert(
            id.to_string(),
            Session {
                ctx: ctx.clone(),
                last_seen: std::time::Instant::now(),
            },
        );
        ctx
    }

    async fn end_session(&self, id: &str) -> bool {
        let removed = self.sessions.lock().await.remove(id);
        match removed {
            Some(s) => {
                s.ctx.browser.shutdown().await;
                true
            }
            None => false,
        }
    }

    pub async fn session_count(&self) -> usize {
        self.sessions.lock().await.len()
    }
}

/// Is this `Origin` acceptable?
///
/// Absent is fine: a native MCP client is not a browser and sends none. A present one
/// must be loopback — that is what stops a web page (or a DNS-rebound remote page) from
/// driving the transport, since a page cannot forge its own Origin.
fn origin_allowed(origin: Option<&str>) -> bool {
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

/// Run the HTTP transport until the process ends.
pub async fn serve(transport: Arc<HttpTransport>) -> std::io::Result<()> {
    let listener = TcpListener::bind((transport.bind.as_str(), transport.port)).await?;
    let loopback = transport.bind.starts_with("127.") || transport.bind == "::1";
    if !loopback {
        // Stated loudly rather than logged at debug: a non-loopback bind means anything
        // that can route to this host can attempt to drive a browser here.
        tracing::warn!(
            bind = %transport.bind,
            port = transport.port,
            "MCP HTTP transport is bound to a NON-LOOPBACK address. Anything that can \
             reach this host can attempt to authenticate. Put it behind a TLS proxy and \
             treat the bearer token as a production credential."
        );
    }
    tracing::info!(
        bind = %transport.bind,
        port = transport.port,
        "MCP HTTP transport listening; run `neobrowser http token` for the bearer token"
    );

    loop {
        let (mut socket, peer) = match listener.accept().await {
            Ok(pair) => pair,
            Err(e) => {
                tracing::warn!(error = %e, "http accept failed");
                continue;
            }
        };
        let transport = transport.clone();
        tokio::spawn(async move {
            if let Err(e) = handle(&transport, &mut socket).await {
                tracing::debug!(peer = %peer, error = %e, "http connection ended");
            }
        });
    }
}

async fn handle(
    transport: &HttpTransport,
    socket: &mut tokio::net::TcpStream,
) -> std::io::Result<()> {
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 8192];

    let header_end = loop {
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break p;
        }
        let n = socket.read(&mut chunk).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > MAX_BODY {
            return reply(socket, 413, None, &json!({ "error": "request too large" })).await;
        }
    };

    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();

    let header = |name: &str| -> Option<String> {
        head.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim()
                .eq_ignore_ascii_case(name)
                .then(|| v.trim().to_string())
        })
    };

    // Origin first: a rejected origin must not even reach the auth comparison, so a
    // hostile page learns nothing from timing.
    if !origin_allowed(header("origin").as_deref()) {
        return reply(
            socket,
            403,
            None,
            &json!({
                "error": "origin not allowed",
                "hint": "the MCP HTTP transport only accepts loopback origins, or none at \
                         all. A browser page cannot be a client here",
            }),
        )
        .await;
    }

    if !transport.authorized(header("authorization").as_deref()) {
        return reply(
            socket,
            401,
            None,
            &json!({
                "error": "missing or invalid bearer token",
                "hint": "send `Authorization: Bearer <token>`; get the value from \
                         `neobrowser http token`",
            }),
        )
        .await;
    }

    if path != "/mcp" {
        return reply(
            socket,
            404,
            None,
            &json!({ "error": "no such route; use /mcp" }),
        )
        .await;
    }

    // A session id is required so isolation is explicit. Generating one silently would
    // mean a client that forgot the header quietly got a brand-new browser per request.
    let session_id = match header("mcp-session-id") {
        Some(id) if !id.trim().is_empty() => id.trim().to_string(),
        _ => {
            return reply(
                socket,
                400,
                None,
                &json!({
                    "error": "missing Mcp-Session-Id header",
                    "hint": "choose any stable opaque id per client session; each one gets \
                             its own isolated browser profile",
                }),
            )
            .await
        }
    };

    // DELETE ends a session, per the Streamable HTTP spec.
    if method == "DELETE" {
        let existed = transport.end_session(&session_id).await;
        return reply(socket, 200, Some(&session_id), &json!({ "ended": existed })).await;
    }
    if method != "POST" {
        return reply(
            socket,
            405,
            Some(&session_id),
            &json!({ "error": "use POST to send JSON-RPC, or DELETE to end the session" }),
        )
        .await;
    }

    let content_length = header("content-length")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    if content_length > MAX_BODY {
        return reply(socket, 413, None, &json!({ "error": "body too large" })).await;
    }
    let body_start = header_end + 4;
    while buf.len() < body_start + content_length {
        let n = socket.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let body = &buf[body_start.min(buf.len())..];

    let request: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(e) => {
            return reply(
                socket,
                200,
                Some(&session_id),
                &json!({
                    "jsonrpc": "2.0",
                    "id": Value::Null,
                    "error": { "code": -32700, "message": format!("Parse error: {e}") },
                }),
            )
            .await
        }
    };

    let ctx = transport.session_ctx(&session_id).await;
    // Reuses the exact same dispatch as stdio, so the two transports cannot drift in
    // behaviour — policy, tracing and validation all apply identically.
    match crate::mcp::handle_request(&transport.registry, &ctx, &request).await {
        Some(response) => reply(socket, 200, Some(&session_id), &response).await,
        // A notification has no response. 202 Accepted is what the spec calls for.
        None => reply_empty(socket, 202, Some(&session_id)).await,
    }
}

async fn reply(
    socket: &mut tokio::net::TcpStream,
    status: u16,
    session: Option<&str>,
    body: &Value,
) -> std::io::Result<()> {
    let text = body.to_string();
    let session_header = session
        .map(|s| format!("mcp-session-id: {s}\r\n"))
        .unwrap_or_default();
    let response = format!(
        "HTTP/1.1 {status} {}\r\n\
         content-type: application/json\r\n\
         {session_header}\
         content-length: {}\r\n\
         connection: close\r\n\
         \r\n{text}",
        reason(status),
        text.len()
    );
    socket.write_all(response.as_bytes()).await?;
    socket.flush().await
}

async fn reply_empty(
    socket: &mut tokio::net::TcpStream,
    status: u16,
    session: Option<&str>,
) -> std::io::Result<()> {
    let session_header = session
        .map(|s| format!("mcp-session-id: {s}\r\n"))
        .unwrap_or_default();
    let response = format!(
        "HTTP/1.1 {status} {}\r\n{session_header}content-length: 0\r\nconnection: close\r\n\r\n",
        reason(status)
    );
    socket.write_all(response.as_bytes()).await?;
    socket.flush().await
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        _ => "Error",
    }
}

/// Where the HTTP bearer token is written, so `neobrowser http token` can print it.
pub fn token_path() -> std::path::PathBuf {
    crate::paths::home().join("http.token")
}

pub fn read_token_file() -> std::io::Result<String> {
    Ok(std::fs::read_to_string(token_path())?.trim().to_string())
}

/// The transport's configuration, or `None` when it is off (the default).
pub fn configured() -> Option<(String, u16)> {
    let port = std::env::var("NEOBROWSER_HTTP_PORT")
        .ok()
        .and_then(|v| v.trim().parse::<u16>().ok())
        .filter(|p| *p >= 1024)?;
    // Loopback unless explicitly overridden: binding a browser-driving API to every
    // interface should never be something that happens by default.
    let bind = std::env::var("NEOBROWSER_HTTP_BIND")
        .ok()
        .filter(|b| !b.trim().is_empty())
        .unwrap_or_else(|| "127.0.0.1".to_string());
    Some((bind, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transport() -> Arc<HttpTransport> {
        HttpTransport::new(
            "127.0.0.1".into(),
            0,
            Arc::new(crate::tool_impls::build_registry()),
        )
    }

    #[test]
    fn bearer_tokens_are_long_random_and_constant_time_compared() {
        let a = transport();
        let b = transport();
        assert_ne!(a.token(), b.token());
        assert!(a.token().len() >= 64);
        assert!(a.authorized(Some(&format!("Bearer {}", a.token()))));
        // A bare token without the scheme is accepted too, since clients differ.
        assert!(a.authorized(Some(a.token())));
        assert!(!a.authorized(Some(&format!("Bearer {}", b.token()))));
        assert!(!a.authorized(None));
        assert!(!a.authorized(Some("Bearer short")));
    }

    /// A browser page cannot forge `Origin`, so validating it is what stops a page —
    /// or a DNS-rebound remote page — from driving this transport.
    #[test]
    fn only_loopback_origins_are_accepted() {
        assert!(origin_allowed(None), "a native client sends no Origin");
        assert!(origin_allowed(Some("http://127.0.0.1:3000")));
        assert!(origin_allowed(Some("http://localhost:5173")));
        assert!(origin_allowed(Some("http://[::1]:8080")));

        assert!(!origin_allowed(Some("https://evil.test")));
        assert!(!origin_allowed(Some("http://attacker.example.com")));
        // A sandboxed iframe or file:// page reports null — not a legitimate client.
        assert!(!origin_allowed(Some("null")));
        // The classic near-miss: a host that merely starts with the right characters.
        // A prefix check accepts these; only an exact host comparison rejects them.
        assert!(!origin_allowed(Some("http://localhost.evil.test")));
        assert!(!origin_allowed(Some("http://127.0.0.1.evil.test")));
        assert!(!origin_allowed(Some("http://localhostx")));
        // A non-http scheme is not a browser origin we should ever trust.
        assert!(!origin_allowed(Some("file://localhost")));
        // Unparseable garbage fails closed.
        assert!(!origin_allowed(Some("not a url at all")));
    }

    #[tokio::test]
    async fn each_session_id_gets_its_own_isolated_browser() {
        let t = transport();
        let a = t.session_ctx("client-a").await;
        let b = t.session_ctx("client-b").await;
        assert_eq!(t.session_count().await, 2);
        // Separate browser instances: sharing one would hand session A's cookies to B.
        assert!(
            !Arc::ptr_eq(&a.browser, &b.browser),
            "sessions must not share a browser"
        );
        // Separate traces, so one client's timeline is not polluted by another's.
        assert_ne!(a.trace.trace_id(), b.trace.trace_id());

        // The same id returns the same session rather than a fresh browser per request.
        let a2 = t.session_ctx("client-a").await;
        assert!(Arc::ptr_eq(&a.browser, &a2.browser));
        assert_eq!(t.session_count().await, 2);
    }

    #[tokio::test]
    async fn a_session_can_be_ended_and_ending_is_idempotent() {
        let t = transport();
        t.session_ctx("gone").await;
        assert!(t.end_session("gone").await, "first end reports it existed");
        assert!(
            !t.end_session("gone").await,
            "second end reports it did not"
        );
        assert_eq!(t.session_count().await, 0);
    }

    /// A session id arrives over the network and becomes a directory name. An
    /// unvalidated one would point Chrome's user-data dir wherever the caller likes.
    #[test]
    fn hostile_session_ids_cannot_escape_the_profiles_directory() {
        let base = crate::paths::profiles_base();
        for hostile in [
            "../../etc",
            "../../../.ssh",
            "..",
            ".",
            "/absolute/path",
            "with/slash",
            "",
            "....//....//",
        ] {
            let name = crate::paths::sanitize_profile_name(hostile);
            let dir = base.join(&name);
            assert!(
                dir.starts_with(&base),
                "{hostile:?} -> {name:?} escaped to {dir:?}"
            );
            assert!(
                !dir.components().any(|c| matches!(
                    c,
                    std::path::Component::ParentDir | std::path::Component::CurDir
                )),
                "{hostile:?} -> {name:?} produced a climbing path"
            );
            assert_eq!(
                dir.components().count(),
                base.components().count() + 1,
                "{hostile:?} -> {name:?} changed the depth"
            );
        }
    }

    #[test]
    fn the_transport_is_off_unless_a_valid_port_is_set() {
        let _g = crate::env_test_guard();
        let prev = std::env::var("NEOBROWSER_HTTP_PORT").ok();
        std::env::remove_var("NEOBROWSER_HTTP_PORT");
        assert_eq!(configured(), None, "must be opt-in");

        std::env::set_var("NEOBROWSER_HTTP_PORT", "8931");
        assert_eq!(configured(), Some(("127.0.0.1".into(), 8931)));

        // Loopback by default; a non-loopback bind must be spelled out.
        std::env::set_var("NEOBROWSER_HTTP_BIND", "0.0.0.0");
        assert_eq!(configured(), Some(("0.0.0.0".into(), 8931)));
        std::env::remove_var("NEOBROWSER_HTTP_BIND");

        // Privileged ports and nonsense do not silently become a default.
        std::env::set_var("NEOBROWSER_HTTP_PORT", "80");
        assert_eq!(configured(), None);
        std::env::set_var("NEOBROWSER_HTTP_PORT", "banana");
        assert_eq!(configured(), None);

        match prev {
            Some(v) => std::env::set_var("NEOBROWSER_HTTP_PORT", v),
            None => std::env::remove_var("NEOBROWSER_HTTP_PORT"),
        }
    }
}
