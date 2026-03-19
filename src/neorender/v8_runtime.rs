//! NeoRender V8 Runtime — embeds V8 via deno_core for SPA JS execution.
//! Uses deno_core's native ES module support for proper import/export handling.

use deno_core::{JsRuntime, RuntimeOptions, PollEventLoopOptions, ModuleSpecifier, ModuleLoadResponse, ModuleSource, ModuleSourceCode, ModuleType, RequestedModuleType, ResolutionKind, resolve_import};
use deno_core::error::AnyError;
use std::collections::HashMap;
use std::rc::Rc;
use std::cell::RefCell;
use super::ops;

deno_core::extension!(
    neorender_ext,
    ops = [
        ops::op_neorender_fetch,
        ops::op_neorender_timer,
        ops::op_neorender_pow,
        ops::op_neorender_log,
        ops::op_storage_get,
        ops::op_storage_set,
        ops::op_storage_remove,
        ops::op_storage_clear,
    ],
);

// ─── Module Loader: serves pre-fetched scripts as ES modules ───

/// Stores pre-fetched script contents keyed by URL.
/// When V8 resolves an import, it looks up the content here.
#[derive(Default)]
pub struct ScriptStore {
    pub scripts: HashMap<String, String>,
}

struct NeoModuleLoader {
    store: Rc<RefCell<ScriptStore>>,
}

impl deno_core::ModuleLoader for NeoModuleLoader {
    fn resolve(
        &self,
        specifier: &str,
        referrer: &str,
        _kind: ResolutionKind,
    ) -> Result<ModuleSpecifier, AnyError> {
        Ok(resolve_import(specifier, referrer)?)
    }

    fn load(
        &self,
        module_specifier: &ModuleSpecifier,
        _maybe_referrer: Option<&ModuleSpecifier>,
        _is_dyn_import: bool,
        _requested_module_type: RequestedModuleType,
    ) -> ModuleLoadResponse {
        let url = module_specifier.to_string();
        // Check pre-fetched store first
        {
            let store = self.store.borrow();
            if let Some(code) = store.scripts.get(&url) {
                eprintln!("[NEORENDER:LOADER] store: {} ({}B)", url.rsplit('/').next().unwrap_or(&url), code.len());
                return ModuleLoadResponse::Sync(Ok(ModuleSource::new(
                    ModuleType::JavaScript,
                    ModuleSourceCode::String(code.clone().into()),
                    module_specifier,
                    None,
                )));
            }
        }

        // Not in store — return empty module (should have been pre-fetched)
        eprintln!("[NEORENDER:LOADER] miss: {} (not pre-fetched)", url.rsplit('/').next().unwrap_or(&url));
        ModuleLoadResponse::Sync(Ok(ModuleSource::new(
            ModuleType::JavaScript,
            ModuleSourceCode::String(format!("/* not pre-fetched: {} */", url).into()),
            module_specifier,
            None,
        )))
    }
}

// ─── Runtime creation ───

/// Script store handle — add pre-fetched scripts before loading modules.
pub type ScriptStoreHandle = Rc<RefCell<ScriptStore>>;

/// Create runtime with linkedom DOM pre-initialized from HTML.
/// Injects cookies, localStorage, and location BEFORE bootstrap parses the HTML.
pub fn create_runtime_with_html(
    html: &str,
    url: &str,
    cookies: &crate::ghost::CookieJar,
    local_storage: Option<&std::collections::HashMap<String, String>>,
) -> Result<(JsRuntime, ScriptStoreHandle), String> {
    let store = Rc::new(RefCell::new(ScriptStore::default()));
    let loader = NeoModuleLoader { store: store.clone() };

    let mut runtime = JsRuntime::new(RuntimeOptions {
        extensions: vec![neorender_ext::init_ops()],
        module_loader: Some(Rc::new(loader)),
        ..Default::default()
    });

    // 0. Node.js polyfills required by linkedom (Buffer, process, atob/btoa)
    let node_polyfills = r#"
        // atob/btoa — base64 encoding (not in deno_core by default)
        if (typeof atob === 'undefined') {
            const _c = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
            const _lookup = new Uint8Array(256);
            for (let i = 0; i < _c.length; i++) _lookup[_c.charCodeAt(i)] = i;

            globalThis.atob = function(b64) {
                b64 = String(b64).replace(/[\s=]+/g, '');
                const len = b64.length;
                const bytes = new Uint8Array(Math.floor(len * 3 / 4));
                let p = 0;
                for (let i = 0; i < len; i += 4) {
                    const a = _lookup[b64.charCodeAt(i)];
                    const b = _lookup[b64.charCodeAt(i+1)];
                    const c = _lookup[b64.charCodeAt(i+2)];
                    const d = _lookup[b64.charCodeAt(i+3)];
                    bytes[p++] = (a << 2) | (b >> 4);
                    if (i+2 < len) bytes[p++] = ((b & 15) << 4) | (c >> 2);
                    if (i+3 < len) bytes[p++] = ((c & 3) << 6) | d;
                }
                // Return latin1 string (standard atob behavior)
                let str = '';
                for (let i = 0; i < p; i++) str += String.fromCharCode(bytes[i]);
                return str;
            };

            globalThis.btoa = function(str) {
                str = String(str);
                let out = '';
                for (let i = 0; i < str.length; i += 3) {
                    const a = str.charCodeAt(i);
                    const b = str.charCodeAt(i+1);
                    const c = str.charCodeAt(i+2);
                    out += _c[a >> 2];
                    out += _c[((a & 3) << 4) | (b >> 4)];
                    out += i+1 < str.length ? _c[((b & 15) << 2) | (c >> 6)] : '=';
                    out += i+2 < str.length ? _c[c & 63] : '=';
                }
                return out;
            };
        }

        // Buffer (Node.js compat for linkedom)
        if (typeof Buffer === 'undefined') {
            globalThis.Buffer = {
                from: (input, encoding) => {
                    if (encoding === 'base64') {
                        const decoded = atob(input);
                        return { toString: () => decoded, length: decoded.length };
                    }
                    if (typeof input === 'string') {
                        const enc = new TextEncoder();
                        const buf = enc.encode(input);
                        buf.toString = (e) => e === 'base64' ? btoa(input) : input;
                        return buf;
                    }
                    return input;
                },
                isBuffer: () => false,
                alloc: (size) => new Uint8Array(size),
            };
        }
        if (typeof process === 'undefined') {
            globalThis.process = { env: {}, version: 'v20.0.0', platform: 'linux' };
        }
        // TextEncoder/TextDecoder — may not be exposed globally in deno_core
        if (typeof TextDecoder === 'undefined') {
            globalThis.TextDecoder = class TextDecoder {
                constructor(label) { this.encoding = label || 'utf-8'; }
                decode(input) {
                    if (!input || input.length === 0) return '';
                    const bytes = input instanceof Uint8Array ? input : new Uint8Array(input);
                    let str = '';
                    for (let i = 0; i < bytes.length; i++) str += String.fromCharCode(bytes[i]);
                    return str;
                }
            };
        }
        if (typeof TextEncoder === 'undefined') {
            globalThis.TextEncoder = class TextEncoder {
                constructor() { this.encoding = 'utf-8'; }
                encode(str) {
                    const bytes = [];
                    for (let i = 0; i < str.length; i++) bytes.push(str.charCodeAt(i) & 0xff);
                    return new Uint8Array(bytes);
                }
            };
        }
    "#;
    runtime.execute_script("<neorender:node_polyfills>", node_polyfills.to_string())
        .map_err(|e| format!("Node polyfills error: {e}"))?;

    // 1. Load linkedom — real spec-compliant DOM implementation
    let linkedom_js: String = include_str!("../../js/linkedom.js").to_string();
    runtime.execute_script("<neorender:linkedom>", linkedom_js)
        .map_err(|e| format!("linkedom load error: {e}"))?;

    // 2. Inject data BEFORE bootstrap.js runs (it reads these globals)
    //    a) HTML for linkedom to parse
    let escaped_html = html.replace('\\', "\\\\").replace('`', "\\`").replace("${", "\\${");
    let js = format!("globalThis.__neorender_html = `{}`;", escaped_html);
    runtime.execute_script("<neorender:html>", js)
        .map_err(|e| format!("HTML injection error: {e}"))?;

    //    b) Cookies
    let cookie_map = cookies.all_headers();
    if !cookie_map.is_empty() {
        let cookies_json = serde_json::to_string(&cookie_map).unwrap_or_default();
        let js = format!("globalThis.__neorender_cookies = {};", cookies_json);
        runtime.execute_script("<neorender:cookies>", js)
            .map_err(|e| format!("Cookie injection error: {e}"))?;
        eprintln!("[NEORENDER] Injected cookies for {} domains", cookie_map.len());
    }

    //    c) localStorage
    if let Some(ls) = local_storage {
        if !ls.is_empty() {
            let ls_json = serde_json::to_string(ls).unwrap_or_default();
            let js = format!("globalThis.__neorender_localStorage = {};", ls_json);
            runtime.execute_script("<neorender:ls_data>", js)
                .map_err(|e| format!("localStorage injection error: {e}"))?;
            eprintln!("[NEORENDER] Injected {} localStorage entries", ls.len());
        }
    }

    // 3. Bootstrap — parses HTML via linkedom, sets up fetch/timers/globals
    let boot_js: String = include_str!("../../js/bootstrap.js").to_string();
    runtime.execute_script("<neorender:bootstrap>", boot_js)
        .map_err(|e| format!("Bootstrap error: {e}"))?;

    // 3b. Request interceptor — wraps fetch to log all network requests
    let intercept_js: String = include_str!("../../js/intercept.js").to_string();
    runtime.execute_script("<neorender:intercept>", intercept_js)
        .map_err(|e| format!("Intercept load error: {e}"))?;

    // 3c. WOM extraction function — extracts page data directly from linkedom DOM
    let wom_js: String = include_str!("../../js/wom.js").to_string();
    runtime.execute_script("<neorender:wom>", wom_js)
        .map_err(|e| format!("WOM load error: {e}"))?;

    // 3d. DOM tree extraction — full DOM as JSON tree (__neo_dom_tree)
    let dom_tree_js: String = include_str!("../../js/dom_tree.js").to_string();
    runtime.execute_script("<neorender:dom_tree>", dom_tree_js)
        .map_err(|e| format!("DOM tree load error: {e}"))?;

    // 3e. Observer — MutationObserver + snapshot diff (__neo_get_mutations, __neo_get_diff)
    let observer_js: String = include_str!("../../js/observer.js").to_string();
    runtime.execute_script("<neorender:observer>", observer_js)
        .map_err(|e| format!("Observer load error: {e}"))?;

    // 3f. Browser bridge — event listeners + interaction API
    let browser_js: String = include_str!("../../js/browser.js").to_string();
    runtime.execute_script("<neorender:browser>", browser_js)
        .map_err(|e| format!("Browser bridge load error: {e}"))?;

    // 4. Set location (after bootstrap, so location object exists)
    set_location(&mut runtime, url)?;

    // 4b. Sync document.location with window.location (linkedom doesn't do this)
    runtime.execute_script("<neorender:doc_location>",
        "document.location = location; try { document.baseURI = location.href; } catch {}".to_string()
    ).map_err(|e| format!("doc.location sync error: {e}"))?;

    // 5. Populate localStorage from injected data
    runtime.execute_script("<neorender:ls_populate>",
        "if(globalThis.__neorender_localStorage){Object.entries(__neorender_localStorage).forEach(([k,v])=>localStorage.setItem(k,v));}".to_string()
    ).map_err(|e| format!("localStorage populate error: {e}"))?;

    Ok((runtime, store))
}

// ─── Script & module execution ───

pub fn set_location(runtime: &mut JsRuntime, url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|e| format!("Invalid URL: {e}"))?;
    let js = format!(
        r#"location.href="{}";location.protocol="{}:";location.host="{}";location.hostname="{}";location.port="{}";location.pathname="{}";location.search="{}";location.hash="{}";location.origin="{}";"#,
        url, parsed.scheme(),
        parsed.host_str().unwrap_or(""), parsed.host_str().unwrap_or(""),
        parsed.port().map(|p| p.to_string()).unwrap_or_default(),
        parsed.path(),
        parsed.query().map(|q| format!("?{q}")).unwrap_or_default(),
        parsed.fragment().map(|f| format!("#{f}")).unwrap_or_default(),
        parsed.origin().ascii_serialization(),
    );
    runtime.execute_script("<neorender:location>", js)
        .map_err(|e| format!("Location error: {e}"))?;
    Ok(())
}

pub fn populate_dom(runtime: &mut JsRuntime, html: &str) -> Result<(), String> {
    let js = crate::neorender::dom_export::html_to_dom_js(html);
    runtime.execute_script("<neorender:populate>", js)
        .map_err(|e| format!("DOM populate error: {e}"))?;
    Ok(())
}

/// Execute a regular (non-module) script.
/// Wraps in try-catch for error isolation — script errors don't crash the render.
pub fn execute_script(runtime: &mut JsRuntime, script: String, name: String) -> Option<String> {
    // Wrap in try-catch so uncaught errors don't abort V8
    let wrapped = format!("try {{ {} }} catch(__e) {{ /* non-fatal */ }}", script);
    match runtime.execute_script("<page>", wrapped) {
        Ok(_) => None,
        Err(e) => {
            let msg = format!("[{}] {}", name, first_line(&e.to_string()));
            eprintln!("[NEORENDER] Script error (non-fatal): {msg}");
            Some(msg)
        }
    }
}

/// Load and execute an ES module using deno_core's native module system.
/// The module's imports are resolved via NeoModuleLoader from the ScriptStore.
pub async fn execute_module(runtime: &mut JsRuntime, url: &str, name: String) -> Option<String> {
    let specifier = match ModuleSpecifier::parse(url) {
        Ok(s) => s,
        Err(e) => return Some(format!("[{}] Bad URL: {}", name, e)),
    };

    let mod_id = match runtime.load_main_es_module(&specifier).await {
        Ok(id) => id,
        Err(e) => return Some(format!("[{}] Module load: {}", name, first_line(&e.to_string()))),
    };

    let eval_result = runtime.mod_evaluate(mod_id);

    // Run event loop to resolve imports and execute
    if let Err(e) = runtime.run_event_loop(PollEventLoopOptions::default()).await {
        eprintln!("[NEORENDER] Module event loop error: {e}");
    }

    match eval_result.await {
        Ok(()) => None,
        Err(e) => {
            let msg = format!("[{}] {}", name, first_line(&e.to_string()));
            eprintln!("[NEORENDER] Module eval error: {msg}");
            Some(msg)
        }
    }
}

/// Load and execute an ES module as a side module (for 2nd+ modules).
/// Uses load_side_es_module instead of load_main_es_module to avoid "main module already loaded" error.
pub async fn execute_side_module(runtime: &mut JsRuntime, url: &str, name: String) -> Option<String> {
    let specifier = match ModuleSpecifier::parse(url) {
        Ok(s) => s,
        Err(e) => return Some(format!("[{}] Bad URL: {}", name, e)),
    };

    let mod_id = match runtime.load_side_es_module(&specifier).await {
        Ok(id) => id,
        Err(e) => return Some(format!("[{}] Side module load: {}", name, first_line(&e.to_string()))),
    };

    let eval_result = runtime.mod_evaluate(mod_id);

    // Run event loop to resolve imports and execute
    if let Err(e) = runtime.run_event_loop(PollEventLoopOptions::default()).await {
        eprintln!("[NEORENDER] Side module event loop error: {e}");
    }

    match eval_result.await {
        Ok(()) => None,
        Err(e) => {
            let msg = format!("[{}] {}", name, first_line(&e.to_string()));
            eprintln!("[NEORENDER] Side module eval error: {msg}");
            Some(msg)
        }
    }
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or(s)
}

pub async fn run_event_loop(runtime: &mut JsRuntime, timeout_ms: u64) -> Result<(), String> {
    match tokio::time::timeout(
        std::time::Duration::from_millis(timeout_ms),
        runtime.run_event_loop(PollEventLoopOptions::default()),
    ).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => { eprintln!("[NEORENDER] Event loop error: {e}"); Ok(()) }
        Err(_) => { eprintln!("[NEORENDER] Event loop timeout {timeout_ms}ms"); Ok(()) }
    }
}

pub fn export_dom_html(runtime: &mut JsRuntime) -> Result<String, String> {
    let result = runtime.execute_script("<neorender:export>", "__neorender_export()".to_string())
        .map_err(|e| format!("DOM export error: {e}"))?;

    let scope = &mut runtime.handle_scope();
    let local = deno_core::v8::Local::new(scope, result);
    if let Some(s) = local.to_string(scope) {
        Ok(s.to_rust_string_lossy(scope))
    } else {
        Ok("<html><head></head><body></body></html>".to_string())
    }
}
