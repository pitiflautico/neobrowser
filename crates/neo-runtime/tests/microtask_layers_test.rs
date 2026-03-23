//! Layer-by-layer microtask drain test.
//! Adds one component at a time to find which layer breaks microtask drain.
//!
//! Run: cargo test -p neo-runtime --test microtask_layers_test -- --nocapture

use deno_core::{JsRuntime as DenoJsRuntime, PollEventLoopOptions, RuntimeOptions};
use std::time::Duration;

fn eval_wrapped(rt: &mut DenoJsRuntime, code: &str) -> String {
    let wrapped = format!(
        "(function(){{var __r;try{{__r=(\n{}\n)}}catch(__e){{__r='Error: '+__e.message}};if(typeof globalThis.__neo_drainMicrotasks==='function')globalThis.__neo_drainMicrotasks();return __r}})()",
        code
    );
    let result = rt
        .execute_script("<eval>", wrapped)
        .expect("execute_script failed");
    let scope = &mut rt.handle_scope();
    let local = deno_core::v8::Local::new(scope, result);
    local
        .to_string(scope)
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_else(|| "undefined".to_string())
}

fn read_global(rt: &mut DenoJsRuntime, name: &str) -> String {
    let result = rt
        .execute_script("<read>", format!("String(globalThis.{})", name))
        .expect("execute_script failed");
    let scope = &mut rt.handle_scope();
    let local = deno_core::v8::Local::new(scope, result);
    local
        .to_string(scope)
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_else(|| "undefined".to_string())
}

fn test_microtask(rt: &mut DenoJsRuntime, label: &str, var: &str) -> bool {
    let code = format!(
        "(function(){{globalThis.{v}='B';Promise.resolve().then(function(){{globalThis.{v}='A'}});return globalThis.{v}}})()",
        v = var
    );
    let r1 = eval_wrapped(rt, &code);
    let r2 = read_global(rt, var);
    let ok = r2 == "A";
    println!("[{label}] eval={r1} read={r2} -> {}", if ok { "PASS" } else { "FAIL" });
    ok
}

/// Layer 0: bare V8
#[test]
fn layer_0_bare() {
    let mut rt = DenoJsRuntime::new(RuntimeOptions::default());
    assert!(test_microtask(&mut rt, "L0-bare", "__l0"));
}

/// Layer 1: with tokio entered
#[test]
fn layer_1_tokio() {
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all().build().unwrap();
    let _guard = tokio_rt.enter();
    let mut rt = DenoJsRuntime::new(RuntimeOptions::default());
    assert!(test_microtask(&mut rt, "L1-tokio", "__l1"));
}

/// Layer 2: with neo_runtime extension
#[test]
fn layer_2_extension() {
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all().build().unwrap();
    let _guard = tokio_rt.enter();
    let mut rt = DenoJsRuntime::new(RuntimeOptions {
        extensions: vec![neo_runtime::v8::neo_runtime_ext::init_ops()],
        ..Default::default()
    });
    assert!(test_microtask(&mut rt, "L2-ext", "__l2"));
}

/// Layer 3: with __neo_drainMicrotasks defined
#[test]
fn layer_3_drain_fn() {
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all().build().unwrap();
    let _guard = tokio_rt.enter();
    let mut rt = DenoJsRuntime::new(RuntimeOptions {
        extensions: vec![neo_runtime::v8::neo_runtime_ext::init_ops()],
        ..Default::default()
    });
    rt.execute_script("<setup>", "globalThis.__neo_drainMicrotasks = function() {};".to_string()).unwrap();
    assert!(test_microtask(&mut rt, "L3-drain", "__l3"));
}

/// Layer 4: with happy-dom loaded
#[test]
fn layer_4_happydom() {
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all().build().unwrap();
    let _guard = tokio_rt.enter();
    let mut rt = DenoJsRuntime::new(RuntimeOptions {
        extensions: vec![neo_runtime::v8::neo_runtime_ext::init_ops()],
        ..Default::default()
    });

    let js_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap().parent().unwrap().join("js");

    let happydom = js_dir.join("happydom.bundle.js");
    if happydom.exists() {
        let code = std::fs::read_to_string(&happydom).unwrap();
        let _ = rt.execute_script("<happydom>", code);
    }

    assert!(test_microtask(&mut rt, "L4-happydom", "__l4"));
}

/// Layer 5: with bootstrap.js loaded
#[test]
fn layer_5_bootstrap() {
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all().build().unwrap();
    let _guard = tokio_rt.enter();
    let mut rt = DenoJsRuntime::new(RuntimeOptions {
        extensions: vec![neo_runtime::v8::neo_runtime_ext::init_ops()],
        ..Default::default()
    });

    let js_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap().parent().unwrap().join("js");

    // Load in order
    let happydom = js_dir.join("happydom.bundle.js");
    if happydom.exists() {
        let code = std::fs::read_to_string(&happydom).unwrap();
        let _ = rt.execute_script("<happydom>", code);
    }

    let bootstrap = js_dir.join("bootstrap.js");
    if bootstrap.exists() {
        let code = std::fs::read_to_string(&bootstrap).unwrap();
        let _ = rt.execute_script("<bootstrap>", code);
    }

    assert!(test_microtask(&mut rt, "L5-bootstrap", "__l5"));
}

/// Layer 6: with browser_shim.js loaded
#[test]
fn layer_6_shim() {
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all().build().unwrap();
    let _guard = tokio_rt.enter();
    let mut rt = DenoJsRuntime::new(RuntimeOptions {
        extensions: vec![neo_runtime::v8::neo_runtime_ext::init_ops()],
        ..Default::default()
    });

    let js_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap().parent().unwrap().join("js");

    for file in &["happydom.bundle.js", "bootstrap.js", "browser_shim.js"] {
        let path = js_dir.join(file);
        if path.exists() {
            let code = std::fs::read_to_string(&path).unwrap();
            let name: &'static str = Box::leak(format!("<{}>", file).into_boxed_str());
            let _ = rt.execute_script(name, code);
        }
    }

    assert!(test_microtask(&mut rt, "L6-shim", "__l6"));
}

/// Layer 7: with sentinel.js loaded
#[test]
fn layer_7_sentinel() {
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all().build().unwrap();
    let _guard = tokio_rt.enter();
    let mut rt = DenoJsRuntime::new(RuntimeOptions {
        extensions: vec![neo_runtime::v8::neo_runtime_ext::init_ops()],
        ..Default::default()
    });

    let js_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap().parent().unwrap().join("js");

    for file in &["happydom.bundle.js", "bootstrap.js", "browser_shim.js", "sentinel.js"] {
        let path = js_dir.join(file);
        if path.exists() {
            let code = std::fs::read_to_string(&path).unwrap();
            let name: &'static str = Box::leak(format!("<{}>", file).into_boxed_str());
            let _ = rt.execute_script(name, code);
        }
    }

    assert!(test_microtask(&mut rt, "L7-sentinel", "__l7"));
}

/// Layer 8: with turbo-stream bundle loaded
#[test]
fn layer_8_turbostream() {
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all().build().unwrap();
    let _guard = tokio_rt.enter();
    let mut rt = DenoJsRuntime::new(RuntimeOptions {
        extensions: vec![neo_runtime::v8::neo_runtime_ext::init_ops()],
        ..Default::default()
    });

    let js_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap().parent().unwrap().join("js");

    for file in &["happydom.bundle.js", "bootstrap.js", "browser_shim.js", "sentinel.js", "turbo-stream.bundle.js"] {
        let path = js_dir.join(file);
        if path.exists() {
            let code = std::fs::read_to_string(&path).unwrap();
            let name: &'static str = Box::leak(format!("<{}>", file).into_boxed_str());
            let _ = rt.execute_script(name, code);
        }
    }

    assert!(test_microtask(&mut rt, "L8-turbo", "__l8"));
}

/// Layer 9: with HTML set (simulated page load)
#[test]
fn layer_9_html_set() {
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all().build().unwrap();
    let _guard = tokio_rt.enter();
    let mut rt = DenoJsRuntime::new(RuntimeOptions {
        extensions: vec![neo_runtime::v8::neo_runtime_ext::init_ops()],
        ..Default::default()
    });

    let js_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap().parent().unwrap().join("js");

    for file in &["happydom.bundle.js", "bootstrap.js", "browser_shim.js", "sentinel.js", "turbo-stream.bundle.js"] {
        let path = js_dir.join(file);
        if path.exists() {
            let code = std::fs::read_to_string(&path).unwrap();
            let name: &'static str = Box::leak(format!("<{}>", file).into_boxed_str());
            let _ = rt.execute_script(name, code);
        }
    }

    // Set HTML like a page load
    let _ = rt.execute_script("<html>", r#"
        if (typeof globalThis.__neorender_html === 'function') {
            globalThis.__neorender_html('<html><body><h1>Test</h1></body></html>', 'https://example.com/');
        }
    "#.to_string());

    assert!(test_microtask(&mut rt, "L9-html", "__l9"));
}

/// Layer 10: after run_event_loop (simulates settle after page load)
#[test]
fn layer_10_after_settle() {
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all().build().unwrap();

    let mut rt = DenoJsRuntime::new(RuntimeOptions {
        extensions: vec![neo_runtime::v8::neo_runtime_ext::init_ops()],
        ..Default::default()
    });

    let js_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap().parent().unwrap().join("js");

    for file in &["happydom.bundle.js", "bootstrap.js", "browser_shim.js", "sentinel.js", "turbo-stream.bundle.js"] {
        let path = js_dir.join(file);
        if path.exists() {
            let code = std::fs::read_to_string(&path).unwrap();
            let name: &'static str = Box::leak(format!("<{}>", file).into_boxed_str());
            let _ = rt.execute_script(name, code);
        }
    }

    // Simulate settle (run event loop briefly)
    tokio_rt.block_on(async {
        let _ = tokio::time::timeout(
            Duration::from_millis(100),
            rt.run_event_loop(PollEventLoopOptions::default()),
        ).await;
    });

    // NOW test microtask after settle
    let ok = test_microtask(&mut rt, "L10-settle", "__l10");
    if !ok {
        println!("[L10] CRITICAL: microtask drain broken AFTER run_event_loop!");
        println!("[L10] This means run_event_loop leaves the runtime in a state");
        println!("[L10] where subsequent execute_script calls don't auto-drain.");
    }
    assert!(ok);
}
