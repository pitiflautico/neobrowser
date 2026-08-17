//! Writing HTTP responses by hand.
//!
//! Small enough not to justify a framework, and a framework here would mean an HTTP stack in
//! the dependency tree of a tool whose entire security posture rests on what it will and will
//! not accept on a loopback port.

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

use serde_json::Value;
use tokio::io::AsyncWriteExt;

pub(super) async fn reply(
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

pub(super) async fn reply_empty(
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

pub(super) fn reason(status: u16) -> &'static str {
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
