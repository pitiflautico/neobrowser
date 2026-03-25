//! Test: does loading the actual ChatGPT manifest break microtask drain?
//!
//! Run: cargo test -p neo-runtime --test chatgpt_manifest_test -- --nocapture

use deno_core::{JsRuntime as DenoJsRuntime, PollEventLoopOptions, RuntimeOptions};
use std::time::Duration;

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
    let var = format!("__mt_{}", label.replace(['-', ' ', '.'], "_"));
    rt.execute_script("<set>", format!(
        "globalThis.{v}='B';Promise.resolve().then(function(){{globalThis.{v}='A'}})", v = var
    )).unwrap();
    // deno_core 0.393 doesn't auto-drain microtasks after execute_script — pump event loop
    let _ = tokio_rt.block_on(rt.run_event_loop(PollEventLoopOptions::default()));
    let r = eval_string(rt, &format!("globalThis.{}", var));
    let ok = r == "A";
    println!("[{label:40}] {r} -> {}", if ok { "PASS" } else { "**FAIL**" });
    ok
}

#[test]
fn manifest_test() {
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all().build().unwrap();
    let _guard = tokio_rt.enter();

    let mut rt = DenoJsRuntime::new(RuntimeOptions {
        extensions: vec![neo_runtime::v8::neo_runtime_ext::init()],
        ..Default::default()
    });

    // Load our full bootstrap stack
    let js_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap().parent().unwrap().join("js");

    for file in &["happy-dom.bundle.js", "bootstrap.js", "browser_shim.js"] {
        let path = js_dir.join(file);
        if path.exists() {
            let code = std::fs::read_to_string(&path).unwrap();
            let name: &'static str = Box::leak(format!("<{}>", file).into_boxed_str());
            let _ = rt.execute_script(name, code);
        }
    }

    assert!(test_microtask(&mut rt, &tokio_rt, "after-bootstrap"));

    // Load the ChatGPT manifest (1.8MB)
    let manifest_path = std::path::Path::new(&std::env::var("HOME").unwrap())
        .join(".neorender/cache/modules/c144191edff10459.js");

    if !manifest_path.exists() {
        println!("SKIP: ChatGPT manifest not cached. Run neorender see chatgpt.com first.");
        return;
    }

    let manifest = std::fs::read_to_string(&manifest_path).unwrap();
    println!("Loaded manifest: {} bytes", manifest.len());

    // Execute manifest as regular script (not module)
    match rt.execute_script("<chatgpt-manifest>", manifest) {
        Ok(_) => println!("Manifest executed OK"),
        Err(e) => {
            println!("Manifest error (expected — it imports things): {}", &e.to_string()[..200.min(e.to_string().len())]);
        }
    }

    let ok = test_microtask(&mut rt, &tokio_rt, "after-manifest-exec");
    if !ok {
        println!(">>> FOUND: ChatGPT manifest execution breaks microtask drain!");
    }

    // Settle
    tokio_rt.block_on(async {
        let _ = tokio::time::timeout(
            Duration::from_millis(2000),
            rt.run_event_loop(PollEventLoopOptions::default()),
        ).await;
    });

    let ok2 = test_microtask(&mut rt, &tokio_rt, "after-manifest-settle");
    if !ok2 {
        println!(">>> FOUND: ChatGPT manifest + settle breaks microtask drain!");
    }
}
