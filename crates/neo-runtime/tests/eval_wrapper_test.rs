//! Test that our eval_and_settle IIFE wrapper doesn't block microtask drain.

use deno_core::{JsRuntime as DenoJsRuntime, RuntimeOptions};

fn bare_runtime() -> DenoJsRuntime {
    DenoJsRuntime::new(RuntimeOptions::default())
}

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

/// Replicate EXACTLY what eval_and_settle does
fn eval_wrapped(rt: &mut DenoJsRuntime, code: &str) -> String {
    let wrapped = format!(
        "(function(){{var __r;try{{__r=(\n{}\n)}}catch(__e){{__r='Error: '+__e.message}};if(typeof globalThis.__neo_drainMicrotasks==='function')globalThis.__neo_drainMicrotasks();return __r}})()",
        code
    );
    let result = rt
        .execute_script("<eval-settle>", wrapped)
        .expect("execute_script failed");
    let scope = &mut rt.handle_scope();
    let local = deno_core::v8::Local::new(scope, result);
    local
        .to_string(scope)
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_else(|| "undefined".to_string())
}

/// Test: does the IIFE wrapper block microtask auto-drain?
#[test]
fn t_wrapper_microtask_drain() {
    let mut rt = bare_runtime();

    // Set up microtask via our IIFE wrapper
    let r1 = eval_wrapped(
        &mut rt,
        "(function(){globalThis.__x='B';Promise.resolve().then(function(){globalThis.__x='A'});return globalThis.__x})()"
    );
    println!("[WRAPPER] eval_wrapped returned: {r1}");

    // Read with bare eval (no wrapper)
    let r2 = eval_string(&mut rt, "globalThis.__x");
    println!("[WRAPPER] bare read after: {r2}");

    // Read with wrapper
    let r3 = eval_wrapped(&mut rt, "globalThis.__x");
    println!("[WRAPPER] wrapped read after: {r3}");

    assert_eq!(r2, "A", "Microtask should have drained between execute_script calls");
}

/// Test: does the IIFE wrapper + tokio block_on affect drain?
#[test]
fn t_wrapper_with_tokio() {
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let _guard = tokio_rt.enter();

    let mut rt = bare_runtime();

    let r1 = eval_wrapped(
        &mut rt,
        "(function(){globalThis.__y='B';Promise.resolve().then(function(){globalThis.__y='A'});return globalThis.__y})()"
    );
    println!("[TOKIO] eval_wrapped returned: {r1}");

    let r2 = eval_string(&mut rt, "globalThis.__y");
    println!("[TOKIO] bare read after: {r2}");

    assert_eq!(r2, "A", "Microtask should drain even with tokio entered");
}

/// Test: does loading bootstrap.js break the drain?
#[test]
fn t_wrapper_with_bootstrap() {
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let _guard = tokio_rt.enter();

    let mut rt = bare_runtime();

    // Load __neo_drainMicrotasks (the minimal version from bootstrap)
    rt.execute_script("<bootstrap-drain>", r#"
        globalThis.__neo_drainMicrotasks = function() {};
    "#.to_string()).unwrap();

    let r1 = eval_wrapped(
        &mut rt,
        "(function(){globalThis.__z='B';Promise.resolve().then(function(){globalThis.__z='A'});return globalThis.__z})()"
    );
    println!("[BOOTSTRAP] eval_wrapped returned: {r1}");

    let r2 = eval_string(&mut rt, "globalThis.__z");
    println!("[BOOTSTRAP] bare read after: {r2}");

    assert_eq!(r2, "A", "Microtask should drain with bootstrap loaded");
}

/// Test: with our actual DenoRuntime (neo_runtime extension)
#[test]
fn t_wrapper_with_neo_runtime() {
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let _guard = tokio_rt.enter();

    let mut rt = DenoJsRuntime::new(RuntimeOptions {
        extensions: vec![neo_runtime::v8::neo_runtime_ext::init_ops()],
        ..Default::default()
    });

    // Minimal bootstrap
    rt.execute_script("<bootstrap>", r#"
        globalThis.__neo_ops = Deno.core.ops;
        globalThis.__neo_drainMicrotasks = function() {};
    "#.to_string()).unwrap();

    let r1 = eval_wrapped(
        &mut rt,
        "(function(){globalThis.__w='B';Promise.resolve().then(function(){globalThis.__w='A'});return globalThis.__w})()"
    );
    println!("[NEO] eval_wrapped returned: {r1}");

    let r2 = eval_string(&mut rt, "globalThis.__w");
    println!("[NEO] bare read after: {r2}");

    assert_eq!(r2, "A", "Microtask should drain with neo_runtime extension");
}

/// Test: with FULL bootstrap.js and happy-dom
#[test]
fn t_wrapper_with_full_bootstrap() {
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let _guard = tokio_rt.enter();

    let mut rt = DenoJsRuntime::new(RuntimeOptions {
        extensions: vec![neo_runtime::v8::neo_runtime_ext::init_ops()],
        ..Default::default()
    });

    // Load happy-dom bundle if exists
    let happydom_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap().parent().unwrap()
        .join("js/happydom.bundle.js");
    if happydom_path.exists() {
        let happydom = std::fs::read_to_string(&happydom_path).unwrap();
        let _ = rt.execute_script("<happydom>", happydom);
    }

    // Load bootstrap
    let bootstrap_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap().parent().unwrap()
        .join("js/bootstrap.js");
    if bootstrap_path.exists() {
        let bootstrap = std::fs::read_to_string(&bootstrap_path).unwrap();
        let _ = rt.execute_script("<bootstrap>", bootstrap);
    }

    let r1 = eval_wrapped(
        &mut rt,
        "(function(){globalThis.__v='B';Promise.resolve().then(function(){globalThis.__v='A'});return globalThis.__v})()"
    );
    println!("[FULL] eval_wrapped returned: {r1}");

    let r2 = eval_string(&mut rt, "globalThis.__v");
    println!("[FULL] bare read after: {r2}");

    if r2 == "A" {
        println!("[FULL] PASS — microtasks drain with full bootstrap");
    } else {
        println!("[FULL] FAIL — microtasks DO NOT drain with full bootstrap");
        println!("[FULL] This means bootstrap.js breaks microtask drain!");
    }
    // Don't assert — we want to see the result even if it fails
}
