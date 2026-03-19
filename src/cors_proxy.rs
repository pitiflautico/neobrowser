//! Built-in CORS proxy for neobrowser.
//!
//! Runs a local HTTP server that proxies requests to any target,
//! adding permissive CORS headers. Useful for bounty hunting when
//! the target blocks cross-origin requests.
//!
//! Usage: neobrowser_rs proxy --port 8888
//! Then:  fetch('http://localhost:8888/https://target.com/api/secret')

use std::io::{Read as _, Write as _};

/// Start the CORS proxy server on the given port.
pub async fn run(port: u16) -> Result<(), Box<dyn std::error::Error>> {
    let listener = std::net::TcpListener::bind(format!("127.0.0.1:{port}"))?;
    eprintln!("[PROXY] CORS proxy listening on http://127.0.0.1:{port}");
    eprintln!("[PROXY] Usage: fetch('http://127.0.0.1:{port}/https://target.com/path')");

    let client = rquest::Client::builder()
        .danger_accept_invalid_certs(true)
        .redirect(rquest::redirect::Policy::limited(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    for stream in listener.incoming() {
        let mut stream = stream?;
        let client = client.clone();

        tokio::spawn(async move {
            let mut buf = [0u8; 8192];
            let n = match stream.read(&mut buf) {
                Ok(n) => n,
                Err(_) => return,
            };
            let request = String::from_utf8_lossy(&buf[..n]).to_string();

            // Parse HTTP request line
            let first_line = request.lines().next().unwrap_or("");
            let parts: Vec<&str> = first_line.split_whitespace().collect();
            if parts.len() < 2 {
                let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n");
                return;
            }

            let method = parts[0];
            let path = parts[1];

            // Handle OPTIONS preflight
            if method == "OPTIONS" {
                let resp = format!(
                    "HTTP/1.1 204 No Content\r\n\
                     Access-Control-Allow-Origin: *\r\n\
                     Access-Control-Allow-Methods: GET, POST, PUT, DELETE, PATCH, OPTIONS\r\n\
                     Access-Control-Allow-Headers: *\r\n\
                     Access-Control-Max-Age: 86400\r\n\
                     Content-Length: 0\r\n\r\n"
                );
                let _ = stream.write_all(resp.as_bytes());
                return;
            }

            // Extract target URL from path (strip leading /)
            let target_url = if path.starts_with("/http") {
                &path[1..]
            } else if path == "/" || path == "/health" {
                let body = r#"{"status":"ok","usage":"GET /https://target.com/path"}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\n\
                     Access-Control-Allow-Origin: *\r\n\
                     Content-Type: application/json\r\n\
                     Content-Length: {}\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes());
                return;
            } else {
                let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\n\r\nUsage: GET /https://target.com/path");
                return;
            };

            // Extract headers from request
            let mut headers = rquest::header::HeaderMap::new();
            for line in request.lines().skip(1) {
                if line.is_empty() {
                    break;
                }
                if let Some((key, value)) = line.split_once(": ") {
                    let key_lower = key.to_lowercase();
                    // Skip hop-by-hop and host headers
                    if matches!(
                        key_lower.as_str(),
                        "host" | "connection" | "keep-alive" | "transfer-encoding" | "te" | "trailer" | "upgrade"
                    ) {
                        continue;
                    }
                    if let (Ok(k), Ok(v)) = (
                        rquest::header::HeaderName::from_bytes(key.as_bytes()),
                        rquest::header::HeaderValue::from_str(value),
                    ) {
                        headers.insert(k, v);
                    }
                }
            }

            // Extract body (after empty line)
            let body = if let Some(pos) = request.find("\r\n\r\n") {
                let body_start = pos + 4;
                if body_start < request.len() {
                    Some(request[body_start..].to_string())
                } else {
                    None
                }
            } else {
                None
            };

            // Make the proxied request
            let req = match method {
                "GET" => client.get(target_url).headers(headers),
                "POST" => {
                    let mut r = client.post(target_url).headers(headers);
                    if let Some(b) = body {
                        r = r.body(b);
                    }
                    r
                }
                "PUT" => {
                    let mut r = client.put(target_url).headers(headers);
                    if let Some(b) = body {
                        r = r.body(b);
                    }
                    r
                }
                "DELETE" => client.delete(target_url).headers(headers),
                "PATCH" => {
                    let mut r = client.patch(target_url).headers(headers);
                    if let Some(b) = body {
                        r = r.body(b);
                    }
                    r
                }
                _ => {
                    let _ = stream.write_all(b"HTTP/1.1 405 Method Not Allowed\r\n\r\n");
                    return;
                }
            };

            match req.send().await {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    let content_type = resp
                        .headers()
                        .get("content-type")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("application/octet-stream")
                        .to_string();

                    let resp_body = resp.bytes().await.unwrap_or_default();

                    let response = format!(
                        "HTTP/1.1 {status} OK\r\n\
                         Access-Control-Allow-Origin: *\r\n\
                         Access-Control-Allow-Headers: *\r\n\
                         Access-Control-Expose-Headers: *\r\n\
                         Content-Type: {content_type}\r\n\
                         Content-Length: {}\r\n\r\n",
                        resp_body.len()
                    );
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.write_all(&resp_body);

                    eprintln!(
                        "[PROXY] {method} {target_url} → {status} ({} bytes)",
                        resp_body.len()
                    );
                }
                Err(e) => {
                    let body = format!("{{\"error\":\"{e}\"}}");
                    let resp = format!(
                        "HTTP/1.1 502 Bad Gateway\r\n\
                         Access-Control-Allow-Origin: *\r\n\
                         Content-Type: application/json\r\n\
                         Content-Length: {}\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(resp.as_bytes());
                    eprintln!("[PROXY] {method} {target_url} → ERROR: {e}");
                }
            }
        });
    }
    Ok(())
}
