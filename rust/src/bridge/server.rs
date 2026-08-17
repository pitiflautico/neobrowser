//! The loopback listener the Chrome extension talks to.
//!
//! Authentication is not optional here, and the reason is specific: a web page can POST
//! `text/plain` to `127.0.0.1` with no preflight, which means any page the user visits could
//! otherwise poison this queue's results or drain it. So every request must carry a custom
//! header — something a page cannot set cross-origin — and the token is compared in constant
//! time.

//! The agent half of the Chrome bridge: a localhost queue the extension polls.
//!
//! See `extension/README.md` for the security model. In short: instead of copying the
//! user's cookies into a second browser, or opening `--remote-debugging-port` on their
//! everyday Chrome and exposing every tab, the user shares one tab at a time from a
//! popup and can revoke it.
//!
//! The transport is a poll rather than a socket, and that is forced by the platform: a
//! Manifest V3 service worker is terminated when idle, so a long-lived WebSocket from
//! the extension would be killed with it. The extension asks for work; this side hands
//! out queued CDP commands and collects the results.
//!
//! Bound to `127.0.0.1` and **authenticated with a per-session token**.
//!
//! Loopback alone is not a boundary here, and it is worth being precise about why. Any
//! web page the user visits can issue a request to `http://127.0.0.1:<port>` — a
//! `text/plain` POST is a "simple" request, so there is no preflight, and while the
//! browser blocks the page from *reading* the response, the side effect still happens.
//! Without authentication a hostile page could therefore:
//!
//! * POST a forged result to `/bridge/results` and poison an in-flight CDP command,
//!   feeding the agent a fabricated answer; or
//! * POST to `/bridge` and drain the queue, so the real extension never receives the
//!   commands and every action times out.
//!
//! Two things close that. Every request must carry `X-NeoBrowser-Token`, which is a
//! *custom* header — that alone makes the request non-simple, so a page cannot send it
//! without a preflight this server refuses. And command ids are random rather than
//! sequential, so a forged result cannot address an outstanding waiter even if the
//! token were known.
//!
//! The token lives in `~/.neobrowser/bridge.token` at 0600, so a same-uid process can
//! read it — the same trust the MCP stdio transport already implies — while nothing
//! else can. Per-tab consent is still enforced in the extension, where Chrome shows
//! its own "being debugged" banner.

use std::sync::Arc;

use super::queue::Bridge;
use super::{MAX_BODY, TOKEN_HEADER};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Run the bridge HTTP server until the process ends.
///
/// A hand-rolled minimal HTTP/1.1 responder rather than a web framework: the surface is
/// two POST routes on loopback, and adding an HTTP server dependency for that would be
/// a large amount of new attack surface and audit burden for no functionality.
pub async fn serve(bridge: Arc<Bridge>) -> std::io::Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", bridge.port())).await?;
    tracing::info!(
        port = bridge.port(),
        "bridge listening on 127.0.0.1; load extension/ in chrome://extensions to connect"
    );
    loop {
        let (mut socket, _peer) = match listener.accept().await {
            Ok(pair) => pair,
            // A failed accept is not fatal: keep serving rather than taking the
            // bridge down for one bad connection.
            Err(e) => {
                tracing::warn!(error = %e, "bridge accept failed");
                continue;
            }
        };
        let bridge = bridge.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(&bridge, &mut socket).await {
                tracing::debug!(error = %e, "bridge connection ended");
            }
        });
    }
}

pub(super) async fn handle_connection(
    bridge: &Bridge,
    socket: &mut tokio::net::TcpStream,
) -> std::io::Result<()> {
    let mut buf = Vec::with_capacity(8192);
    let mut chunk = [0u8; 8192];

    // Read until the headers are complete.
    let header_end = loop {
        if let Some(pos) = find_header_end(&buf) {
            break pos;
        }
        let n = socket.read(&mut chunk).await?;
        if n == 0 {
            return Ok(()); // client closed before sending a full request
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > MAX_BODY {
            return respond(socket, 413, &json!({ "error": "request too large" })).await;
        }
    };

    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or_default().to_string();
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

    // Authenticated BEFORE the body is parsed, so an unauthenticated caller cannot even
    // make this process allocate for its payload.
    if !bridge.token_matches(header(TOKEN_HEADER).as_deref()) {
        return respond(
            socket,
            401,
            &json!({
                "error": "missing or invalid X-NeoBrowser-Token",
                "hint": "run `neobrowser bridge token` and paste the value into the \
                         NeoBrowser Bridge popup",
            }),
        )
        .await;
    }

    let content_length = header("content-length")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    if content_length > MAX_BODY {
        return respond(socket, 413, &json!({ "error": "body too large" })).await;
    }

    // Read the declared body length.
    let body_start = header_end + 4;
    while buf.len() < body_start + content_length {
        let n = socket.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let body = &buf[body_start.min(buf.len())..];
    let parsed: Value = serde_json::from_slice(body).unwrap_or(Value::Null);

    if method != "POST" {
        return respond(socket, 405, &json!({ "error": "only POST is supported" })).await;
    }

    match path.as_str() {
        "/bridge" => {
            let shared: Vec<i64> = parsed
                .get("shared_tabs")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_i64).collect())
                .unwrap_or_default();
            let work = bridge.take_work(shared).await;
            respond(socket, 200, &work).await
        }
        "/bridge/results" => {
            let results: Vec<Value> = parsed
                .get("results")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let delivered = bridge.deliver(&results).await;
            respond(socket, 200, &json!({ "delivered": delivered })).await
        }
        _ => respond(socket, 404, &json!({ "error": "no such route" })).await,
    }
}

pub(super) fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

async fn respond(
    socket: &mut tokio::net::TcpStream,
    status: u16,
    body: &Value,
) -> std::io::Result<()> {
    let text = body.to_string();
    let reason = match status {
        200 => "OK",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        _ => "Error",
    };
    // No CORS headers, deliberately: an extension service worker is not subject to
    // CORS, and adding permissive headers would let any web page in the browser talk
    // to the bridge.
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\n\
         content-type: application/json\r\n\
         content-length: {}\r\n\
         connection: close\r\n\
         \r\n{text}",
        text.len()
    );
    socket.write_all(response.as_bytes()).await?;
    socket.flush().await
}
