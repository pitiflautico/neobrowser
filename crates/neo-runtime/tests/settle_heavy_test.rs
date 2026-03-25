//! Test with REAL pending ops that keep event loop alive long enough
//! for the watchdog to fire terminate_execution.

use deno_core::{JsRuntime as DenoJsRuntime, PollEventLoopOptions, RuntimeOptions};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn make_runtime() -> DenoJsRuntime {
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

/// Simulate run_until_settled with watchdog — EXACT code from v8_runtime_impl.rs
fn run_until_settled_sim(
    tokio_rt: &tokio::runtime::Runtime,
    rt: &mut DenoJsRuntime,
    timeout_ms: u64,
) {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);

    let isolate_handle = rt.v8_isolate().thread_safe_handle();
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let cancel_clone = cancel_flag.clone();
    let watchdog_deadline = deadline;

    let watchdog = std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(50));
            if cancel_clone.load(Ordering::Relaxed) { return; }
            if Instant::now() >= watchdog_deadline {
                eprintln!("[watchdog] TERMINATING execution");
                isolate_handle.terminate_execution();
                return;
            }
        }
    });

    let result = tokio_rt.block_on(async {
        loop {
            if Instant::now() >= deadline {
                return Ok::<_, deno_core::error::CoreError>(());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            match tokio::time::timeout(
                Duration::from_millis(50).min(remaining),
                rt.run_event_loop(PollEventLoopOptions {
                    wait_for_inspector: false,
                }),
            ).await {
                Ok(Ok(())) => {
                    // Event loop idle — check if settled
                    let elapsed = deadline.duration_since(Instant::now() + Duration::from_millis(timeout_ms) - Duration::from_millis(timeout_ms));
                    return Ok(());
                }
                Ok(Err(e)) => {
                    eprintln!("[settle] event loop error: {e}");
                    return Ok(());
                }
                Err(_) => {
                    // Timeout — event loop had work, continue
                    continue;
                }
            }
        }
    });

    cancel_flag.store(true, Ordering::Relaxed);
    let _ = watchdog.join();

    rt.v8_isolate().cancel_terminate_execution();
    match rt.execute_script("<recovery>", "void 0".to_string()) {
        Ok(_) => {}
        Err(_) => {
            rt.v8_isolate().cancel_terminate_execution();
            let _ = rt.execute_script("<recovery2>", "void 0".to_string());
        }
    }
}

/// Test: create REAL pending ops via op_microtask_tick (multiple)
/// that keep the event loop alive, then settle with watchdog
#[test]
fn heavy_t1_many_pending_ops() {
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all().build().unwrap();

    let mut rt = make_runtime();
    assert!(test_microtask(&mut rt, &tokio_rt, "heavy-before"));

    // Create many pending promises (JS-only, no internal ops)
    rt.execute_script("<ops>", r#"
        for (var i = 0; i < 50; i++) {
            Promise.resolve(i).then(function(v) { globalThis['__op_' + v] = true; });
        }
    "#.to_string()).unwrap();

    // Settle with watchdog (500ms timeout)
    run_until_settled_sim(&tokio_rt, &mut rt, 500);

    let ok = test_microtask(&mut rt, &tokio_rt, "heavy-after-settle-500ms");
    if !ok {
        println!(">>> FOUND: many pending ops + settle breaks microtask drain!");
    }
}

/// Test: create pending ops + long-running JS (triggers watchdog terminate)
#[test]
fn heavy_t2_long_running_js() {
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all().build().unwrap();

    let mut rt = make_runtime();
    assert!(test_microtask(&mut rt, &tokio_rt, "heavy-t2-before"));

    // Create long-running JS that will be terminated by watchdog
    rt.execute_script("<long>", r#"
        // This creates a promise chain that keeps creating more promises
        var count = 0;
        function makeWork() {
            count++;
            if (count < 10000) {
                Promise.resolve().then(makeWork);
            }
        }
        makeWork();
    "#.to_string()).unwrap();

    // Short settle (200ms) — watchdog will terminate if JS is still running
    run_until_settled_sim(&tokio_rt, &mut rt, 200);

    let ok = test_microtask(&mut rt, &tokio_rt, "heavy-t2-after-long-js");
    if !ok {
        println!(">>> FOUND: long-running JS + watchdog terminate breaks microtask drain!");
    }
}

/// Test: use the ACTUAL DenoRuntime (full NeoRender stack) and call run_until_settled
#[test]
fn heavy_t3_actual_deno_runtime() {
    let config = neo_runtime::RuntimeConfig::default();
    let mut deno_rt = neo_runtime::v8::DenoRuntime::new(&config).unwrap();

    // Load bootstrap
    let js_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap().parent().unwrap().join("js");

    let bootstrap = js_dir.join("bootstrap.js");
    if bootstrap.exists() {
        let code = std::fs::read_to_string(&bootstrap).unwrap();
        let _ = deno_rt.runtime.execute_script("<bootstrap>", code);
    }

    // Microtask test BEFORE settle
    {
        deno_rt.runtime.execute_script("<set>",
            "globalThis.__pre='B';Promise.resolve().then(function(){globalThis.__pre='A'})".to_string()
        ).unwrap();
        let r = {
            let result = deno_rt.runtime.execute_script("<read>", "String(globalThis.__pre)".to_string()).unwrap();
            let context = deno_rt.runtime.main_context();
            deno_core::v8::scope!(scope, deno_rt.runtime.v8_isolate());
            let context = deno_core::v8::Local::new(scope, context);
            let scope = &mut deno_core::v8::ContextScope::new(scope, context);
            let local = deno_core::v8::Local::new(scope, result);
            local.to_string(scope).map(|s| s.to_rust_string_lossy(scope)).unwrap_or_default()
        };
        println!("[actual-before-settle            ] {r} -> {}", if r == "A" { "PASS" } else { "**FAIL**" });
    }

    // Call actual run_until_settled via trait
    use neo_runtime::JsRuntime;
    let _ = deno_rt.run_until_settled(2000);

    // Microtask test AFTER settle
    {
        deno_rt.runtime.execute_script("<set2>",
            "globalThis.__post='B';Promise.resolve().then(function(){globalThis.__post='A'})".to_string()
        ).unwrap();
        let r = {
            let result = deno_rt.runtime.execute_script("<read2>", "String(globalThis.__post)".to_string()).unwrap();
            let context = deno_rt.runtime.main_context();
            deno_core::v8::scope!(scope, deno_rt.runtime.v8_isolate());
            let context = deno_core::v8::Local::new(scope, context);
            let scope = &mut deno_core::v8::ContextScope::new(scope, context);
            let local = deno_core::v8::Local::new(scope, result);
            local.to_string(scope).map(|s| s.to_rust_string_lossy(scope)).unwrap_or_default()
        };
        let ok = r == "A";
        println!("[actual-after-settle              ] {r} -> {}", if ok { "PASS" } else { "**FAIL**" });
        if !ok {
            println!(">>> ROOT CAUSE CONFIRMED!");
            println!(">>> DenoRuntime::run_until_settled() breaks microtask auto-drain.");
            println!(">>> This is the exact function called during ChatGPT page load.");
        }
    }
}
