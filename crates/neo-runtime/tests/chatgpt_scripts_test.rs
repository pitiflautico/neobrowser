//! Test which ChatGPT inline script breaks microtask drain.
//! Loads each script one at a time on top of the full bootstrap stack.
//!
//! Run: cargo test -p neo-runtime --test chatgpt_scripts_test -- --nocapture

use deno_core::{JsRuntime as DenoJsRuntime, PollEventLoopOptions, RuntimeOptions};
use std::time::Duration;

fn setup_engine() -> DenoJsRuntime {
    let mut rt = DenoJsRuntime::new(RuntimeOptions {
        extensions: vec![neo_runtime::v8::neo_runtime_ext::init()],
        ..Default::default()
    });

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

    rt
}

fn test_microtask(rt: &mut DenoJsRuntime, label: &str) -> bool {
    let var = format!("__mt_{}", label.replace("-", "_").replace(" ", "_"));
    let code = format!(
        "(function(){{globalThis.{v}='B';Promise.resolve().then(function(){{globalThis.{v}='A'}});return globalThis.{v}}})()",
        v = var
    );
    let wrapped = format!(
        "(function(){{var __r;try{{__r=({code})}}catch(__e){{__r='Error: '+__e.message}};if(typeof globalThis.__neo_drainMicrotasks==='function')globalThis.__neo_drainMicrotasks();return __r}})()"
    );
    let result = rt.execute_script("<test>", wrapped).expect("exec failed");
    let r1 = {
        let context = rt.main_context();
        deno_core::v8::scope!(scope, rt.v8_isolate());
        let context = deno_core::v8::Local::new(scope, context);
        let scope = &mut deno_core::v8::ContextScope::new(scope, context);
        let local = deno_core::v8::Local::new(scope,result);
        local.to_string(scope).map(|s| s.to_rust_string_lossy(scope)).unwrap_or_default()
    };

    let result2 = rt.execute_script("<read>", format!("String(globalThis.{})", var)).expect("read failed");
    let r2 = {
        let context = rt.main_context();
        deno_core::v8::scope!(scope, rt.v8_isolate());
        let context = deno_core::v8::Local::new(scope, context);
        let scope = &mut deno_core::v8::ContextScope::new(scope, context);
        let local = deno_core::v8::Local::new(scope,result2);
        local.to_string(scope).map(|s| s.to_rust_string_lossy(scope)).unwrap_or_default()
    };

    let ok = r2 == "A";
    println!("[{label:30}] eval={r1} read={r2} -> {}", if ok { "PASS" } else { "**FAIL**" });
    ok
}

// ChatGPT inline scripts extracted from the page
const SCRIPT_THEME: &str = r#"!function(){try{var d=document.documentElement,c=d.classList;c.remove('light','dark');var e=localStorage.getItem('theme');if('system'===e||(!e&&true)){var t='(prefers-color-scheme: dark)',m=window.matchMedia(t);if(m.media!==t||m.matches){d.style.colorScheme = 'dark';c.add('dark')}else{d.style.colorScheme = 'light';c.add('light')}}else if(e){d.style.colorScheme = e;c.add(e)}}catch(e){}}()"#;

const SCRIPT_ROUTER_CONTEXT: &str = r#"window.__reactRouterContext = {"basename":"/","future":{"unstable_optimizeDeps":false,"unstable_subResourceIntegrity":false},"isSpaMode":false,"stream":new ReadableStream({start(controller){window.__reactRouterContext.streamController = controller;}}).pipeThrough(new TextEncoderStream())}"#;

const SCRIPT_TURBO_ENQUEUE: &str = r#"window.__reactRouterContext.streamController.enqueue("[{\"_1\":2}]")"#;

const SCRIPT_TURBO_CLOSE: &str = r#"window.__reactRouterContext.streamController.close()"#;

const SCRIPT_RAF: &str = r#"requestAnimationFrame(function(){$RT=performance.now()})"#;

const SCRIPT_REACT_SUSPENSE: &str = r#"$RB=[];$RV=function(a){$RT=performance.now();for(var b=0;b<a.length;b+=2){var c=a[b],e=a[b+1];null!=c&&null!=e&&(c=document.getElementById(c))&&(e=document.getElementById(e))&&c.parentNode&&e.parentNode&&(a[b]=c,a[b+1]=e)}for(b=0;b<a.length;b+=2)if(null!=(c=a[b])&&null!=(e=a[b+1])){for(var f=c.nextSibling,d=0,g=e.parentNode,l=e.previousSibling;;){if(f===e)break;var n=f.nextSibling;g.insertBefore(f,l?l.nextSibling:g.firstChild);l=f;f=n;d++}for(;d--;)f=c.nextSibling,g.removeChild(f);c.parentNode.removeChild(c);e.parentNode.removeChild(e)}};$RC=function(a,b){a=document.getElementById(a);b=document.getElementById(b);for(a.parentNode.removeChild(a);a.firstChild;)b.parentNode.insertBefore(a.firstChild,b);b.parentNode.removeChild(b);$RB.push(b.id)}"#;

const SCRIPT_RC1: &str = r#"$RC("B:1","S:1")"#;

const SCRIPT_REACT_QUERY: &str = r#"window.ReactQueryError ??= class ReactQueryError extends Error {};
null==Promise.withResolvers&&(Promise.withResolvers=function(){let e,r;return{promise:new Promise((s,o)=>{e=s,r=o}),resolve:e,reject:r}});
window.__REACT_QUERY_CACHE__ ??= {}"#;

const SCRIPT_RC2: &str = r#"$RC("B:2","S:2")"#;

const SCRIPT_IFRAME: &str = r#"(function(){function c(){var b=a.contentDocument||a.contentWindow.document;if(b){var d=b.createElement("script");d.textContent='void 0';b.head.appendChild(d)}}var a=document.createElement("iframe");a.style.display="none";a.onload=c;document.body.appendChild(a)})()"#;

const SCRIPT_SSR_HTML: &str = r#"window.__oai_logHTML?window.__oai_logHTML():window.__oai_SSR_HTML=window.__oai_SSR_HTML||Date.now()"#;

/// Test each ChatGPT script individually
#[test]
fn chatgpt_scripts_individual() {
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all().build().unwrap();
    let _guard = tokio_rt.enter();

    let scripts: Vec<(&str, &str)> = vec![
        ("baseline", ""),
        ("theme", SCRIPT_THEME),
        ("router-context", SCRIPT_ROUTER_CONTEXT),
        ("react-query", SCRIPT_REACT_QUERY),
        ("raf", SCRIPT_RAF),
        ("react-suspense", SCRIPT_REACT_SUSPENSE),
        ("ssr-html", SCRIPT_SSR_HTML),
        ("iframe", SCRIPT_IFRAME),
    ];

    for (name, script) in &scripts {
        let mut rt = setup_engine();

        // Run settle like page load does
        tokio_rt.block_on(async {
            let _ = tokio::time::timeout(
                Duration::from_millis(100),
                rt.run_event_loop(PollEventLoopOptions::default()),
            ).await;
        });

        if !script.is_empty() {
            let sname: &'static str = Box::leak(format!("<chatgpt-{}>", name).into_boxed_str());
            let _ = rt.execute_script(sname, script.to_string());
        }

        test_microtask(&mut rt, name);
    }
}

/// Test scripts cumulatively (add one at a time like ChatGPT page load)
#[test]
fn chatgpt_scripts_cumulative() {
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all().build().unwrap();
    let _guard = tokio_rt.enter();

    let mut rt = setup_engine();

    // Run initial settle
    tokio_rt.block_on(async {
        let _ = tokio::time::timeout(
            Duration::from_millis(100),
            rt.run_event_loop(PollEventLoopOptions::default()),
        ).await;
    });

    println!("\n=== Cumulative test ===");
    test_microtask(&mut rt, "0-baseline");

    let scripts: Vec<(&str, &str)> = vec![
        ("1-theme", SCRIPT_THEME),
        ("2-router-ctx", SCRIPT_ROUTER_CONTEXT),
        ("3-turbo-enq", SCRIPT_TURBO_ENQUEUE),
        ("4-turbo-close", SCRIPT_TURBO_CLOSE),
        ("5-raf", SCRIPT_RAF),
        ("6-suspense", SCRIPT_REACT_SUSPENSE),
        ("7-rc1", SCRIPT_RC1),
        ("8-react-query", SCRIPT_REACT_QUERY),
        ("9-rc2", SCRIPT_RC2),
        ("10-iframe", SCRIPT_IFRAME),
        ("11-ssr-html", SCRIPT_SSR_HTML),
    ];

    for (name, script) in &scripts {
        let sname: &'static str = Box::leak(format!("<chatgpt-{}>", name).into_boxed_str());
        let _ = rt.execute_script(sname, script.to_string());

        // Settle after each script (like page load pipeline)
        tokio_rt.block_on(async {
            let _ = tokio::time::timeout(
                Duration::from_millis(50),
                rt.run_event_loop(PollEventLoopOptions::default()),
            ).await;
        });

        if !test_microtask(&mut rt, name) {
            println!(">>> FOUND IT: script '{}' broke microtask drain!", name);
            println!(">>> Script content: {}", &script[..script.len().min(200)]);
            break;
        }
    }
}

/// Test with run_event_loop between scripts (simulate full page load settle)
#[test]
fn chatgpt_scripts_with_heavy_settle() {
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all().build().unwrap();
    let _guard = tokio_rt.enter();

    let mut rt = setup_engine();

    // Load ALL scripts
    let all_scripts = [
        SCRIPT_THEME, SCRIPT_ROUTER_CONTEXT,
        SCRIPT_REACT_QUERY, SCRIPT_RAF,
        SCRIPT_REACT_SUSPENSE, SCRIPT_SSR_HTML,
    ];
    for (i, script) in all_scripts.iter().enumerate() {
        let sname: &'static str = Box::leak(format!("<chatgpt-all-{}>", i).into_boxed_str());
        let _ = rt.execute_script(sname, script.to_string());
    }

    // Heavy settle (like run_until_settled with 2s timeout)
    println!("\n=== Heavy settle test ===");
    tokio_rt.block_on(async {
        let _ = tokio::time::timeout(
            Duration::from_millis(2000),
            rt.run_event_loop(PollEventLoopOptions::default()),
        ).await;
    });

    let ok = test_microtask(&mut rt, "after-heavy-settle");
    if !ok {
        println!(">>> CRITICAL: heavy settle breaks microtask drain!");
        println!(">>> This means run_event_loop leaves V8 in a broken state");
    }
}
