//! HTTP client backed by impit with Chrome 142 TLS fingerprint.
//!
//! Used as a fallback when wreq gets Cloudflare-blocked (403).
//! impit uses patched rustls to replicate browser TLS fingerprints exactly,
//! bypassing JA3/JA4 fingerprint checks that BoringSSL-based clients fail.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use impit::cookie::Jar;
use impit::fingerprint::database::chrome_142;
use impit::impit::Impit;

/// HTTP client using impit with Chrome 142 TLS fingerprint.
///
/// Thread-safe via Arc wrapping. Designed as a Cloudflare fallback client
/// that gets tried when wreq returns 403 with Cloudflare markers.
#[derive(Clone)]
pub struct ImpitClient {
    client: Arc<Impit<Jar>>,
    timeout: Duration,
}

impl std::fmt::Debug for ImpitClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImpitClient")
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

/// Result of an impit fetch: status, headers, body.
pub struct ImpitResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: String,
}

impl ImpitClient {
    /// Create a new impit client with Chrome 142 TLS fingerprint.
    pub fn new(timeout_ms: u64) -> Result<Self, String> {
        let timeout = Duration::from_millis(timeout_ms);
        let client = Impit::<Jar>::builder()
            .with_fingerprint(chrome_142::fingerprint())
            .with_default_timeout(timeout)
            .build()
            .map_err(|e| format!("impit init: {e}"))?;

        Ok(Self {
            client: Arc::new(client),
            timeout,
        })
    }

    /// Create a client with the default 15-second timeout.
    pub fn default_client() -> Result<Self, String> {
        Self::new(15_000)
    }

    /// Fetch a URL with the given method, body, and headers.
    ///
    /// Returns ImpitResponse on success.
    pub async fn fetch(
        &self,
        url: &str,
        method: &str,
        body: Option<&str>,
        headers: &HashMap<String, String>,
    ) -> Result<ImpitResponse, String> {
        // Build header pairs for impit RequestOptions.
        let header_pairs: Vec<(String, String)> = headers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        let opts = if !header_pairs.is_empty() {
            Some(impit::request::RequestOptions {
                headers: header_pairs,
                ..Default::default()
            })
        } else {
            None
        };

        let body_bytes = body.map(|b| b.as_bytes().to_vec());

        let response = match method.to_uppercase().as_str() {
            "GET" => self.client.get(url.to_string(), body_bytes, opts).await,
            "POST" => self.client.post(url.to_string(), body_bytes, opts).await,
            "PUT" => self.client.put(url.to_string(), body_bytes, opts).await,
            "PATCH" => self.client.patch(url.to_string(), body_bytes, opts).await,
            "DELETE" => self.client.delete(url.to_string(), body_bytes, opts).await,
            "HEAD" => self.client.head(url.to_string(), body_bytes, opts).await,
            "OPTIONS" => self.client.options(url.to_string(), body_bytes, opts).await,
            _ => self.client.get(url.to_string(), body_bytes, opts).await,
        };

        match response {
            Ok(resp) => {
                let status = resp.status().as_u16();
                let resp_headers: HashMap<String, String> = resp
                    .headers()
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                    .collect();

                // Handle brotli manually like we do for wreq.
                let content_encoding = resp_headers
                    .get("content-encoding")
                    .map(|s| s.as_str())
                    .unwrap_or("");

                let body_text = if content_encoding.contains("br") {
                    let raw_bytes = resp
                        .bytes()
                        .await
                        .map_err(|e| format!("impit body bytes: {e}"))?;
                    let mut decompressed = Vec::new();
                    let mut reader = brotli::Decompressor::new(&raw_bytes[..], 4096);
                    match std::io::Read::read_to_end(&mut reader, &mut decompressed) {
                        Ok(_) => String::from_utf8_lossy(&decompressed).to_string(),
                        Err(_) => String::from_utf8_lossy(&raw_bytes).to_string(),
                    }
                } else {
                    resp.text()
                        .await
                        .map_err(|e| format!("impit body text: {e}"))?
                };

                Ok(ImpitResponse {
                    status,
                    headers: resp_headers,
                    body: body_text,
                })
            }
            Err(e) => Err(format!("impit fetch: {e}")),
        }
    }
}
