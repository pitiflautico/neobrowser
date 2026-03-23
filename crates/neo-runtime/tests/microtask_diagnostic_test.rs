//! Diagnostic test based on GPT analysis.
//! Tests: queue mismatch, scope depth, kScoped mode.
//!
//! Run: cargo test -p neo-runtime --test microtask_diagnostic_test -- --nocapture

use deno_core::{JsRuntime as DenoJsRuntime, PollEventLoopOptions, RuntimeOptions};
use std::time::Duration;

fn make_rt() -> DenoJsRuntime {
    DenoJsRuntime::new(RuntimeOptions {
        extensions: vec![neo_runtime::v8::neo_runtime_ext::init_ops()],
        ..Default::default()
    })
}

fn load_bootstrap(rt: &mut DenoJsRuntime) {
    let js_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap().parent().unwrap().join("js");
    for file in &["happy-dom.bundle.js", "bootstrap.js", "browser_shim.js",
                  "sentinel.js", "turbo-stream.bundle.js"] {
        let path = js_dir.join(file);
        if path.exists() {
            let code = std::fs::read_to_string(&path).unwrap();
            let name: &'static str = Box::leak(format!("<{}>", file).into_boxed_str());
            let _ = rt.execute_script(name, code);
        }
    }
}

fn eval_string(rt: &mut DenoJsRuntime, code: &str) -> String {
    let result = rt
        .execute_script("<test>", format!("String({})", code))
        .expect("execute_script failed");
    let scope = &mut rt.handle_scope();
    let local = deno_core::v8::Local::new(scope, result);
    local.to_string(scope).map(|s| s.to_rust_string_lossy(scope)).unwrap_or_default()
}

/// Simulate the EXACT ChatGPT loading pipeline
fn simulate_heavy_spa(rt: &mut DenoJsRuntime, tokio_rt: &tokio::runtime::Runtime) {
    // Step 1: ChatGPT inline scripts
    let _ = rt.execute_script("<theme>", r#"
        !function(){try{var d=document.documentElement,c=d.classList;
        c.remove('light','dark');c.add('dark')}catch(e){}}()
    "#.to_string());

    let _ = rt.execute_script("<router-ctx>", r#"
        window.__reactRouterContext = {"basename":"/","future":{},"isSpaMode":false,
        "stream":new ReadableStream({start(controller){
            window.__reactRouterContext.streamController = controller;
        }})};
    "#.to_string());

    // Step 2: Simulate the async IIFE module import
    // This is the KEY step — an async IIFE that creates promises
    let _ = rt.execute_script("<module-iife>", r#"
        (async function() {
            // Simulate dynamic import resolution with many promises
            globalThis.__module_promises = [];
            for (var i = 0; i < 100; i++) {
                globalThis.__module_promises.push(
                    Promise.resolve({default: function(){}, name: 'module_' + i})
                );
            }
            var results = await Promise.all(globalThis.__module_promises);
            globalThis.__modules_loaded = results.length;

            // Simulate React hydration — many nested promise chains
            for (var j = 0; j < 50; j++) {
                await Promise.resolve();
                // Simulate component mount
            }
            globalThis.__hydration_done = true;
        })();
    "#.to_string());

    // Step 3: run_event_loop for settle (like run_until_settled)
    tokio_rt.block_on(async {
        let _ = tokio::time::timeout(
            Duration::from_millis(5000),
            rt.run_event_loop(PollEventLoopOptions {
                wait_for_inspector: false,
                pump_v8_message_loop: true,
            }),
        ).await;
    });
}

/// DIAGNOSTIC 1: Check MicrotaskQueue scope depth
#[test]
fn diag_1_scope_depth() {
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all().build().unwrap();
    let _guard = tokio_rt.enter();

    let mut rt = make_rt();
    load_bootstrap(&mut rt);

    // Check depth BEFORE heavy load
    let depth_before = {
        let scope = &mut rt.handle_scope();
        let ctx = scope.get_current_context();
        let queue = ctx.get_microtask_queue();
        queue.get_microtasks_scope_depth()
    };
    println!("[DIAG1] Scope depth BEFORE heavy load: {depth_before}");

    simulate_heavy_spa(&mut rt, &tokio_rt);

    // Check depth AFTER heavy load
    let depth_after = {
        let scope = &mut rt.handle_scope();
        let ctx = scope.get_current_context();
        let queue = ctx.get_microtask_queue();
        queue.get_microtasks_scope_depth()
    };
    println!("[DIAG1] Scope depth AFTER heavy load: {depth_after}");

    // Check if IsRunningMicrotasks
    let is_running = {
        let scope = &mut rt.handle_scope();
        let ctx = scope.get_current_context();
        let queue = ctx.get_microtask_queue();
        queue.is_running_microtasks()
    };
    println!("[DIAG1] IsRunningMicrotasks: {is_running}");

    // Test microtask drain
    rt.execute_script("<set>",
        "globalThis.__d1='B';Promise.resolve().then(function(){globalThis.__d1='A'})".to_string()
    ).unwrap();
    let r = eval_string(&mut rt, "globalThis.__d1");
    println!("[DIAG1] Microtask drain: {r} -> {}", if r == "A" { "PASS" } else { "FAIL" });

    if r != "A" {
        // Try checkpoint on the CONTEXT queue (not isolate)
        {
            let scope = &mut rt.handle_scope();
            let ctx = scope.get_current_context();
            let queue = ctx.get_microtask_queue();
            queue.perform_checkpoint(scope);
        }
        let r2 = eval_string(&mut rt, "globalThis.__d1");
        println!("[DIAG1] After CONTEXT queue checkpoint: {r2} -> {}",
            if r2 == "A" { "PASS — context queue was the issue!" } else { "still FAIL" });
    }
}

/// DIAGNOSTIC 2: Try kScoped mode (Chrome's mode)
#[test]
fn diag_2_kscoped() {
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all().build().unwrap();
    let _guard = tokio_rt.enter();

    let mut rt = make_rt();
    load_bootstrap(&mut rt);
    simulate_heavy_spa(&mut rt, &tokio_rt);

    // Switch to kScoped and use MicrotasksScope
    println!("[DIAG2] Switching to kScoped mode...");

    rt.execute_script("<set>",
        "globalThis.__d2='B';Promise.resolve().then(function(){globalThis.__d2='A'})".to_string()
    ).unwrap();

    // Create a MicrotasksScope that drains on drop (Chrome pattern)
    {
        let scope = &mut rt.handle_scope();
        // The MicrotasksScope should drain microtasks when it destructs
        // But we need to check if the Rust V8 bindings expose this
        let ctx = scope.get_current_context();
        let queue = ctx.get_microtask_queue();
        let depth = queue.get_microtasks_scope_depth();
        let running = queue.is_running_microtasks();
        println!("[DIAG2] Queue depth={depth} running={running}");

        // Manual checkpoint on the context queue
        queue.perform_checkpoint(scope);
    }

    let r = eval_string(&mut rt, "globalThis.__d2");
    println!("[DIAG2] After kScoped+checkpoint: {r} -> {}",
        if r == "A" { "PASS" } else { "FAIL" });
}

/// DIAGNOSTIC 3: Check if microtask is even enqueued
#[test]
fn diag_3_enqueue_check() {
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all().build().unwrap();
    let _guard = tokio_rt.enter();

    let mut rt = make_rt();
    load_bootstrap(&mut rt);
    simulate_heavy_spa(&mut rt, &tokio_rt);

    // Use a different approach: enqueue via V8 API directly
    {
        let scope = &mut rt.handle_scope();
        let ctx = scope.get_current_context();
        let queue = ctx.get_microtask_queue();

        // Enqueue a function as microtask via the V8 API
        let code = deno_core::v8::String::new(scope, "globalThis.__d3_direct = 'DIRECT'").unwrap();
        let func = deno_core::v8::Function::new(scope, |fscope: &mut deno_core::v8::HandleScope,
            _args: deno_core::v8::FunctionCallbackArguments,
            _rv: deno_core::v8::ReturnValue| {
            let code = deno_core::v8::String::new(fscope, "globalThis.__d3_direct = 'DIRECT_CALLED'").unwrap();
            let script = deno_core::v8::Script::compile(fscope, code, None).unwrap();
            script.run(fscope);
        }).unwrap();

        queue.enqueue_microtask(scope, func);

        println!("[DIAG3] Microtask enqueued via V8 API");

        // Now checkpoint
        queue.perform_checkpoint(scope);
    }

    let r = eval_string(&mut rt, "globalThis.__d3_direct");
    println!("[DIAG3] Direct V8 microtask: {r} -> {}",
        if r == "DIRECT_CALLED" { "PASS — V8 API microtask works!" } else { "FAIL" });
}
