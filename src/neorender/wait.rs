//! Wait-for-condition — Rust-side polling for wait conditions.
//!
//! Polls JS helpers injected via js/wait.js until condition is met or timeout.

use deno_core::JsRuntime;

/// Wait until a CSS selector matches an element in the DOM.
/// Returns true if found before timeout, false if timed out.
pub async fn wait_for_selector(runtime: &mut JsRuntime, selector: &str, timeout_ms: u64) -> Result<bool, String> {
    let start = std::time::Instant::now();
    let escaped = selector.replace('\'', "\\'").replace('\\', "\\\\");
    let js_check = format!("!!document.querySelector('{}')", escaped);

    loop {
        let result = eval_bool(runtime, &js_check)?;
        if result {
            return Ok(true);
        }
        if start.elapsed().as_millis() as u64 >= timeout_ms {
            return Ok(false);
        }
        // Run event loop briefly to let JS timers/promises execute
        super::v8_runtime::run_event_loop(runtime, 100).await.ok();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

/// Wait until specific text appears anywhere in the page body.
pub async fn wait_for_text(runtime: &mut JsRuntime, text: &str, timeout_ms: u64) -> Result<bool, String> {
    let start = std::time::Instant::now();
    let escaped = text.replace('\\', "\\\\").replace('\'', "\\'");
    let js_check = format!("(document.body?.textContent || '').includes('{}')", escaped);

    loop {
        let result = eval_bool(runtime, &js_check)?;
        if result {
            return Ok(true);
        }
        if start.elapsed().as_millis() as u64 >= timeout_ms {
            return Ok(false);
        }
        super::v8_runtime::run_event_loop(runtime, 100).await.ok();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

/// Wait until the DOM stabilizes (children count unchanged for `stable_ms`).
/// Default stable window: 500ms.
pub async fn wait_for_stable(runtime: &mut JsRuntime, timeout_ms: u64) -> Result<bool, String> {
    let start = std::time::Instant::now();
    let stable_ms: u64 = 500;
    let js_count = "document.body ? document.body.children.length : 0";

    let mut last_count = eval_number(runtime, js_count)?;
    let mut stable_since = std::time::Instant::now();

    loop {
        super::v8_runtime::run_event_loop(runtime, 100).await.ok();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let current = eval_number(runtime, js_count)?;
        if current != last_count {
            last_count = current;
            stable_since = std::time::Instant::now();
        }

        if stable_since.elapsed().as_millis() as u64 >= stable_ms {
            return Ok(true);
        }
        if start.elapsed().as_millis() as u64 >= timeout_ms {
            return Ok(false);
        }
    }
}

// ─── Helpers ───

fn eval_bool(runtime: &mut JsRuntime, js: &str) -> Result<bool, String> {
    let result = runtime.execute_script("<neo:wait>", js.to_string())
        .map_err(|e| format!("wait eval error: {e}"))?;
    let scope = &mut runtime.handle_scope();
    let local = deno_core::v8::Local::new(scope, result);
    Ok(local.boolean_value(scope))
}

fn eval_number(runtime: &mut JsRuntime, js: &str) -> Result<i64, String> {
    let result = runtime.execute_script("<neo:wait>", js.to_string())
        .map_err(|e| format!("wait eval error: {e}"))?;
    let scope = &mut runtime.handle_scope();
    let local = deno_core::v8::Local::new(scope, result);
    Ok(local.integer_value(scope).unwrap_or(0))
}
