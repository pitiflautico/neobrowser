//! V8 operations — bridge between JavaScript and Rust.
//!
//! Fetch ops use async I/O on the existing tokio runtime (Chromium-style:
//! single event loop, shared connection pool, HTTP/2 multiplexing).
//! Timers use thread::sleep.

use crate::scheduler::{FetchBudget, TaskTracker, TimerBudget, TimerState};

/// No-op async op that resolves immediately.
/// Used to force deno_core's event loop to do a full cycle (including
/// microtask checkpoint) when there are no other pending ops.
#[op2(async)]
pub async fn op_microtask_tick() {
    // Resolves immediately — no tokio dependency
}
use deno_core::op2;
use deno_core::OpState;
use neo_http::{CookieStore, HttpClient, WebStorage};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

/// Shared HTTP client stored in OpState for fetch ops.
pub struct SharedHttpClient(pub Arc<dyn HttpClient>);

/// Shared cookie store for auto-injecting cookies into fetch requests.
///
/// Wraps `Option<Arc<dyn CookieStore>>` so it can be absent (no cookie store attached).
pub struct SharedCookieStore(pub Option<Arc<dyn CookieStore>>);

/// Console log buffer — captures JS console output.
#[derive(Default, Clone)]
pub struct ConsoleBuffer {
    /// Captured log messages.
    pub messages: Arc<std::sync::Mutex<Vec<String>>>,
}

/// Web storage state: wraps a `WebStorage` trait object + current origin.
///
/// Falls back to an in-memory HashMap when no `WebStorage` backend is provided
/// (preserves backward compatibility with code that used `StorageState::default()`).
#[derive(Clone)]
pub struct StorageState {
    /// Backend (SQLite, in-memory mock, etc.).
    pub backend: Arc<dyn WebStorage>,
    /// Current storage origin (set on navigation, e.g. "https://example.com").
    pub origin: String,
}

impl Default for StorageState {
    fn default() -> Self {
        Self {
            backend: Arc::new(neo_http::InMemoryWebStorage::new()),
            origin: String::new(),
        }
    }
}
/// Shared scheduler config values accessible from ops.
#[derive(Clone)]
pub struct OpsSchedulerConfig {
    /// Max ticks per setInterval (exposed to JS).
    pub interval_max_ticks: u32,
}

impl Default for OpsSchedulerConfig {
    fn default() -> Self {
        Self {
            interval_max_ticks: 20,
        }
    }
}

// ─── Streaming Fetch (G2) ───

/// Store for active streaming HTTP responses.
///
/// Keeps `wreq::Response` objects alive between `op_fetch_start` and
/// subsequent `op_fetch_read_chunk` calls. Each stream gets a unique u32 id.
pub struct StreamStore {
    streams: HashMap<u32, ActiveStream>,
    next_id: u32,
}

struct ActiveStream {
    response: Option<wreq::Response>,
    /// Tracked for future TTL-based cleanup of abandoned streams.
    #[allow(dead_code)]
    created_at: std::time::Instant,
}

impl Default for StreamStore {
    fn default() -> Self {
        Self {
            streams: HashMap::new(),
            next_id: 1,
        }
    }
}

/// Shared raw wreq client for streaming fetch ops.
///
/// Stored separately from `SharedHttpClient` because streaming needs the raw
/// `wreq::Client` to get an `wreq::Response` without reading the body.
pub struct SharedRquestClient(pub Arc<wreq::Client>);

/// Start a streaming fetch — sends request, returns headers + stream_id.
///
/// The response body stays open for incremental reading via `op_fetch_read_chunk`.
/// Uses the same URL-skip logic, cookie injection, and header merging as `op_fetch`.
#[op2(async)]
#[string]
pub async fn op_fetch_start(
    state: Rc<RefCell<OpState>>,
    #[string] url: String,
    #[string] method: String,
    #[string] body: String,
    #[string] headers_json: String,
) -> Result<String, deno_core::error::AnyError> {
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

    // Check fetch budget.
    let (raw_client, timeout_ms, budget) = {
        let s = state.borrow();
        let fetch_budget = s.try_borrow::<FetchBudget>().cloned();
        if let Some(ref fb) = fetch_budget {
            if fb.is_aborted() {
                return Err(deno_core::error::generic_error("fetch aborted by watchdog"));
            }
            if !fb.start_fetch() {
                return Err(deno_core::error::generic_error(
                    "fetch budget exceeded: too many concurrent requests",
                ));
            }
        }
        let timeout = fetch_budget
            .as_ref()
            .map(|fb| fb.per_request_timeout_ms())
            .unwrap_or(5000);
        let handle = s
            .try_borrow::<SharedRquestClient>()
            .ok_or_else(|| deno_core::error::generic_error("No RquestClient in OpState"))?;
        (handle.0.clone(), timeout, fetch_budget)
    };

    let mut headers = parse_headers(&headers_json);
    let body_opt = if body.is_empty() { None } else { Some(body) };

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
        .map_err(|e| deno_core::error::generic_error(format!("bad method: {e}")))?;
    let mut builder = raw_client
        .request(m, &url)
        .timeout(std::time::Duration::from_millis(timeout_ms as u64));

    // Apply merged headers (classification defaults + request-specific).
    let fetch_headers = neo_http::headers::fetch_headers();
    for (k, v) in &fetch_headers {
        builder = builder.header(k.as_str(), v.as_str());
    }
    for (k, v) in &headers {
        builder = builder.header(k.as_str(), v.as_str());
    }
    if let Some(b) = body_opt {
        builder = builder.body(b);
    }

    let resp = builder
        .send()
        .await
        .map_err(|e| deno_core::error::generic_error(format!("fetch_start send: {e}")))?;

    // Release budget slot (connection established, headers received).
    if let Some(ref fb) = budget {
        fb.finish_fetch();
    }

    let status = resp.status().as_u16();
    let resp_headers: HashMap<String, String> = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let resp_url = resp.uri().to_string();

    // Store Set-Cookie headers.
    if let Some(ref cs) = cookie_store_arc {
        for key in &["set-cookie", "Set-Cookie"] {
            if let Some(val) = resp_headers.get(*key) {
                cs.store_set_cookie(&url, val);
            }
        }
    }

    // Store response for streaming reads.
    let stream_id = {
        let mut s = state.borrow_mut();
        let store = s.borrow_mut::<StreamStore>();
        let id = store.next_id;
        store.next_id += 1;
        store.streams.insert(
            id,
            ActiveStream {
                response: Some(resp),
                created_at: std::time::Instant::now(),
            },
        );
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
#[op2(async)]
#[string]
pub async fn op_fetch_read_chunk(
    state: Rc<RefCell<OpState>>,
    #[smi] stream_id: u32,
) -> Result<String, deno_core::error::AnyError> {
    // Extract the response from the store — MUST NOT hold borrow across await.
    let mut resp = {
        let mut s = state.borrow_mut();
        let store = s.borrow_mut::<StreamStore>();
        let stream = store
            .streams
            .get_mut(&stream_id)
            .ok_or_else(|| deno_core::error::generic_error("stream not found"))?;
        stream
            .response
            .take()
            .ok_or_else(|| deno_core::error::generic_error("stream already reading"))?
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
            Err(deno_core::error::generic_error(format!(
                "chunk read error: {e}"
            )))
        }
        Err(_) => {
            // Timeout — clean up.
            let mut s = state.borrow_mut();
            let store = s.borrow_mut::<StreamStore>();
            store.streams.remove(&stream_id);
            Err(deno_core::error::generic_error("chunk read timeout (30s)"))
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
pub struct SharedFetchRuntime(pub Arc<tokio::runtime::Runtime>);

/// Chromium-style fetch — shared network runtime, shared connection pool.
///
/// Uses spawn_blocking to keep the fetch off deno_core's event loop (so settle
/// works correctly), but the blocking thread dispatches the actual HTTP request
/// on a shared tokio runtime instead of creating a new one per fetch.
///
/// Being async is CRITICAL for SPA correctness: fetch().then(cb) must
/// resolve cb as a microtask in a FUTURE event loop tick, not the current one.
#[op2(async)]
#[string]
pub async fn op_fetch(
    state: Rc<RefCell<OpState>>,
    #[string] url: String,
    #[string] method: String,
    #[string] body: String,
    #[string] headers_json: String,
) -> Result<String, deno_core::error::AnyError> {
    if should_skip_url(&url) {
        tokio::task::yield_now().await;
        return Ok(r#"{"status":200,"body":"","headers":{}}"#.to_string());
    }

    // Check fetch budget before proceeding.
    let (raw_client, timeout_ms, budget, fetch_rt) = {
        let s = state.borrow();

        let fetch_budget = s.try_borrow::<FetchBudget>().cloned();
        if let Some(ref fb) = fetch_budget {
            if fb.is_aborted() {
                return Err(deno_core::error::generic_error("fetch aborted by watchdog"));
            }
            if !fb.start_fetch() {
                return Err(deno_core::error::generic_error(
                    "fetch budget exceeded: too many concurrent requests",
                ));
            }
        }

        let timeout = fetch_budget
            .as_ref()
            .map(|fb| fb.per_request_timeout_ms())
            .unwrap_or(5000);

        let handle = s
            .try_borrow::<SharedRquestClient>()
            .ok_or_else(|| deno_core::error::generic_error("No RquestClient in OpState"))?;

        let rt = s
            .try_borrow::<SharedFetchRuntime>()
            .ok_or_else(|| deno_core::error::generic_error("No FetchRuntime in OpState"))?;

        (handle.0.clone(), timeout, fetch_budget, rt.0.clone())
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

    // Run fetch on a dedicated thread using the shared fetch runtime.
    // Uses std::thread::spawn + fetch_rt.block_on (NOT tokio::spawn_blocking
    // which would try to use the deno runtime and cause nested-block_on panics).
    // The shared fetch_rt provides connection pooling across all fetches.
    // spawn_blocking moves us to a tokio blocking thread where block_on
    // on the SEPARATE fetch_rt is safe (no nested block_on on same runtime).
    // Use tokio::task::spawn_blocking which properly integrates with deno_core's
    // event loop — the blocking task wakes up the async poller when done.
    // Inside, we use fetch_rt.block_on() on the SEPARATE shared fetch runtime.
    // This is safe because spawn_blocking runs on a dedicated blocking thread pool,
    // not inside the current_thread runtime's async executor.
    let result = tokio::task::spawn_blocking(move || {
        let m: wreq::Method = method
            .parse()
            .map_err(|e| format!("bad method: {e}"))?;

        let mut builder = raw_client
            .request(m, &url_clone)
            .timeout(std::time::Duration::from_millis(timeout_ms as u64));

        let fetch_hdrs = neo_http::headers::fetch_headers();
        for (k, v) in &fetch_hdrs {
            builder = builder.header(k.as_str(), v.as_str());
        }
        for (k, v) in &headers {
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
                resp.text().await.map_err(|e| format!("fetch body: {e}"))?
            };

            Ok::<_, String>((status, resp_headers, body_text, is_sse))
        })
    }).await
        .map_err(|e| deno_core::error::generic_error(format!("fetch task: {e}")))?
        .map_err(|e: String| deno_core::error::generic_error(e))?;

    let (status, resp_headers, body_text, is_sse) = result;

    // Release budget slot.
    if let Some(ref fb) = budget {
        fb.finish_fetch();
    }

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

/// Timer — sync with nested clamping per the HTML spec and abort support.
///
/// Applies nested clamping (depth >= 5 → min 4 ms), then caps at 10 ms.
/// Checks the abort flag before sleeping; returns `false` if aborted.
#[op2(fast)]
pub fn op_timer(state: Rc<RefCell<OpState>>, #[smi] ms: u32) -> bool {
    let s = state.borrow();

    // Check abort flag first — bail if watchdog cancelled timers.
    if let Some(ts) = s.try_borrow::<TimerState>() {
        if ts.is_aborted() {
            return false;
        }
        let depth = ts.nesting_depth();
        let effective = ts.effective_delay(ms, depth);
        let delay = if effective == 0 { 0 } else { effective.clamp(1, 10) };
        // Release borrow before sleeping.
        drop(s);
        if delay > 0 {
            std::thread::sleep(std::time::Duration::from_millis(delay as u64));
        }
    } else {
        // Fallback when no TimerState is installed (backward compat).
        let delay = if ms == 0 { 0 } else { ms.clamp(1, 10) };
        drop(s);
        if delay > 0 {
            std::thread::sleep(std::time::Duration::from_millis(delay as u64));
        }
    }
    true
}

/// Register a new timer in the task tracker.
///
/// Called by JS setTimeout/setInterval to signal pending async work.
/// Returns false if the timer budget is exhausted.
#[op2(fast)]
pub fn op_timer_register(state: Rc<RefCell<OpState>>) -> bool {
    let s = state.borrow();
    // Check budget first
    if let Some(budget) = s.try_borrow::<TimerBudget>() {
        if budget.is_exhausted() {
            return false;
        }
    }
    if let Some(tracker) = s.try_borrow::<TaskTracker>() {
        tracker.add_timer();
    }
    true
}

/// Signal that a timer callback has fired.
///
/// Decrements the timer count and consumes one tick from the budget.
/// Returns false if the budget is now exhausted (interval should stop).
#[op2(fast)]
pub fn op_timer_fire(state: Rc<RefCell<OpState>>) -> bool {
    let s = state.borrow();
    if let Some(tracker) = s.try_borrow::<TaskTracker>() {
        tracker.resolve_timer();
    }
    // Consume budget tick
    if let Some(budget) = s.try_borrow::<TimerBudget>() {
        return budget.tick();
    }
    true
}

/// Get the configured interval max ticks.
#[op2(fast)]
pub fn op_scheduler_config(state: Rc<RefCell<OpState>>) -> u32 {
    let s = state.borrow();
    if let Some(cfg) = s.try_borrow::<OpsSchedulerConfig>() {
        cfg.interval_max_ticks
    } else {
        20
    }
}

/// Get a value from localStorage.
#[op2]
#[string]
pub fn op_storage_get(
    state: Rc<RefCell<OpState>>,
    #[string] key: String,
) -> Result<String, deno_core::error::AnyError> {
    let s = state.borrow();
    let storage = s
        .try_borrow::<StorageState>()
        .ok_or_else(|| deno_core::error::generic_error("No StorageState"))?;
    let val = storage
        .backend
        .get(&storage.origin, &key)
        .unwrap_or_default();
    Ok(val)
}
/// Set a value in localStorage.
#[op2(fast)]
pub fn op_storage_set(
    state: Rc<RefCell<OpState>>,
    #[string] key: String,
    #[string] value: String,
) -> Result<(), deno_core::error::AnyError> {
    let s = state.borrow();
    let storage = s
        .try_borrow::<StorageState>()
        .ok_or_else(|| deno_core::error::generic_error("No StorageState"))?;
    storage.backend.set(&storage.origin, &key, &value);
    Ok(())
}
/// Remove a key from localStorage.
#[op2(fast)]
pub fn op_storage_remove(
    state: Rc<RefCell<OpState>>,
    #[string] key: String,
) -> Result<(), deno_core::error::AnyError> {
    let s = state.borrow();
    let storage = s
        .try_borrow::<StorageState>()
        .ok_or_else(|| deno_core::error::generic_error("No StorageState"))?;
    storage.backend.remove(&storage.origin, &key);
    Ok(())
}
/// Capture console.log output from JavaScript.
#[op2(fast)]
pub fn op_console_log(state: Rc<RefCell<OpState>>, #[string] msg: String) {
    // Print errors/warnings to stderr for debugging
    if msg.starts_with("[error]") || msg.starts_with("[warn]") || msg.starts_with("[script-error]") {
        eprintln!("[js] {}", &msg[..msg.len().min(300)]);
    }
    let s = state.borrow();
    if let Some(buf) = s.try_borrow::<ConsoleBuffer>() {
        let mut messages = buf.messages.lock().expect("console buffer lock poisoned");
        messages.push(msg);
    }
}

/// Check if a URL should be skipped (telemetry, analytics, tracking).
fn should_skip_url(url: &str) -> bool {
    const SKIP_PATTERNS: &[&str] = &[
        "telemetry",
        "analytics",
        "tracking",
        "beacon",
        "sentry",
        "newrelic",
        "amplitude",
        "segment.",
        "hotjar",
        "googletagmanager",
        "doubleclick",
        "/pixel",
        "/collect",
        "adservice",
        "facebook.com/tr",
        "bat.bing.com",
    ];
    SKIP_PATTERNS.iter().any(|p| url.contains(p))
}

/// Parse JSON headers string into HashMap.
fn parse_headers(json: &str) -> HashMap<String, String> {
    if json.is_empty() {
        return HashMap::new();
    }
    serde_json::from_str(json).unwrap_or_default()
}

// ─── Browser Shim Ops ───

/// Queue of navigation requests from JS (form.submit, location.href, window.open).
///
/// The engine drains this queue after every interaction to handle
/// client-side navigation attempts.
#[derive(Default, Clone)]
pub struct NavigationQueue {
    requests: Arc<Mutex<Vec<String>>>,
}

impl NavigationQueue {
    /// Push a new navigation request (called from JS via op_navigation_request).
    pub fn push(&self, req: String) {
        if let Ok(mut q) = self.requests.lock() {
            q.push(req);
        }
    }

    /// Drain all pending navigation requests. Returns empty vec if none.
    pub fn drain(&self) -> Vec<String> {
        if let Ok(mut q) = self.requests.lock() {
            q.drain(..).collect()
        } else {
            vec![]
        }
    }

    /// Check if there are pending navigation requests.
    pub fn has_pending(&self) -> bool {
        if let Ok(q) = self.requests.lock() {
            !q.is_empty()
        } else {
            false
        }
    }
}

/// Cookie state for `document.cookie` access, backed by a simple in-process store.
///
/// Cookies are stored per-origin. The origin is set when the page navigates.
#[derive(Clone)]
pub struct CookieState {
    cookies: Arc<Mutex<HashMap<String, String>>>,
    origin: String,
}

impl CookieState {
    /// Create a new cookie state for the given origin.
    pub fn new(origin: &str) -> Self {
        Self {
            cookies: Arc::new(Mutex::new(HashMap::new())),
            origin: origin.to_string(),
        }
    }

    /// Get the cookie string for `document.cookie` getter.
    pub fn get_cookie_string(&self) -> String {
        if let Ok(cookies) = self.cookies.lock() {
            cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; ")
        } else {
            String::new()
        }
    }

    /// Parse and store a `Set-Cookie`-style string from `document.cookie` setter.
    ///
    /// Only the name=value part is stored; attributes (Path, Domain, etc.)
    /// are ignored since we operate in a single-origin context.
    pub fn set_from_string(&self, cookie_str: &str) {
        // cookie_str format: "name=value; Path=/; Domain=..."
        // We only care about the name=value part (first segment before ';')
        let name_value = cookie_str.split(';').next().unwrap_or("");
        if let Some((name, value)) = name_value.split_once('=') {
            let name = name.trim().to_string();
            let value = value.trim().to_string();
            if !name.is_empty() {
                if let Ok(mut cookies) = self.cookies.lock() {
                    cookies.insert(name, value);
                }
            }
        }
    }

    /// Set the origin (called on navigation).
    pub fn set_origin(&mut self, origin: &str) {
        self.origin = origin.to_string();
    }

    /// Get the current origin.
    pub fn origin(&self) -> &str {
        &self.origin
    }
}

impl Default for CookieState {
    fn default() -> Self {
        Self {
            cookies: Arc::new(Mutex::new(HashMap::new())),
            origin: String::new(),
        }
    }
}

/// Capture a navigation request from JS (form.submit, location.href, etc.)
///
/// Stores the request JSON in the navigation queue for the engine to
/// pick up after script execution or interaction completes.
#[op2]
#[string]
pub fn op_navigation_request(state: Rc<RefCell<OpState>>, #[string] request_json: String) -> String {
    let s = state.borrow();
    if let Some(nav_queue) = s.try_borrow::<NavigationQueue>() {
        nav_queue.push(request_json);
    }
    "ok".to_string()
}

/// Get cookies for current origin (called by document.cookie getter).
#[op2]
#[string]
pub fn op_cookie_get(state: Rc<RefCell<OpState>>) -> String {
    let s = state.borrow();
    if let Some(cookies) = s.try_borrow::<CookieState>() {
        cookies.get_cookie_string()
    } else {
        String::new()
    }
}

/// Set a cookie (called by document.cookie setter).
#[op2(fast)]
pub fn op_cookie_set(state: Rc<RefCell<OpState>>, #[string] cookie_str: String) {
    let s = state.borrow();
    if let Some(cookies) = s.try_borrow::<CookieState>() {
        cookies.set_from_string(&cookie_str);
    }
}

/// Get cookies for a given URL from the shared cookie store.
///
/// Called from JS as a fallback when `__neorender_cookies` is empty.
/// Returns "name=val; name2=val2" format or empty string.
#[op2]
#[string]
pub fn op_cookie_get_for_url(state: Rc<RefCell<OpState>>, #[string] url: String) -> String {
    let s = state.borrow();
    if let Some(store) = s.try_borrow::<SharedCookieStore>() {
        if let Some(ref cs) = store.0 {
            return cs.get_for_request(&url, None, true);
        }
    }
    String::new()
}

/// Minimal async op — tests async op integration.
#[op2(async)]
pub async fn op_yield() -> () {
}

/// Async sleep — tests tokio reactor availability.
#[op2(async)]
pub async fn op_sleep_ms(#[smi] ms: u32) -> () {
    if ms > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(ms as u64)).await;
    }
}

// ─── SHA-256 Proof-of-Work solver (native speed) ───

/// SHA-256 proof-of-work solver. Returns JSON with nonce and hash.
/// Used for ChatGPT's anti-bot challenge.
#[op2]
#[string]
pub fn op_pow_solve(
    #[string] seed: String,
    #[string] difficulty: String,
    #[smi] max_iters: u32,
) -> String {
    let t0 = std::time::Instant::now();
    let max = if max_iters == 0 { 500_000 } else { max_iters };

    for i in 0..max {
        let input = format!("{}{}", seed, i);
        let hash = sha256_hex(input.as_bytes());
        if hash[..difficulty.len()] <= *difficulty {
            let elapsed = t0.elapsed();
            return serde_json::json!({
                "found": true,
                "nonce": i,
                "hash": hash,
                "elapsed_ms": elapsed.as_millis() as u64,
            }).to_string();
        }
    }
    serde_json::json!({
        "found": false,
        "elapsed_ms": t0.elapsed().as_millis() as u64,
    }).to_string()
}

fn sha256_hex(data: &[u8]) -> String {
    let hash = sha256(data);
    let mut hex = String::with_capacity(64);
    for b in &hash {
        use std::fmt::Write;
        write!(hex, "{:02x}", b).unwrap();
    }
    hex
}

fn sha256(data: &[u8]) -> [u8; 32] {
    let k: [u32; 64] = [
        0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,
        0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,
        0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,
        0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,
        0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,
        0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,
        0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,
        0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667,0xbb67ae85,0x3c6ef372,0xa54ff53a,
        0x510e527f,0x9b05688c,0x1f83d9ab,0x5be0cd19,
    ];
    let bit_len = (data.len() as u64) * 8;
    let pad_len = ((56u64.wrapping_sub(data.len() as u64 + 1) % 64) + 64) % 64;
    let total = data.len() as u64 + 1 + pad_len + 8;
    let mut padded = vec![0u8; total as usize];
    padded[..data.len()].copy_from_slice(data);
    padded[data.len()] = 0x80;
    padded[total as usize - 8..].copy_from_slice(&bit_len.to_be_bytes());
    for chunk in padded.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[i*4], chunk[i*4+1], chunk[i*4+2], chunk[i*4+3]]);
        }
        for i in 16..64 {
            let s0 = w[i-15].rotate_right(7) ^ w[i-15].rotate_right(18) ^ (w[i-15] >> 3);
            let s1 = w[i-2].rotate_right(17) ^ w[i-2].rotate_right(19) ^ (w[i-2] >> 10);
            w[i] = w[i-16].wrapping_add(s0).wrapping_add(w[i-7]).wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d) = (h[0], h[1], h[2], h[3]);
        let (mut e, mut f, mut g, mut hh) = (h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(k[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g; g = f; f = e; e = d.wrapping_add(t1);
            d = c; c = b; b = a; a = t1.wrapping_add(t2);
        }
        h[0]=h[0].wrapping_add(a); h[1]=h[1].wrapping_add(b);
        h[2]=h[2].wrapping_add(c); h[3]=h[3].wrapping_add(d);
        h[4]=h[4].wrapping_add(e); h[5]=h[5].wrapping_add(f);
        h[6]=h[6].wrapping_add(g); h[7]=h[7].wrapping_add(hh);
    }
    let mut result = [0u8; 32];
    for i in 0..8 {
        result[i*4..i*4+4].copy_from_slice(&h[i].to_be_bytes());
    }
    result
}
