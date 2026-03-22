//! V8 operations — bridge between JavaScript and Rust.
//!
//! All ops are sync to avoid deno_core async RefCell panics.
//! HTTP fetches run on dedicated threads. Timers use thread::sleep.

use crate::scheduler::{FetchBudget, TaskTracker, TimerBudget, TimerState};
use deno_core::op2;
use deno_core::OpState;
use neo_http::{HttpClient, HttpRequest, RequestContext, RequestKind, WebStorage};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

/// Shared HTTP client stored in OpState for fetch ops.
pub struct SharedHttpClient(pub Arc<dyn HttpClient>);

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

/// Fetch a URL. Sync op — runs HTTP on a dedicated thread.
///
/// Delegates to the `HttpClient` trait object in OpState.
/// Skips telemetry/analytics URLs with a fake 200 response.
/// Respects `FetchBudget` concurrency limits and abort flag.
#[op2]
#[string]
pub fn op_fetch(
    state: Rc<RefCell<OpState>>,
    #[string] url: String,
    #[string] method: String,
    #[string] body: String,
    #[string] headers_json: String,
) -> Result<String, deno_core::error::AnyError> {
    if should_skip_url(&url) {
        return Ok(r#"{"status":200,"body":"","headers":{}}"#.to_string());
    }

    // Check fetch budget before proceeding.
    let (client, timeout_ms, budget) = {
        let s = state.borrow();

        // Check abort flag and concurrency budget.
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
            .try_borrow::<SharedHttpClient>()
            .ok_or_else(|| deno_core::error::generic_error("No HttpClient in OpState"))?;

        (handle.0.clone(), timeout, fetch_budget)
    };

    let headers = parse_headers(&headers_json);
    let body_opt = if body.is_empty() { None } else { Some(body) };

    let req = HttpRequest {
        method: method.clone(),
        url: url.clone(),
        headers,
        body: body_opt,
        context: RequestContext {
            kind: RequestKind::Fetch,
            initiator: "script".to_string(),
            referrer: None,
            frame_id: None,
            top_level_url: None,
        },
        timeout_ms: timeout_ms as u64,
    };

    // Run on dedicated thread to avoid async conflicts.
    let result = std::thread::spawn(move || client.request(&req))
        .join()
        .map_err(|_| deno_core::error::generic_error("fetch thread panicked"));

    // Always release the budget slot when done.
    if let Some(ref fb) = budget {
        fb.finish_fetch();
    }

    match result? {
        Ok(resp) => {
            let json = serde_json::json!({
                "status": resp.status,
                "body": resp.body,
                "headers": resp.headers,
            });
            Ok(json.to_string())
        }
        Err(e) => Err(deno_core::error::generic_error(e.to_string())),
    }
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
