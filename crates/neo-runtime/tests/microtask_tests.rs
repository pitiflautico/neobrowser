//! Microtask pipeline diagnostic tests.
//!
//! Tests V8 microtask draining step by step to find EXACTLY where
//! `perform_microtask_checkpoint()` breaks with deno_core 0.311.
//!
//! Run with: cargo test -p neo-runtime --test microtask_tests -- --nocapture

use deno_core::{JsRuntime as DenoJsRuntime, PollEventLoopOptions, RuntimeOptions};

// ─── Helpers ───

/// Create a bare deno_core JsRuntime — no extensions, no happy-dom, no polyfills.
fn bare_runtime() -> DenoJsRuntime {
    DenoJsRuntime::new(RuntimeOptions::default())
}

/// Execute JS and return the string result.
fn eval_string(rt: &mut DenoJsRuntime, code: &str) -> String {
    let result = rt
        .execute_script("<test>", format!("String({})", code))
        .expect("execute_script failed");
    let scope = &mut rt.handle_scope();
    let local = deno_core::v8::Local::new(scope, result);
    local
        .to_string(scope)
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_else(|| "undefined".to_string())
}

/// Execute JS, ignoring result.
fn exec(rt: &mut DenoJsRuntime, code: &str) {
    rt.execute_script("<test>", code.to_string())
        .expect("execute_script failed");
}

// ─── Test 1: Raw V8 microtask checkpoint ───

#[test]
fn t1_raw_v8_microtask_checkpoint() {
    let mut rt = bare_runtime();

    exec(
        &mut rt,
        "globalThis.__x = 'before'; Promise.resolve().then(() => { globalThis.__x = 'after' })",
    );

    let before_checkpoint = eval_string(&mut rt, "globalThis.__x");
    println!("[T1] Before checkpoint: __x = {before_checkpoint}");

    // Explicit microtask checkpoint via V8 isolate
    rt.v8_isolate().perform_microtask_checkpoint();

    let after_checkpoint = eval_string(&mut rt, "globalThis.__x");
    println!("[T1] After checkpoint:  __x = {after_checkpoint}");

    if after_checkpoint == "after" {
        println!("[T1] PASS — perform_microtask_checkpoint() drains .then()");
    } else {
        println!("[T1] FAIL — perform_microtask_checkpoint() did NOT drain .then()");
        println!("[T1]   expected 'after', got '{after_checkpoint}'");
    }
    // Don't assert — we want to see ALL results even if some fail
}

// ─── Test 2: Raw V8 auto-drain (kAuto policy) ───

#[test]
fn t2_v8_auto_drain_policy() {
    let mut rt = bare_runtime();

    // Check the microtask policy
    let policy = rt.v8_isolate().get_microtasks_policy();
    println!("[T2] Microtasks policy: {:?}", policy);

    exec(
        &mut rt,
        "globalThis.__x = 'before'; Promise.resolve().then(() => { globalThis.__x = 'after' })",
    );

    // NO explicit checkpoint — does auto-drain work between execute_script calls?
    let result = eval_string(&mut rt, "globalThis.__x");
    println!("[T2] After 2nd execute_script (no checkpoint): __x = {result}");

    if result == "after" {
        println!("[T2] PASS — auto-drain works between execute_script calls");
    } else {
        println!("[T2] FAIL — auto-drain did NOT fire between execute_script calls");
    }
}

// ─── Test 3: run_event_loop drain ───

#[tokio::test]
async fn t3_run_event_loop_drain() {
    let mut rt = bare_runtime();

    exec(
        &mut rt,
        "globalThis.__x = 'before'; Promise.resolve().then(() => { globalThis.__x = 'after' })",
    );

    let before = eval_string(&mut rt, "globalThis.__x");
    println!("[T3] Before run_event_loop: __x = {before}");

    // Run event loop briefly
    let loop_result = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        rt.run_event_loop(PollEventLoopOptions::default()),
    )
    .await;
    println!("[T3] Event loop result: {:?}", loop_result);

    let after = eval_string(&mut rt, "globalThis.__x");
    println!("[T3] After run_event_loop: __x = {after}");

    if after == "after" {
        println!("[T3] PASS — run_event_loop drains microtasks");
    } else {
        println!("[T3] FAIL — run_event_loop did NOT drain microtasks");
    }
}

// ─── Test 4: queueMicrotask ───

#[test]
fn t4_queue_microtask() {
    let mut rt = bare_runtime();

    exec(
        &mut rt,
        "globalThis.__y = 'before'; queueMicrotask(() => { globalThis.__y = 'after' })",
    );

    // Try checkpoint
    rt.v8_isolate().perform_microtask_checkpoint();
    let after_cp = eval_string(&mut rt, "globalThis.__y");
    println!("[T4] After checkpoint: __y = {after_cp}");

    if after_cp == "after" {
        println!("[T4] PASS — checkpoint drains queueMicrotask");
    } else {
        println!("[T4] FAIL — checkpoint did NOT drain queueMicrotask");
        // Try second execute_script (auto-drain?)
        let after_auto = eval_string(&mut rt, "globalThis.__y");
        println!("[T4] After 2nd eval (auto-drain?): __y = {after_auto}");
    }
}

// ─── Test 5: async function body (sync to first await) ───

#[test]
fn t5_async_body_sync() {
    let mut rt = bare_runtime();

    exec(
        &mut rt,
        "globalThis.__z = 'before'; (async () => { globalThis.__z = 'inside' })()",
    );

    let result = eval_string(&mut rt, "globalThis.__z");
    println!("[T5] After async IIFE (no await): __z = {result}");

    if result == "inside" {
        println!("[T5] PASS — async body runs synchronously up to first await");
    } else {
        println!("[T5] FAIL — async body did NOT run: got '{result}'");
    }
}

// ─── Test 6: await Promise.resolve() ───

#[tokio::test]
async fn t6_await_promise_resolve() {
    let mut rt = bare_runtime();

    exec(
        &mut rt,
        "globalThis.__w = 'before'; (async () => { await Promise.resolve(); globalThis.__w = 'after_await' })()",
    );

    let before_loop = eval_string(&mut rt, "globalThis.__w");
    println!("[T6] Before event loop: __w = {before_loop}");

    // Checkpoint first
    rt.v8_isolate().perform_microtask_checkpoint();
    let after_cp = eval_string(&mut rt, "globalThis.__w");
    println!("[T6] After checkpoint: __w = {after_cp}");

    // If checkpoint didn't do it, try event loop
    if after_cp != "after_await" {
        let _ = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            rt.run_event_loop(PollEventLoopOptions::default()),
        )
        .await;

        let after_loop = eval_string(&mut rt, "globalThis.__w");
        println!("[T6] After run_event_loop: __w = {after_loop}");

        if after_loop == "after_await" {
            println!("[T6] PASS — run_event_loop drained await microtask (checkpoint didn't)");
        } else {
            println!("[T6] FAIL — neither checkpoint nor run_event_loop drained 'await'");
        }
    } else {
        println!("[T6] PASS — checkpoint drained await microtask");
    }
}

// ─── Test 7: With NeoRender extensions ───

#[test]
fn t7_with_neo_extensions() {
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::{JsRuntime as JsRuntimeTrait, RuntimeConfig};

    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();

    // Use execute() for multi-statement code (eval wraps in String() which breaks on semicolons)
    rt.execute(
        "globalThis.__ext_x = 'before'; Promise.resolve().then(() => { globalThis.__ext_x = 'after' })",
    )
    .unwrap();

    // Checkpoint
    rt.runtime.v8_isolate().perform_microtask_checkpoint();

    let result = rt.eval("globalThis.__ext_x").unwrap();
    println!("[T7] With neo extensions — after checkpoint: __ext_x = {result}");

    if result == "after" {
        println!("[T7] PASS — extensions don't break microtask drain");
    } else {
        println!("[T7] FAIL — extensions may break microtask drain: got '{result}'");
    }
}

// ─── Test 8: With bootstrap.js loaded ───

#[test]
fn t8_with_bootstrap() {
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::{JsRuntime as JsRuntimeTrait, RuntimeConfig};

    let mut rt = DenoRuntime::new(&RuntimeConfig::default()).unwrap();

    // set_document_html loads bootstrap.js (on first call)
    rt.set_document_html("<html><body><p>Hello</p></body></html>", "https://example.com")
        .unwrap();

    // Now test microtasks after full bootstrap — use execute() for multi-statement
    rt.execute(
        "globalThis.__boot_x = 'before'; Promise.resolve().then(() => { globalThis.__boot_x = 'after' })",
    )
    .unwrap();

    // Checkpoint
    rt.runtime.v8_isolate().perform_microtask_checkpoint();

    let result = rt.eval("globalThis.__boot_x").unwrap();
    println!("[T8] With bootstrap — after checkpoint: __boot_x = {result}");

    if result == "after" {
        println!("[T8] PASS — bootstrap doesn't break microtask drain");
    } else {
        println!("[T8] FAIL — bootstrap may break microtask drain: got '{result}'");

        // Try run_until_settled
        rt.execute(
            "globalThis.__boot_y = 'before'; Promise.resolve().then(() => { globalThis.__boot_y = 'after' })",
        )
        .unwrap();
        rt.run_until_settled(500).unwrap();
        let result2 = rt.eval("globalThis.__boot_y").unwrap();
        println!("[T8] After run_until_settled: __boot_y = {result2}");
    }
}

// ─── Test 9: Chained microtasks (multi-level) ───

#[test]
fn t9_chained_microtasks() {
    let mut rt = bare_runtime();

    exec(
        &mut rt,
        r#"
        globalThis.__chain = [];
        Promise.resolve()
            .then(() => { globalThis.__chain.push('a') })
            .then(() => { globalThis.__chain.push('b') })
            .then(() => { globalThis.__chain.push('c') });
        "#,
    );

    rt.v8_isolate().perform_microtask_checkpoint();
    let result = eval_string(&mut rt, "JSON.stringify(globalThis.__chain)");
    println!("[T9] Chained .then() after checkpoint: {result}");

    if result == r#"["a","b","c"]"# {
        println!("[T9] PASS — all chained microtasks drained");
    } else {
        println!("[T9] PARTIAL/FAIL — got {result}");
    }
}

// ─── Test 10: Microtask policy inspection ───

#[test]
fn t10_policy_inspection() {
    // Bare runtime
    let mut rt1 = bare_runtime();
    let p1 = rt1.v8_isolate().get_microtasks_policy();
    println!("[T10] Bare runtime policy: {:?}", p1);

    // With extensions
    use neo_runtime::v8::DenoRuntime;
    use neo_runtime::RuntimeConfig;
    let mut rt2 = DenoRuntime::new(&RuntimeConfig::default()).unwrap();
    let p2 = rt2.runtime.v8_isolate().get_microtasks_policy();
    println!("[T10] Neo runtime policy: {:?}", p2);

    println!(
        "[T10] Match: {}",
        if format!("{:?}", p1) == format!("{:?}", p2) {
            "YES — same policy"
        } else {
            "NO — DIFFERENT policies (this could be the bug!)"
        }
    );
}
