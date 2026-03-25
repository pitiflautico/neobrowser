//! Fetch ops — HTTP fetch with streaming, brotli fallback, impit fallback, Chrome fallback.

use crate::ops::headers::parse_headers;
use crate::ops::url_filter::should_skip_url;
use crate::ops::{SharedCookieStore, SharedImpitClient, SharedRquestClient, StreamStore};
use crate::scheduler::{FetchBudget, FetchGuard};
use deno_core::op2;
use deno_core::OpState;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use super::{ActiveStream, SharedFetchRuntime};

/// Start a streaming fetch — sends request, returns headers + stream_id.
///
/// The response body stays open for incremental reading via `op_fetch_read_chunk`.
/// Uses the same URL-skip logic, cookie injection, and header merging as `op_fetch`.
#[op2(async(lazy), fast)]
#[string]
pub async fn op_fetch_start(
    state: Rc<RefCell<OpState>>,
    #[string] url: String,
    #[string] method: String,
    #[string] body: String,
    #[string] headers_json: String,
) -> Result<String, deno_error::JsErrorBox> {
    if should_skip_url(&url) {
        tokio::task::yield_now().await;
        let stream_id = {
            let mut s = state.borrow_mut();
            let store = s.borrow_mut::<StreamStore>();
            let id = store.next_id;
            store.next_id += 1;
            id
        };
        return Ok(serde_json::json!({
            "stream_id": stream_id,
            "status": 200,
            "headers": {},
            "url": url,
        })
        .to_string());
    }

    // Check fetch budget — FetchGuard ensures finish_fetch on all exit paths.
    let (raw_client, timeout_ms, _fetch_guard) = {
        let s = state.borrow();
        let fetch_budget = s.try_borrow::<FetchBudget>().cloned();
        let guard = if let Some(ref fb) = fetch_budget {
            if fb.is_aborted() {
                return Err(deno_error::JsErrorBox::generic("fetch aborted by watchdog"));
            }
            let g = FetchGuard::acquire(fb).ok_or_else(|| {
                deno_error::JsErrorBox::generic(
                    "fetch budget exceeded: too many concurrent requests",
                )
            })?;
            Some(g)
        } else {
            None
        };
        let timeout = fetch_budget
            .as_ref()
            .map(|fb| fb.per_request_timeout_ms())
            .unwrap_or(5000);
        let handle = s
            .try_borrow::<SharedRquestClient>()
            .ok_or_else(|| deno_error::JsErrorBox::generic("No RqwestClient in OpState"))?;
        (handle.0.clone(), timeout, guard)
    };

    let mut headers = parse_headers(&headers_json);
    let body_opt = if body.is_empty() { None } else { Some(body) };

    // Save copies for Chrome fallback before values are moved.
    let method_str_copy = method.clone();
    let body_for_fallback = body_opt.clone();
    let headers_for_fallback = headers.clone();

    // Auto-inject cookies.
    let cookie_store_arc = {
        let s = state.borrow();
        if !headers.contains_key("cookie") && !headers.contains_key("Cookie") {
            if let Some(store) = s.try_borrow::<SharedCookieStore>() {
                if let Some(ref cs) = store.0 {
                    let cookie_header = cs.get_for_request(&url, None, true);
                    if !cookie_header.is_empty() {
                        headers.insert("Cookie".to_string(), cookie_header);
                    }
                }
            }
        }
        s.try_borrow::<SharedCookieStore>()
            .and_then(|s| s.0.clone())
    };

    // Build and send request.
    let m: wreq::Method = method
        .parse()
        .map_err(|e| deno_error::JsErrorBox::generic(format!("bad method: {e}")))?;
    let mut builder = raw_client
        .request(m, &url)
        .timeout(std::time::Duration::from_millis(timeout_ms as u64));

    // Merge headers: defaults + request-specific (override, not append).
    let mut merged_hdrs: HashMap<String, String> =
        neo_http::headers::fetch_headers().into_iter().collect();
    for (k, v) in &headers {
        merged_hdrs.insert(k.clone(), v.clone());
    }
    for (k, v) in &merged_hdrs {
        builder = builder.header(k.as_str(), v.as_str());
    }
    if let Some(b) = body_opt {
        builder = builder.body(b);
    }

    let resp = builder
        .send()
        .await
        .map_err(|e| deno_error::JsErrorBox::generic(format!("fetch_start send: {e}")))?;

    // Budget slot released automatically by _fetch_guard drop at function end.

    let mut status = resp.status().as_u16();
    let mut resp_headers: HashMap<String, String> = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let mut resp_url = resp.uri().to_string();

    // Store Set-Cookie headers.
    if let Some(ref cs) = cookie_store_arc {
        for key in &["set-cookie", "Set-Cookie"] {
            if let Some(val) = resp_headers.get(*key) {
                cs.store_set_cookie(&url, val);
            }
        }
    }

    // Cloudflare fallback for streaming: if 403, try impit first, then Chrome.
    if status == 403 {
        // Read the 403 body — consumes resp.
        let body_text = resp.text().await.unwrap_or_default();
        let mut fallback_body = body_text.clone();
        if crate::chrome_fallback::is_cloudflare_block(&body_text) {
            // Try impit first (lightweight TLS impersonation).
            eprintln!("[neo-impit-fallback] streaming 403 for {url}, trying impit...");
            let impit_client = {
                let s = state.borrow();
                s.try_borrow::<SharedImpitClient>().cloned()
            };
            if let Some(impit) = impit_client {
                match impit.0.fetch(
                    &url, &method_str_copy, body_for_fallback.as_deref(),
                    &headers_for_fallback,
                ).await {
                    Ok(ir) => {
                        eprintln!("[neo-impit-fallback] impit returned status {}", ir.status);
                        status = ir.status;
                        resp_headers = ir.headers;
                        resp_url = url.clone();
                        fallback_body = ir.body;
                    }
                    Err(e) => eprintln!("[neo-impit-fallback] impit fetch failed: {e}"),
                }
            }
        }
        // If still blocked after impit, try Chrome.
        if status == 403 && crate::chrome_fallback::is_cloudflare_block(&fallback_body) {
            eprintln!("[neo-chrome-fallback] streaming 403 for {url}, retrying via Chrome...");
            let fallback = {
                let s = state.borrow();
                s.try_borrow::<crate::chrome_fallback::SharedChromeFallback>().cloned()
            };
            if let Some(fb) = fallback {
                let cookie_tuples: Option<Vec<(String, String, String)>> = cookie_store_arc.as_ref().and_then(|cs| {
                    url::Url::parse(&url).ok().and_then(|u| {
                        u.host_str().map(|domain| {
                            cs.list_for_domain(domain)
                                .into_iter()
                                .map(|c| (c.name.clone(), c.value.clone(), c.domain.clone()))
                                .collect()
                        })
                    })
                });
                match fb.fetch_via_chrome(
                    &url, &method_str_copy, body_for_fallback.as_deref(),
                    &headers_for_fallback, cookie_tuples.as_deref(),
                ).await {
                    Ok(cr) => {
                        eprintln!("[neo-chrome-fallback] Chrome returned status {}", cr.status);
                        status = cr.status;
                        resp_headers = cr.headers;
                        resp_url = url.clone();
                        fallback_body = cr.body;
                    }
                    Err(e) => eprintln!("[neo-chrome-fallback] Chrome fetch failed: {e}"),
                }
            }
        }
        // Store as pre-filled stream (resp was consumed by text()).
        let stream_id = {
            let mut s = state.borrow_mut();
            let store = s.borrow_mut::<StreamStore>();
            let id = store.next_id;
            store.next_id += 1;
            store.streams.insert(id, ActiveStream {
                response: None,
                created_at: std::time::Instant::now(),
                prefilled_body: Some(fallback_body),
            });
            id
        };
        return Ok(serde_json::json!({
            "stream_id": stream_id, "status": status,
            "headers": resp_headers, "url": resp_url,
        }).to_string());
    }

    // Normal path: store live response for streaming reads.
    let stream_id = {
        let mut s = state.borrow_mut();
        let store = s.borrow_mut::<StreamStore>();
        let id = store.next_id;
        store.next_id += 1;
        store.streams.insert(id, ActiveStream {
            response: Some(resp),
            created_at: std::time::Instant::now(),
            prefilled_body: None,
        });
        id
    };

    Ok(serde_json::json!({
        "stream_id": stream_id,
        "status": status,
        "headers": resp_headers,
        "url": resp_url,
    })
    .to_string())
}

/// Read the next chunk from a streaming fetch response.
///
/// Returns `{ "done": false, "data": "base64..." }` for each chunk,
/// or `{ "done": true }` when the stream is exhausted.
/// Automatically removes the stream from the store on EOF or error.
#[op2(async(lazy), fast)]
#[string]
pub async fn op_fetch_read_chunk(
    state: Rc<RefCell<OpState>>,
    #[smi] stream_id: u32,
) -> Result<String, deno_error::JsErrorBox> {
    // Check for pre-filled body (Chrome fallback) — return as single chunk then EOF.
    {
        let mut s = state.borrow_mut();
        let store = s.borrow_mut::<StreamStore>();
        if let Some(stream) = store.streams.get_mut(&stream_id) {
            if let Some(body) = stream.prefilled_body.take() {
                // Return the full body as one chunk, next call will be EOF.
                return Ok(serde_json::json!({
                    "done": false,
                    "data": body,
                }).to_string());
            }
            // If prefilled_body was already taken (second call), return EOF.
            if stream.response.is_none() {
                store.streams.remove(&stream_id);
                return Ok(r#"{"done":true}"#.to_string());
            }
        }
    }

    // Extract the response from the store — MUST NOT hold borrow across await.
    let mut resp = {
        let mut s = state.borrow_mut();
        let store = s.borrow_mut::<StreamStore>();
        let stream = store
            .streams
            .get_mut(&stream_id)
            .ok_or_else(|| deno_error::JsErrorBox::generic("stream not found"))?;
        stream
            .response
            .take()
            .ok_or_else(|| deno_error::JsErrorBox::generic("stream already reading"))?
    };

    // Read next chunk with a 30s timeout.
    let chunk_result =
        tokio::time::timeout(std::time::Duration::from_secs(30), resp.chunk()).await;

    match chunk_result {
        Ok(Ok(Some(bytes))) => {
            // Put response back for next read.
            {
                let mut s = state.borrow_mut();
                let store = s.borrow_mut::<StreamStore>();
                if let Some(stream) = store.streams.get_mut(&stream_id) {
                    stream.response = Some(resp);
                }
            }
            // Return chunk as UTF-8 text (most web responses are text).
            let text = String::from_utf8_lossy(&bytes);
            Ok(serde_json::json!({
                "done": false,
                "data": text,
            })
            .to_string())
        }
        Ok(Ok(None)) => {
            // EOF — clean up.
            let mut s = state.borrow_mut();
            let store = s.borrow_mut::<StreamStore>();
            store.streams.remove(&stream_id);
            Ok(r#"{"done":true}"#.to_string())
        }
        Ok(Err(e)) => {
            // Read error — clean up.
            let mut s = state.borrow_mut();
            let store = s.borrow_mut::<StreamStore>();
            store.streams.remove(&stream_id);
            Err(deno_error::JsErrorBox::generic(format!(
                "chunk read error: {e}"
            )))
        }
        Err(_) => {
            // Timeout — clean up.
            let mut s = state.borrow_mut();
            let store = s.borrow_mut::<StreamStore>();
            store.streams.remove(&stream_id);
            Err(deno_error::JsErrorBox::generic("chunk read timeout (30s)"))
        }
    }
}

/// Close a streaming fetch, releasing the response.
///
/// Safe to call multiple times or on already-closed streams.
#[op2(fast)]
pub fn op_fetch_close(state: Rc<RefCell<OpState>>, #[smi] stream_id: u32) {
    let mut s = state.borrow_mut();
    if let Some(store) = s.try_borrow_mut::<StreamStore>() {
        store.streams.remove(&stream_id);
    }
}

/// Fetch a URL. ASYNC op — yields to event loop during I/O.
///
/// Delegates to the `HttpClient` trait object in OpState.
/// Skips telemetry/analytics URLs with a fake 200 response.
/// Respects `FetchBudget` concurrency limits and abort flag.
/// Shared tokio runtime for fetch ops — Chromium-style single network thread.
///
/// Chrome runs all network I/O on ONE thread with async I/O and a shared connection pool.
/// Previous NeoRender: each fetch → spawn_blocking → thread::spawn → new tokio runtime
/// = 20 fetches → 40 threads × 20 runtimes → connection pool chaos.
///
/// Now: ONE shared multi-thread tokio runtime for all fetches. Runs on spawn_blocking
/// so deno_core's event loop doesn't see pending ops (allowing settle to work), but
/// all fetches share one connection pool and runtime internally.
///
/// Chromium-style fetch — shared network runtime, shared connection pool.
///
/// Uses spawn_blocking to keep the fetch off deno_core's event loop (so settle
/// works correctly), but the blocking thread dispatches the actual HTTP request
/// on a shared tokio runtime instead of creating a new one per fetch.
///
/// Being async is CRITICAL for SPA correctness: fetch().then(cb) must
/// resolve cb as a microtask in a FUTURE event loop tick, not the current one.
#[op2(async(lazy), fast)]
#[string]
pub async fn op_fetch(
    state: Rc<RefCell<OpState>>,
    #[string] url: String,
    #[string] method: String,
    #[string] body: String,
    #[string] headers_json: String,
) -> Result<String, deno_error::JsErrorBox> {
    if should_skip_url(&url) {
        tokio::task::yield_now().await;
        return Ok(r#"{"status":200,"body":"","headers":{}}"#.to_string());
    }

    // Check fetch budget — FetchGuard ensures finish_fetch on all exit paths.
    let (raw_client, timeout_ms, _fetch_guard, fetch_rt) = {
        let s = state.borrow();

        let fetch_budget = s.try_borrow::<FetchBudget>().cloned();
        let guard = if let Some(ref fb) = fetch_budget {
            if fb.is_aborted() {
                return Err(deno_error::JsErrorBox::generic("fetch aborted by watchdog"));
            }
            let g = FetchGuard::acquire(fb).ok_or_else(|| {
                deno_error::JsErrorBox::generic(
                    "fetch budget exceeded: too many concurrent requests",
                )
            })?;
            Some(g)
        } else {
            None
        };

        let timeout = fetch_budget
            .as_ref()
            .map(|fb| fb.per_request_timeout_ms())
            .unwrap_or(5000);

        let handle = s
            .try_borrow::<SharedRquestClient>()
            .ok_or_else(|| deno_error::JsErrorBox::generic("No RquestClient in OpState"))?;

        let rt = s
            .try_borrow::<SharedFetchRuntime>()
            .ok_or_else(|| deno_error::JsErrorBox::generic("No FetchRuntime in OpState"))?;

        (handle.0.clone(), timeout, guard, rt.0.clone())
    };

    let mut headers = parse_headers(&headers_json);
    let body_opt = if body.is_empty() { None } else { Some(body) };

    // Auto-inject cookies from the cookie store if no Cookie header is set.
    let cookie_store_arc = {
        let s = state.borrow();
        if !headers.contains_key("cookie") && !headers.contains_key("Cookie") {
            if let Some(store) = s.try_borrow::<SharedCookieStore>() {
                if let Some(ref cs) = store.0 {
                    let cookie_header = cs.get_for_request(&url, None, true);
                    if !cookie_header.is_empty() {
                        headers.insert("Cookie".to_string(), cookie_header);
                    }
                }
            }
        }
        s.try_borrow::<SharedCookieStore>()
            .and_then(|s| s.0.clone())
    };

    let url_clone = url.clone();
    let method_for_fallback = method.clone();
    let body_for_fallback = body_opt.clone();
    let headers_for_fallback = headers.clone();

    // Check tokio context. After web_timeout panics, spawn_blocking crashes.
    // Return a graceful error instead of aborting.
    if tokio::runtime::Handle::try_current().is_err() {
        // Tokio context corrupted — return the request result using the shared fetch runtime directly.
        let fetch_result = fetch_rt.block_on(async {
            let m: wreq::Method = method.parse().map_err(|e| format!("bad method: {e}"))?;
            let mut builder = raw_client.request(m, &url_clone).timeout(std::time::Duration::from_millis(timeout_ms as u64));
            let merged_hdrs: HashMap<String, String> = neo_http::headers::fetch_headers().into_iter().collect();
            for (k, v) in &merged_hdrs { builder = builder.header(k.as_str(), v.as_str()); }
            for (k, v) in &headers { builder = builder.header(k.as_str(), v.as_str()); }
            if let Some(b) = body_opt { builder = builder.body(b); }
            let resp = builder.send().await.map_err(|e| format!("fetch: {e}"))?;
            let status = resp.status().as_u16();
            let resp_headers: HashMap<String, String> = resp.headers().iter().map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string())).collect();
            let body_text = resp.text().await.map_err(|e| format!("fetch body: {e}"))?;
            Ok::<_, String>((status, resp_headers, body_text, false))
        });
        let (status, resp_headers, body_text, is_sse) = fetch_result
            .map_err(|e: String| deno_error::JsErrorBox::generic(e))?;
        // Store cookies + build response (simplified)
        if let Some(ref cs) = cookie_store_arc {
            for key in &["set-cookie", "Set-Cookie"] {
                if let Some(val) = resp_headers.get(*key) { cs.store_set_cookie(&url, val); }
            }
        }
        let resp_json = serde_json::json!({ "status": status, "headers": resp_headers, "body": body_text, "is_sse": is_sse });
        return Ok(resp_json.to_string());
    }

    let result = tokio::task::spawn_blocking(move || {
        let m: wreq::Method = method
            .parse()
            .map_err(|e| format!("bad method: {e}"))?;

        let mut builder = raw_client
            .request(m, &url_clone)
            .timeout(std::time::Duration::from_millis(timeout_ms as u64));

        // Merge headers: defaults first, request-specific override.
        let mut merged_hdrs: HashMap<String, String> =
            neo_http::headers::fetch_headers().into_iter().collect();
        for (k, v) in &headers {
            merged_hdrs.insert(k.clone(), v.clone());
        }
        for (k, v) in &merged_hdrs {
            builder = builder.header(k.as_str(), v.as_str());
        }
        if let Some(b) = body_opt {
            builder = builder.body(b);
        }

        // block_on the shared fetch runtime — safe because we're on a
        // std::thread, not inside tokio.
        fetch_rt.block_on(async move {
            let mut resp = builder.send().await
                .map_err(|e| format!("fetch send: {e}"))?;

            let status = resp.status().as_u16();
            let resp_headers: HashMap<String, String> = resp
                .headers()
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                .collect();

            // Detect SSE responses.
            let is_sse = resp_headers
                .get("content-type")
                .map(|ct| ct.contains("text/event-stream") || ct.contains("text/x-sse"))
                .unwrap_or(false);

            let body_text = if is_sse {
                let sse_deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
                let mut body_buf = String::new();
                loop {
                    let remaining = sse_deadline.saturating_duration_since(std::time::Instant::now());
                    if remaining.is_zero() { break; }
                    match tokio::time::timeout(
                        std::time::Duration::from_secs(15).min(remaining),
                        resp.chunk(),
                    ).await {
                        Ok(Ok(Some(chunk))) => {
                            body_buf.push_str(&String::from_utf8_lossy(&chunk));
                            if body_buf.contains("[DONE]") { break; }
                        }
                        Ok(Ok(None)) => break,
                        _ => break,
                    }
                }
                body_buf
            } else {
                // Always read as bytes first to handle brotli correctly.
                let raw_bytes = resp.bytes()
                    .await
                    .map_err(|e| format!("fetch body bytes: {e}"))?;
                if raw_bytes.is_empty() && status == 200 {
                    eprintln!("[op_fetch] 0 raw bytes for 200: {url_clone}");
                }
                let content_encoding = resp_headers
                    .get("content-encoding")
                    .or_else(|| resp_headers.get("Content-Encoding"))
                    .map(|s| s.as_str())
                    .unwrap_or("");
                if content_encoding.contains("br") && !raw_bytes.is_empty() {
                    eprintln!("[op_fetch] brotli decompress {} bytes for {url_clone}", raw_bytes.len());
                    let mut decompressed = Vec::new();
                    let mut reader = brotli::Decompressor::new(&raw_bytes[..], 4096);
                    match std::io::Read::read_to_end(&mut reader, &mut decompressed) {
                        Ok(_) => String::from_utf8_lossy(&decompressed).to_string(),
                        Err(e) => {
                            eprintln!("[op_fetch] brotli fail: {e}, using raw {}", raw_bytes.len());
                            String::from_utf8_lossy(&raw_bytes).to_string()
                        }
                    }
                } else {
                    String::from_utf8_lossy(&raw_bytes).to_string()
                }
            };

            Ok::<_, String>((status, resp_headers, body_text, is_sse))
        })
    }).await
        .map_err(|e| deno_error::JsErrorBox::generic(format!("fetch task: {e}")))?
        .map_err(|e: String| deno_error::JsErrorBox::generic(e))?;

    let (mut status, mut resp_headers, mut body_text, is_sse) = result;

    // Impit fallback: if Cloudflare blocked the request, retry with Chrome 142 TLS fingerprint.
    if status == 403 && crate::chrome_fallback::is_cloudflare_block(&body_text) {
        eprintln!("[neo-impit-fallback] 403 Cloudflare detected for {url}, trying impit (Chrome 142 TLS)...");
        let impit_client = {
            let s = state.borrow();
            s.try_borrow::<SharedImpitClient>().cloned()
        };
        if let Some(impit) = impit_client {
            let impit_clone = impit.0.clone();
            let impit_url = url.clone();
            let impit_method = method_for_fallback.clone();
            let impit_body = body_for_fallback.clone();
            let impit_headers = headers_for_fallback.clone();

            let impit_rt = {
                let s = state.borrow();
                let rt = s.try_borrow::<SharedFetchRuntime>()
                    .ok_or_else(|| deno_error::JsErrorBox::generic("No FetchRuntime for impit fallback"))?;
                rt.0.clone()
            };

            let impit_fetch = tokio::task::spawn_blocking(move || {
                impit_rt.block_on(async move {
                    impit_clone.fetch(
                        &impit_url,
                        &impit_method,
                        impit_body.as_deref(),
                        &impit_headers,
                    ).await
                })
            }).await;

            match impit_fetch {
                Ok(Ok(ir)) => {
                    eprintln!("[neo-impit-fallback] impit response: {} ({} bytes)", ir.status, ir.body.len());
                    status = ir.status;
                    resp_headers = ir.headers;
                    body_text = ir.body;
                }
                Ok(Err(e)) => {
                    eprintln!("[neo-impit-fallback] impit fetch failed: {e}");
                }
                Err(e) => {
                    eprintln!("[neo-impit-fallback] impit task panicked: {e}");
                }
            }
        } else {
            eprintln!("[neo-impit-fallback] no ImpitClient in OpState, skipping");
        }
    }

    // Chrome fallback: if STILL Cloudflare-blocked after impit, retry through Chrome.
    if status == 403 {
        eprintln!("[neo-chrome-fallback] 403 detected for {url}, cloudflare={}", crate::chrome_fallback::is_cloudflare_block(&body_text));
    }
    if status == 403 && crate::chrome_fallback::is_cloudflare_block(&body_text) {
        let fallback = {
            let s = state.borrow();
            s.try_borrow::<crate::chrome_fallback::SharedChromeFallback>().cloned()
        };
        if let Some(fallback) = fallback {
            // Collect cookies for the target domain from our cookie store.
            let cookie_tuples: Option<Vec<(String, String, String)>> = cookie_store_arc.as_ref().and_then(|cs| {
                url::Url::parse(&url).ok().and_then(|u| {
                    u.host_str().map(|domain| {
                        cs.list_for_domain(domain)
                            .into_iter()
                            .map(|c| (c.name.clone(), c.value.clone(), c.domain.clone()))
                            .collect()
                    })
                })
            });

            // Run the Chrome fetch on the shared fetch runtime.
            let fallback_clone = fallback.clone();
            let url_for_chrome = url.clone();
            let method_str = method_for_fallback.clone();
            let headers_for_chrome = headers_for_fallback.clone();
            let body_for_chrome = body_for_fallback.clone();

            let chrome_rt = {
                let s = state.borrow();
                let rt = s.try_borrow::<SharedFetchRuntime>()
                    .ok_or_else(|| deno_error::JsErrorBox::generic("No FetchRuntime for chrome fallback"))?;
                rt.0.clone()
            };

            let chrome_fetch = tokio::task::spawn_blocking(move || {
                chrome_rt.block_on(async move {
                    fallback_clone.fetch_via_chrome(
                        &url_for_chrome,
                        &method_str,
                        body_for_chrome.as_deref(),
                        &headers_for_chrome,
                        cookie_tuples.as_deref(),
                    ).await
                })
            }).await;

            match chrome_fetch {
                Ok(Ok(cr)) => {
                    status = cr.status;
                    resp_headers = cr.headers;
                    body_text = cr.body;
                }
                Ok(Err(e)) => {
                    eprintln!("[neo-chrome-fallback] Chrome fetch failed: {e}");
                    // Keep original 403 result.
                }
                Err(e) => {
                    eprintln!("[neo-chrome-fallback] Chrome task panicked: {e}");
                }
            }
        }
    }

    // Budget slot released automatically by _fetch_guard drop at function end.

    // Store Set-Cookie headers.
    if let Some(ref cs) = cookie_store_arc {
        for key in &["set-cookie", "Set-Cookie"] {
            if let Some(val) = resp_headers.get(*key) {
                cs.store_set_cookie(&url, val);
            }
        }
    }

    // Build JSON response.
    let json = if is_sse {
        let events: Vec<String> = body_text
            .split("\n\n")
            .filter(|e| !e.trim().is_empty())
            .map(|e| {
                e.lines()
                    .filter(|l| l.starts_with("data: "))
                    .map(|l| &l[6..])
                    .collect::<Vec<_>>()
                    .join("")
            })
            .filter(|d| !d.is_empty() && d != "[DONE]")
            .collect();
        serde_json::json!({
            "status": status,
            "body": body_text,
            "headers": resp_headers,
            "sse_events": events,
        })
    } else {
        serde_json::json!({
            "status": status,
            "body": body_text,
            "headers": resp_headers,
        })
    };
    Ok(json.to_string())
}
