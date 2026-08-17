//! Writing HTTP responses by hand.
//!
//! Small enough not to justify a framework, and a framework here would mean an HTTP stack in
//! the dependency tree of a tool whose entire security posture rests on what it will and will
//! not accept on a loopback port.

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
