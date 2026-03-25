//! Test: heavy module with React-like patterns + run_until_settled
//!
//! Run: cargo test -p neo-runtime --test heavy_module_test -- --nocapture

use deno_core::{
    JsRuntime as DenoJsRuntime, ModuleLoadOptions, ModuleLoadResponse, ModuleLoader,
    ModuleLoadReferrer, ModuleSource, ModuleSourceCode, ModuleSpecifier, ModuleType,
    PollEventLoopOptions, ResolutionKind, RuntimeOptions,
};
use std::time::Duration;

struct TestLoader {
    modules: std::collections::HashMap<String, String>,
}

impl ModuleLoader for TestLoader {
    fn resolve(&self, specifier: &str, referrer: &str, _kind: ResolutionKind,
    ) -> Result<ModuleSpecifier, deno_error::JsErrorBox> {
        let base = ModuleSpecifier::parse(referrer)
            .unwrap_or_else(|_| ModuleSpecifier::parse("file:///test/").unwrap());
        if specifier.starts_with("file:") || specifier.starts_with("http") {
            ModuleSpecifier::parse(specifier).map_err(deno_error::JsErrorBox::from_err)
        } else {
            base.join(specifier).map_err(deno_error::JsErrorBox::from_err)
        }
    }

    fn load(&self, spec: &ModuleSpecifier, _ref: Option<&ModuleLoadReferrer>,
            _options: ModuleLoadOptions) -> ModuleLoadResponse {
        let url = spec.to_string();
        match self.modules.get(&url) {
            Some(src) => ModuleLoadResponse::Sync(Ok(ModuleSource::new(
                ModuleType::JavaScript,
                ModuleSourceCode::String(src.clone().into()),
                spec, None,
            ))),
            None => ModuleLoadResponse::Sync(Err(
                deno_error::JsErrorBox::generic(format!("not found: {url}"))
            )),
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
    let var = format!("__mt_{}", label.replace(['-', ' ', '.'], "_"));
    rt.execute_script("<set>", format!(
        "globalThis.{v}='B';Promise.resolve().then(function(){{globalThis.{v}='A'}})", v = var
    )).unwrap();
    // deno_core 0.393: execute_script doesn't auto-drain microtasks
    let _ = tokio_rt.block_on(rt.run_event_loop(PollEventLoopOptions::default()));
    let r = eval_string(rt, &format!("globalThis.{}", var));
    let ok = r == "A";
    println!("[{label:40}] {r} -> {}", if ok { "PASS" } else { "**FAIL**" });
    ok
}

/// Generate a heavy React-like module (many closures, promises, state)
fn generate_heavy_module() -> String {
    let mut code = String::new();
    code.push_str("// Heavy React-like module\n");

    // DataDog-like instrumentation wrapper
    code.push_str(r#"
    const __dd_instrumented = new WeakMap();
    function __dd_instrument(target, name) {
        if (__dd_instrumented.has(target)) return;
        const orig = target[name];
        if (typeof orig === 'function') {
            target[name] = function() {
                return orig.apply(this, arguments);
            };
            __dd_instrumented.set(target, true);
        }
    }
    // Instrument fetch
    if (typeof globalThis.fetch === 'function') {
        __dd_instrument(globalThis, 'fetch');
    }
    "#);

    // React-like scheduler
    code.push_str(r#"
    const __scheduler = {
        queue: [],
        pending: false,
        schedule(cb) {
            this.queue.push(cb);
            if (!this.pending) {
                this.pending = true;
                Promise.resolve().then(() => {
                    this.pending = false;
                    const tasks = this.queue.splice(0);
                    tasks.forEach(t => t());
                });
            }
        }
    };
    globalThis.__scheduler = __scheduler;
    "#);

    // Many component-like closures
    for i in 0..100 {
        code.push_str(&format!(r#"
        (function component_{i}() {{
            const state = {{ count: 0, data: null }};
            const setState = (update) => {{
                Object.assign(state, update);
                __scheduler.schedule(() => {{ /* re-render */ }});
            }};
            // Simulate useEffect
            Promise.resolve().then(() => {{
                setState({{ data: 'loaded_{i}' }});
            }});
        }})();
        "#));
    }

    // React Query-like cache
    code.push_str(r#"
    globalThis.__queryCache = new Map();
    function useQuery(key, fn) {
        if (!globalThis.__queryCache.has(key)) {
            const entry = { status: 'pending', data: null, error: null };
            globalThis.__queryCache.set(key, entry);
            Promise.resolve().then(() => {
                try {
                    entry.data = fn();
                    entry.status = 'success';
                } catch(e) {
                    entry.error = e;
                    entry.status = 'error';
                }
            });
        }
        return globalThis.__queryCache.get(key);
    }
    // Create many queries
    for (let i = 0; i < 50; i++) {
        useQuery('key_' + i, () => 'value_' + i);
    }
    "#);

    code.push_str("\nexport default { loaded: true };\n");
    code
}

#[test]
fn heavy_module_microtask() {
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all().build().unwrap();

    let mut modules = std::collections::HashMap::new();
    let heavy = generate_heavy_module();
    println!("Heavy module size: {} bytes", heavy.len());
    modules.insert("file:///test/heavy.js".to_string(), heavy);

    let mut rt = DenoJsRuntime::new(RuntimeOptions {
        extensions: vec![neo_runtime::v8::neo_runtime_ext::init()],
        module_loader: Some(std::rc::Rc::new(TestLoader { modules })),
        ..Default::default()
    });

    assert!(test_microtask(&mut rt, &tokio_rt, "before-heavy-module"));

    // Load module
    let mod_id = tokio_rt.block_on(async {
        rt.load_main_es_module(&ModuleSpecifier::parse("file:///test/heavy.js").unwrap()).await
    }).expect("load failed");

    let receiver = rt.mod_evaluate(mod_id);

    // Settle like run_until_settled with watchdog
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Instant;

    let isolate_handle = rt.v8_isolate().thread_safe_handle();
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let cancel_clone = cancel_flag.clone();
    let deadline = Instant::now() + Duration::from_millis(5000);

    let watchdog = std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(50));
            if cancel_clone.load(Ordering::Relaxed) { return; }
            if Instant::now() >= deadline {
                eprintln!("[watchdog] TERMINATING");
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
    rt.v8_isolate().cancel_terminate_execution();
    let _ = rt.execute_script("<recovery>", "void 0".to_string());

    let _ = tokio_rt.block_on(receiver);

    // Test microtask after heavy module + settle + possible watchdog terminate
    let ok = test_microtask(&mut rt, &tokio_rt, "after-heavy-module-settle");
    if !ok {
        println!(">>> FOUND: heavy module + settle with watchdog breaks microtask drain!");

        // Check if terminate was actually called
        let running = eval_string(&mut rt, "globalThis.__scheduler?.queue?.length || 'no scheduler'");
        println!(">>> scheduler queue: {running}");
    }
}
