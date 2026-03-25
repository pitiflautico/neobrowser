//! Test if loading ES modules breaks microtask drain.
//! The hypothesis: ChatGPT loads its code as ES modules via import().
//! The module evaluation path in deno_core may leave V8 in a state
//! where microtask auto-drain stops working.
//!
//! Run: cargo test -p neo-runtime --test module_microtask_test -- --nocapture

use deno_core::{
    JsRuntime as DenoJsRuntime, ModuleLoadOptions, ModuleLoadResponse, ModuleLoader,
    ModuleLoadReferrer, ModuleSource, ModuleSourceCode, ModuleSpecifier, ModuleType,
    PollEventLoopOptions, ResolutionKind, RuntimeOptions,
};
use std::time::Duration;

/// Simple module loader that serves from a HashMap
struct TestModuleLoader {
    modules: std::collections::HashMap<String, String>,
}

impl ModuleLoader for TestModuleLoader {
    fn resolve(
        &self,
        specifier: &str,
        referrer: &str,
        _kind: ResolutionKind,
    ) -> Result<ModuleSpecifier, deno_error::JsErrorBox> {
        if specifier.starts_with("http") || specifier.starts_with("file") {
            ModuleSpecifier::parse(specifier).map_err(deno_error::JsErrorBox::from_err)
        } else {
            let base = ModuleSpecifier::parse(referrer)
                .unwrap_or_else(|_| ModuleSpecifier::parse("file:///test/").unwrap());
            base.join(specifier).map_err(deno_error::JsErrorBox::from_err)
        }
    }

    fn load(
        &self,
        module_specifier: &ModuleSpecifier,
        _maybe_referrer: Option<&ModuleLoadReferrer>,
        _options: ModuleLoadOptions,
    ) -> ModuleLoadResponse {
        let url = module_specifier.to_string();
        if let Some(source) = self.modules.get(&url) {
            ModuleLoadResponse::Sync(Ok(ModuleSource::new(
                ModuleType::JavaScript,
                ModuleSourceCode::String(source.clone().into()),
                module_specifier,
                None,
            )))
        } else {
            ModuleLoadResponse::Sync(Err(deno_error::JsErrorBox::generic(
                format!("Module not found: {url}"),
            )))
        }
    }
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
    let var = format!("__mt_{}", label.replace("-", "_").replace(" ", "_"));
    let code = format!(
        "globalThis.{v}='B';Promise.resolve().then(function(){{globalThis.{v}='A'}})",
        v = var
    );
    rt.execute_script("<set>", code).unwrap();
    // deno_core 0.393: explicit pump needed for microtask drain
    let _ = tokio_rt.block_on(rt.run_event_loop(PollEventLoopOptions::default()));
    let r = eval_string(rt, &format!("globalThis.{}", var));
    let ok = r == "A";
    println!("[{label:30}] read={r} -> {}", if ok { "PASS" } else { "**FAIL**" });
    ok
}

/// Test 1: baseline without modules
#[test]
fn module_t1_baseline() {
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all().build().unwrap();
    let _guard = tokio_rt.enter();

    let mut modules = std::collections::HashMap::new();
    modules.insert("file:///test/simple.js".to_string(), "globalThis.__module_loaded = true;".to_string());

    let mut rt = DenoJsRuntime::new(RuntimeOptions {
        module_loader: Some(std::rc::Rc::new(TestModuleLoader { modules })),
        ..Default::default()
    });

    assert!(test_microtask(&mut rt, &tokio_rt, "before-module"));
}

/// Test 2: after loading a simple ES module
#[test]
fn module_t2_after_simple_module() {
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all().build().unwrap();

    let mut modules = std::collections::HashMap::new();
    modules.insert(
        "file:///test/simple.js".to_string(),
        "globalThis.__module_loaded = true; export default 42;".to_string(),
    );

    let mut rt = DenoJsRuntime::new(RuntimeOptions {
        module_loader: Some(std::rc::Rc::new(TestModuleLoader { modules })),
        ..Default::default()
    });

    // Load and evaluate module
    let mod_id = tokio_rt.block_on(async {
        rt.load_main_es_module(&ModuleSpecifier::parse("file:///test/simple.js").unwrap()).await
    }).expect("load failed");

    let _ = tokio_rt.block_on(async {
        rt.mod_evaluate(mod_id)
    });

    // Settle
    tokio_rt.block_on(async {
        let _ = tokio::time::timeout(
            Duration::from_millis(100),
            rt.run_event_loop(PollEventLoopOptions::default()),
        ).await;
    });

    let loaded = eval_string(&mut rt, "globalThis.__module_loaded");
    println!("[module-loaded] {loaded}");

    let ok = test_microtask(&mut rt, &tokio_rt, "after-simple-module");
    if !ok {
        println!(">>> FOUND: simple ES module loading breaks microtask drain!");
    }
}

/// Test 3: after loading a module with Promise usage
#[test]
fn module_t3_module_with_promises() {
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all().build().unwrap();

    let mut modules = std::collections::HashMap::new();
    modules.insert(
        "file:///test/promises.js".to_string(),
        r#"
        globalThis.__promise_module = 'loaded';
        const p = Promise.resolve(42);
        p.then(v => { globalThis.__promise_result = v; });
        export default p;
        "#.to_string(),
    );

    let mut rt = DenoJsRuntime::new(RuntimeOptions {
        module_loader: Some(std::rc::Rc::new(TestModuleLoader { modules })),
        ..Default::default()
    });

    let mod_id = tokio_rt.block_on(async {
        rt.load_main_es_module(&ModuleSpecifier::parse("file:///test/promises.js").unwrap()).await
    }).expect("load failed");

    let receiver = rt.mod_evaluate(mod_id);

    tokio_rt.block_on(async {
        let _ = tokio::time::timeout(
            Duration::from_millis(500),
            rt.run_event_loop(PollEventLoopOptions::default()),
        ).await;
    });

    // Wait for mod_evaluate to complete
    let _ = tokio_rt.block_on(receiver);

    let ok = test_microtask(&mut rt, &tokio_rt, "after-promise-module");
    if !ok {
        println!(">>> FOUND: module with Promises breaks microtask drain!");
    }
}

/// Test 4: after loading module + execute_script + run_event_loop cycle
/// This simulates the exact NeoRender page load pipeline
#[test]
fn module_t4_full_pipeline() {
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all().build().unwrap();

    let mut modules = std::collections::HashMap::new();
    modules.insert(
        "file:///test/app.js".to_string(),
        r#"
        import { helper } from './helper.js';
        globalThis.__app_loaded = true;
        globalThis.__helper_result = helper();
        // Simulate React-like async work
        Promise.resolve().then(() => {
            globalThis.__react_mounted = true;
        });
        export default { loaded: true };
        "#.to_string(),
    );
    modules.insert(
        "file:///test/helper.js".to_string(),
        r#"
        export function helper() { return 'hello from helper'; }
        "#.to_string(),
    );

    let mut rt = DenoJsRuntime::new(RuntimeOptions {
        module_loader: Some(std::rc::Rc::new(TestModuleLoader { modules })),
        ..Default::default()
    });

    // Step 1: execute some scripts (like bootstrap)
    rt.execute_script("<setup>", "globalThis.__setup = true".to_string()).unwrap();

    // Step 2: load module (like ChatGPT manifest)
    let mod_id = tokio_rt.block_on(async {
        rt.load_main_es_module(&ModuleSpecifier::parse("file:///test/app.js").unwrap()).await
    }).expect("load failed");

    let receiver = rt.mod_evaluate(mod_id);

    // Step 3: run_event_loop (like run_until_settled)
    tokio_rt.block_on(async {
        let _ = tokio::time::timeout(
            Duration::from_millis(1000),
            rt.run_event_loop(PollEventLoopOptions::default()),
        ).await;
    });

    let _ = tokio_rt.block_on(receiver);

    // Step 4: verify state
    let app_loaded = eval_string(&mut rt, "globalThis.__app_loaded");
    let react_mounted = eval_string(&mut rt, "globalThis.__react_mounted");
    println!("[pipeline] app_loaded={app_loaded} react_mounted={react_mounted}");

    // Step 5: test microtask drain AFTER all this
    let ok = test_microtask(&mut rt, &tokio_rt, "after-full-pipeline");
    if !ok {
        println!(">>> FOUND: full module pipeline breaks microtask drain!");
        println!(">>> The module evaluation + settle cycle leaves V8 broken");
    }
}

/// Test 5: after module + heavy event loop with pending timers
#[test]
fn module_t5_with_timers() {
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all().build().unwrap();

    let mut modules = std::collections::HashMap::new();
    modules.insert(
        "file:///test/timers.js".to_string(),
        r#"
        globalThis.__timer_fired = false;
        setTimeout(() => { globalThis.__timer_fired = true; }, 10);
        // Create many pending promises
        for (let i = 0; i < 100; i++) {
            Promise.resolve(i).then(v => {});
        }
        export default true;
        "#.to_string(),
    );

    let mut rt = DenoJsRuntime::new(RuntimeOptions {
        module_loader: Some(std::rc::Rc::new(TestModuleLoader { modules })),
        ..Default::default()
    });

    let mod_id = tokio_rt.block_on(async {
        rt.load_main_es_module(&ModuleSpecifier::parse("file:///test/timers.js").unwrap()).await
    }).expect("load failed");

    let receiver = rt.mod_evaluate(mod_id);

    // Long settle with timers
    tokio_rt.block_on(async {
        let _ = tokio::time::timeout(
            Duration::from_millis(2000),
            rt.run_event_loop(PollEventLoopOptions::default()),
        ).await;
    });

    let _ = tokio_rt.block_on(receiver);

    let ok = test_microtask(&mut rt, &tokio_rt, "after-timers-module");
    if !ok {
        println!(">>> FOUND: module with timers + settle breaks microtask drain!");
    }
}
