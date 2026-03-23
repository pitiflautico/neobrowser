//! HTTP client backed by rquest with Chrome 136 TLS fingerprint.

use crate::classify::should_skip;
use crate::headers;
use crate::{HttpClient, HttpError, HttpRequest, RequestKind};
use neo_types::HttpResponse;
use std::sync::Arc;
use std::time::Duration;

/// HTTP client using rquest with Chrome 136 TLS emulation.
///
/// Wraps a connection-pooled `rquest::Client` and executes requests
/// on a dedicated tokio runtime thread to avoid async conflicts.
#[derive(Debug, Clone)]
pub struct RquestClient {
    client: Arc<rquest::Client>,
    timeout: Duration,
}

impl RquestClient {
    /// Create a new client with Chrome 136 TLS and the given timeout.
    ///
    /// Uses `rquest_util::Emulation::Chrome135` for an authentic TLS fingerprint.
    pub fn new(timeout_ms: u64) -> Result<Self, HttpError> {
        let timeout = Duration::from_millis(timeout_ms);
        let client = rquest::Client::builder()
            .emulation(rquest_util::Emulation::Chrome136)
            .cookie_store(true)
            .redirect(rquest::redirect::Policy::limited(10))
            .timeout(timeout)
            .connect_timeout(Duration::from_secs(10))
            // Default pool settings — let rquest manage HTTP/2 multiplexing
            .build()
            .map_err(|e| HttpError::Network(e.to_string()))?;
        Ok(Self {
            client: Arc::new(client),
            timeout,
        })
    }

    /// Create a client with the default 10-second timeout.
    pub fn default_client() -> Result<Self, HttpError> {
        Self::new(10_000)
    }
}

impl RquestClient {
    /// Expose the raw rquest client for streaming ops.
    ///
    /// Used by neo-runtime's streaming fetch ops which need to hold the
    /// response open across multiple read_chunk calls.
    pub fn raw_client(&self) -> Arc<rquest::Client> {
        Arc::clone(&self.client)
    }

    /// Get the configured timeout.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }
}

impl Default for RquestClient {
    fn default() -> Self {
        Self::default_client().expect("failed to build default RquestClient")
    }
}

impl HttpClient for RquestClient {
    /// Send an HTTP request through Chrome 136 TLS.
    ///
    /// Telemetry URLs are rejected with `HttpError::Skipped`.
    /// Runs on a dedicated thread with its own tokio runtime.
    fn request(&self, req: &HttpRequest) -> Result<HttpResponse, HttpError> {
        if should_skip(&req.url) {
            return Err(HttpError::Skipped {
                url: req.url.clone(),
            });
        }

        let client = Arc::clone(&self.client);
        let method = req.method.clone();
        let url = req.url.clone();
        let body = req.body.clone();
        let timeout = self.timeout;
        let merged = build_headers(req);

        let handle =
            std::thread::spawn(move || run_request(client, &method, &url, body, merged, timeout));

        handle
            .join()
            .map_err(|_| HttpError::Network("request thread panicked".into()))?
    }
}

/// Merge classification-based defaults with request-specific headers.
///
/// Public so streaming fetch ops can build the same header set.
pub fn build_headers(req: &HttpRequest) -> Vec<(String, String)> {
    let base = match req.context.kind {
        RequestKind::Navigation | RequestKind::FormSubmit => headers::navigation_headers(),
        _ => headers::fetch_headers(),
    };
    let mut merged: Vec<(String, String)> = base.into_iter().collect();
    for (k, v) in &req.headers {
        if let Some(entry) = merged.iter_mut().find(|(ek, _)| ek == k) {
            entry.1 = v.clone();
        } else {
            merged.push((k.clone(), v.clone()));
        }
    }
    merged
}

/// Execute the HTTP request inside a dedicated tokio runtime.
fn run_request(
    client: Arc<rquest::Client>,
    method: &str,
    url: &str,
    body: Option<String>,
    headers: Vec<(String, String)>,
    timeout: Duration,
) -> Result<HttpResponse, HttpError> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| HttpError::Network(format!("runtime: {e}")))?;

    rt.block_on(async {
        let start = std::time::Instant::now();
        let m = method
            .parse::<rquest::Method>()
            .map_err(|e| HttpError::Network(format!("bad method: {e}")))?;

        let mut builder = client.request(m, url).timeout(timeout);

        for (k, v) in &headers {
            builder = builder.header(k.as_str(), v.as_str());
        }
        if let Some(b) = body {
            builder = builder.body(b);
        }

        let mut resp = builder
            .send()
            .await
            .map_err(|e| HttpError::Network(e.to_string()))?;

        let status = resp.status().as_u16();
        let resp_headers: std::collections::HashMap<String, String> = resp
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();
        let resp_url = resp.url().to_string();

        // Detect SSE (streaming) responses — read chunk by chunk until [DONE]
        // instead of waiting for EOF (which never comes for live streams).
        let is_sse = resp_headers
            .get("content-type")
            .map(|ct| ct.contains("text/event-stream") || ct.contains("text/x-sse"))
            .unwrap_or(false);

        let text = if is_sse {
            // Stream SSE: read chunks until [DONE] marker or timeout.
            // ChatGPT sends "data: [DONE]" as the last SSE event.
            let sse_deadline = std::time::Instant::now() + Duration::from_secs(60);
            let mut body = String::new();
            loop {
                let remaining = sse_deadline.saturating_duration_since(std::time::Instant::now());
                if remaining.is_zero() {
                    eprintln!("[SSE] deadline reached after {}KB", body.len() / 1024);
                    break;
                }
                match tokio::time::timeout(
                    Duration::from_secs(15).min(remaining),
                    resp.chunk(),
                )
                .await
                {
                    Ok(Ok(Some(chunk))) => {
                        let s = String::from_utf8_lossy(&chunk);
                        body.push_str(&s);
                        if body.contains("[DONE]") {
                            break;
                        }
                    }
                    Ok(Ok(None)) => break,       // stream ended
                    Ok(Err(e)) => {
                        eprintln!("[SSE] chunk error: {e}");
                        break;
                    }
                    Err(_) => {
                        eprintln!("[SSE] chunk timeout, got {}KB so far", body.len() / 1024);
                        break;
                    }
                }
            }
            body
        } else {
            resp.text()
                .await
                .map_err(|e| HttpError::Decode(e.to_string()))?
        };

        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(HttpResponse {
            status,
            headers: resp_headers,
            body: text,
            url: resp_url,
            duration_ms,
        })
    })
}
