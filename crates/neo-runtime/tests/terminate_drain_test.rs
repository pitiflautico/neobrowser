//! DEFINITIVE TEST: does terminate_execution break microtask auto-drain?
//! We FORCE the watchdog to fire by creating truly infinite work.
//!
//! Run: cargo test -p neo-runtime --test terminate_drain_test -- --nocapture

use deno_core::{JsRuntime as DenoJsRuntime, PollEventLoopOptions, RuntimeOptions};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn make_rt() -> DenoJsRuntime {
    DenoJsRuntime::new(RuntimeOptions {
        extensions: vec![neo_runtime::v8::neo_runtime_ext::init()],
        ..Default::default()
    })
}

fn eval_string(rt: &mut DenoJsRuntime, code: &str) -> String {
    let result = rt
        .execute_script("<test>", format!("String({})", code))
        .expect("execute_script failed");
    let context = rt.main_context();
    deno_core::v8::scope!(scope, rt.v8_isolate());
    let context = deno_core::v8::Local::new(scope, context);
    let scope = &mut deno_core::v8::ContextScope::new(scope, context);
    let local = deno_core::v8::Local::new(scope, result);
    local.to_string(scope).map(|s| s.to_rust_string_lossy(scope)).unwrap_or_default()
}

fn test_microtask(rt: &mut DenoJsRuntime, tokio_rt: &tokio::runtime::Runtime, label: &str) -> bool {
    let var = format!("__mt_{}", label.replace(['-', ' '], "_"));
    rt.execute_script("<set>", format!(
        "globalThis.{v}='B';Promise.resolve().then(function(){{globalThis.{v}='A'}})", v = var
    )).unwrap();
    let _ = tokio_rt.block_on(rt.run_event_loop(PollEventLoopOptions::default()));
    let r = eval_string(rt, &format!("globalThis.{}", var));
    let ok = r == "A";
    println!("[{label:40}] {r} -> {}", if ok { "PASS" } else { "**FAIL**" });
    ok
}

/// Create infinite JS work that WILL trigger watchdog terminate
#[test]
fn terminate_breaks_microtask_drain() {
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all().build().unwrap();

    let mut rt = make_rt();
    assert!(test_microtask(&mut rt, &tokio_rt, "before-terminate"));

    // Use op_timer (sync sleep) to simulate heavy work.
    // op_timer(1) sleeps 1ms. 500 of these = 500ms > 200ms watchdog.
    // But op_timer is sync so execute_script blocks...
    // Instead: create a recursive setTimeout chain that keeps the event
    // loop busy via Deno's WebTimers (if available) or our custom setTimeout.
    // The key: we need pending work in the event loop for >200ms.
    rt.execute_script("<work>", r#"
        // Create many promises that chain via .then — this creates
        // work that run_event_loop processes one tick at a time
        globalThis.__work_done = false;
        var chain = Promise.resolve(0);
        for (var i = 0; i < 50000; i++) {
            chain = chain.then(function(v) { return v + 1; });
        }
        chain.then(function(v) {
            globalThis.__work_done = true;
            globalThis.__chain_result = v;
        });
    "#.to_string()).unwrap();

    // Watchdog with short deadline (200ms) — WILL fire
    let isolate_handle = rt.v8_isolate().thread_safe_handle();
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let cancel_clone = cancel_flag.clone();
    let deadline = Instant::now() + Duration::from_millis(200);

    let watchdog = std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(50));
            if cancel_clone.load(Ordering::Relaxed) {
                eprintln!("[test-watchdog] cancelled (normal exit)");
                return false;
            }
            if Instant::now() >= deadline {
                eprintln!("[test-watchdog] FIRING terminate_execution!");
                isolate_handle.terminate_execution();
                return true;
            }
        }
    });

    // Run event loop — watchdog will terminate after 200ms
    tokio_rt.block_on(async {
        let _ = tokio::time::timeout(
            Duration::from_millis(5000),
            rt.run_event_loop(PollEventLoopOptions {
                wait_for_inspector: false,
            }),
        ).await;
    });

    cancel_flag.store(true, Ordering::Relaxed);
    let fired = watchdog.join().unwrap();
    eprintln!("[test] watchdog fired: {fired}");

    // Recovery — exact pattern from run_until_settled
    rt.v8_isolate().cancel_terminate_execution();
    match rt.execute_script("<recovery>", "void 0".to_string()) {
        Ok(_) => eprintln!("[test] recovery OK"),
        Err(_) => {
            rt.v8_isolate().cancel_terminate_execution();
            let _ = rt.execute_script("<recovery2>", "void 0".to_string());
            eprintln!("[test] recovery OK (2nd attempt)");
        }
    }

    let counter = eval_string(&mut rt, "globalThis.__infinite_counter");
    eprintln!("[test] infinite counter reached: {counter}");

    // THE KEY TEST
    let ok = test_microtask(&mut rt, &tokio_rt, "AFTER-TERMINATE");
    if !ok {
        println!("\n========================================");
        println!("ROOT CAUSE CONFIRMED!");
        println!("terminate_execution() during run_event_loop");
        println!("breaks V8's kAuto microtask auto-drain.");
        println!("All subsequent execute_script() calls");
        println!("no longer drain microtasks automatically.");
        println!("========================================\n");

        // Try to fix by explicit checkpoint
        rt.v8_isolate().perform_microtask_checkpoint();
        let ok2 = test_microtask(&mut rt, &tokio_rt, "after-explicit-checkpoint");
        if ok2 {
            println!("FIX: perform_microtask_checkpoint() after terminate restores drain!");
        } else {
            println!("perform_microtask_checkpoint also broken after terminate.");
            // Try force policy
            rt.v8_isolate().set_microtasks_policy(deno_core::v8::MicrotasksPolicy::Auto);
            let ok3 = test_microtask(&mut rt, &tokio_rt, "after-force-auto-policy");
            if ok3 {
                println!("FIX: re-setting MicrotasksPolicy::Auto restores drain!");
            }
        }
    }
}
