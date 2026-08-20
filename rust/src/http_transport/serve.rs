//! The HTTP listener and request handler for MCP over Streamable HTTP.
//!
//! Each `Mcp-Session-Id` gets its own `Browser`, so two clients never share a tab, a cookie
//! jar, or a login. Sessions expire on idle rather than living until shutdown, because a
//! long-lived server accumulating browsers is how a machine runs out of memory overnight.

use std::sync::Arc;

use serde_json::{json, Value};
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;

use super::origin::origin_allowed;
use super::respond::{reply, reply_empty};
use super::{HttpTransport, MAX_BODY};

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
