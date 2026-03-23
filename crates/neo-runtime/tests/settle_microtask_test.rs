//! Test: does run_until_settled's watchdog pattern break microtask drain?
//!
//! Run: cargo test -p neo-runtime --test settle_microtask_test -- --nocapture

use deno_core::{JsRuntime as DenoJsRuntime, PollEventLoopOptions, RuntimeOptions};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn make_runtime() -> DenoJsRuntime {
    DenoJsRuntime::new(RuntimeOptions {
        extensions: vec![neo_runtime::v8::neo_runtime_ext::init_ops()],
        ..Default::default()
    })
}

fn eval_string(rt: &mut DenoJsRuntime, code: &str) -> String {
    let result = rt
        .execute_script("<test>", format!("String({})", code))
        .expect("execute_script failed");
    let scope = &mut rt.handle_scope();
    let local = deno_core::v8::Local::new(scope, result);
    local.to_string(scope).map(|s| s.to_rust_string_lossy(scope)).unwrap_or_default()
}

fn test_microtask(rt: &mut DenoJsRuntime, label: &str) -> bool {
    let var = format!("__mt_{}", label.replace(['-', ' '], "_"));
    rt.execute_script("<set>", format!(
        "globalThis.{v}='B';Promise.resolve().then(function(){{globalThis.{v}='A'}})", v = var
    )).unwrap();
    let r = eval_string(rt, &format!("globalThis.{}", var));
    let ok = r == "A";
    println!("[{label:40}] {r} -> {}", if ok { "PASS" } else { "**FAIL**" });
    ok
}

/// Test 1: run_event_loop with pending async op + timeout
#[test]
fn settle_t1_event_loop_timeout_with_pending() {
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all().build().unwrap();
    let _guard = tokio_rt.enter();

    let mut rt = make_runtime();
    assert!(test_microtask(&mut rt, "t1-before"));

    // Create pending async op
    rt.execute_script("<pending>",
        "Deno.core.ops.op_microtask_tick()".to_string()
    ).unwrap();

    // run_event_loop with timeout
    tokio_rt.block_on(async {
        let _ = tokio::time::timeout(
            Duration::from_millis(500),
            rt.run_event_loop(PollEventLoopOptions::default()),
        ).await;
    });

    test_microtask(&mut rt, "t1-after-timeout-settle");
}

/// Test 2: watchdog terminate during event loop (EXACT NeoRender pattern)
#[test]
fn settle_t2_watchdog_terminate() {
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all().build().unwrap();

    let mut rt = make_runtime();
    assert!(test_microtask(&mut rt, "t2-before"));

    // Create work that keeps event loop alive via never-resolving promise
    rt.execute_script("<pending>", r#"
        new Promise(function(resolve) {
            // Never resolves — simulates background work
            globalThis.__never_resolve = resolve;
        });
    "#.to_string()).unwrap();

    // Watchdog pattern from run_until_settled
    let isolate_handle = rt.v8_isolate().thread_safe_handle();
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let cancel_clone = cancel_flag.clone();
    let deadline = Instant::now() + Duration::from_millis(200);

    let watchdog = std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(50));
            if cancel_clone.load(Ordering::Relaxed) { return; }
            if Instant::now() >= deadline {
                isolate_handle.terminate_execution();
                return;
            }
        }
    });

    tokio_rt.block_on(async {
        let _ = tokio::time::timeout(
            Duration::from_millis(5000),
            rt.run_event_loop(PollEventLoopOptions::default()),
        ).await;
    });

    cancel_flag.store(true, Ordering::Relaxed);
    let _ = watchdog.join();

    // Cancel termination + recovery (exact pattern from v8_runtime_impl.rs)
    rt.v8_isolate().cancel_terminate_execution();
    match rt.execute_script("<recovery>", "void 0".to_string()) {
        Ok(_) => {}
        Err(_) => {
            rt.v8_isolate().cancel_terminate_execution();
            let _ = rt.execute_script("<recovery2>", "void 0".to_string());
        }
    }

    let ok = test_microtask(&mut rt, "t2-after-watchdog");
    if !ok {
        println!(">>> ROOT CAUSE FOUND!");
        println!(">>> The watchdog terminate_execution + cancel_terminate_execution");
        println!(">>> during run_event_loop breaks V8's kAuto microtask drain.");
        println!(">>> All subsequent execute_script calls no longer auto-drain microtasks.");
    }
}

/// Test 3: just timeout without terminate (no watchdog)
#[test]
fn settle_t3_timeout_only() {
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all().build().unwrap();

    let mut rt = make_runtime();
    assert!(test_microtask(&mut rt, "t3-before"));

    // Create never-resolving promise
    rt.execute_script("<pending>", "new Promise(function(){})".to_string()).unwrap();

    // Timeout without watchdog terminate
    tokio_rt.block_on(async {
        let _ = tokio::time::timeout(
            Duration::from_millis(200),
            rt.run_event_loop(PollEventLoopOptions::default()),
        ).await;
    });

    let ok = test_microtask(&mut rt, "t3-after-timeout-only");
    if !ok {
        println!(">>> Even timeout WITHOUT terminate breaks microtask drain!");
    } else {
        println!(">>> Timeout alone is FINE — the watchdog terminate is the culprit.");
    }
}

/// Test 4: terminate_execution + cancel WITHOUT event loop
#[test]
fn settle_t4_terminate_no_event_loop() {
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all().build().unwrap();
    let _guard = tokio_rt.enter();

    let mut rt = make_runtime();
    assert!(test_microtask(&mut rt, "t4-before"));

    // Just terminate + cancel, no event loop
    let handle = rt.v8_isolate().thread_safe_handle();
    handle.terminate_execution();
    rt.v8_isolate().cancel_terminate_execution();
    let _ = rt.execute_script("<recovery>", "void 0".to_string());

    let ok = test_microtask(&mut rt, "t4-after-terminate-no-loop");
    if !ok {
        println!(">>> Even terminate WITHOUT event loop breaks drain!");
    }
}
