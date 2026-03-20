//! NeoRender V8 Runtime — embeds V8 via deno_core for SPA JS execution.
//! Uses deno_core's native ES module support for proper import/export handling.

use deno_core::{JsRuntime, RuntimeOptions, PollEventLoopOptions, ModuleSpecifier, ModuleLoadResponse, ModuleSource, ModuleSourceCode, ModuleType, RequestedModuleType, ResolutionKind, resolve_import, SourceCodeCacheInfo};
use deno_core::error::AnyError;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::rc::Rc;
use std::cell::RefCell;
use std::pin::Pin;
use std::future::Future;
use futures::FutureExt;
use super::ops;

// ─── V8 Bytecode Cache ───

/// Disk-backed V8 compiled bytecode cache.
/// Directory: `~/.neobrowser/cache/v8/`
/// File format: [8 bytes source_hash LE] [V8 bytecode...]
struct V8CodeCache {
    cache_dir: PathBuf,
}

impl V8CodeCache {
    fn new() -> Option<Self> {
        let home = dirs::home_dir()?;
        let cache_dir = home.join(".neobrowser").join("cache").join("v8");
        if let Err(e) = std::fs::create_dir_all(&cache_dir) {
            eprintln!("[V8CACHE] Failed to create cache dir: {e}");
            return None;
        }
        eprintln!("[V8CACHE] Dir: {}", cache_dir.display());
        Some(Self { cache_dir })
    }

    /// Deterministic filename from URL
    fn cache_path(&self, url: &str) -> PathBuf {
        let mut hasher = DefaultHasher::new();
        url.hash(&mut hasher);
        let url_hash = hasher.finish();
        self.cache_dir.join(format!("{:016x}.v8cache", url_hash))
    }

    /// Hash source code for invalidation
    fn hash_source(code: &str) -> u64 {
        let mut hasher = DefaultHasher::new();
        code.hash(&mut hasher);
        hasher.finish()
    }

    /// Try to read cached bytecode. Returns None if missing or source hash mismatch.
    fn read(&self, url: &str, source_hash: u64) -> Option<Vec<u8>> {
        let path = self.cache_path(url);
        let data = std::fs::read(&path).ok()?;
        if data.len() < 8 {
            return None;
        }
        let stored_hash = u64::from_le_bytes(data[..8].try_into().ok()?);
        if stored_hash != source_hash {
            eprintln!("[V8CACHE] Stale: {} (hash mismatch)", url.rsplit('/').next().unwrap_or(url));
            return None;
        }
        let bytecode = data[8..].to_vec();
        eprintln!("[V8CACHE] Hit: {} ({}B bytecode)", url.rsplit('/').next().unwrap_or(url), bytecode.len());
        Some(bytecode)
    }

    /// Write bytecode to disk with source hash prefix.
    fn write(&self, url: &str, source_hash: u64, bytecode: &[u8]) {
        let path = self.cache_path(url);
        let mut data = Vec::with_capacity(8 + bytecode.len());
        data.extend_from_slice(&source_hash.to_le_bytes());
        data.extend_from_slice(bytecode);
        match std::fs::write(&path, &data) {
            Ok(_) => eprintln!("[V8CACHE] Wrote: {} ({}B)", url.rsplit('/').next().unwrap_or(url), bytecode.len()),
            Err(e) => eprintln!("[V8CACHE] Write error: {e}"),
        }
    }
}

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
        ops::op_chatgpt_pow,
        ops::op_cookie_get,
        ops::op_cookie_set,
    ],
);

// ─── Module Loader: serves pre-fetched scripts as ES modules ───

/// Stores pre-fetched script contents keyed by URL.
/// When V8 resolves an import, it looks up the content here.
#[derive(Default)]
pub struct ScriptStore {
    pub scripts: HashMap<String, String>,
    /// URLs that returned non-JS content (HTML) — skip on repeat requests
    pub failed_urls: HashSet<String>,
    /// URLs of heavy modules to stub — V8 gets a minimal re-export skeleton
    /// instead of parsing multi-MB bundles not needed for hydration.
    pub stub_modules: HashSet<String>,
}

/// Extract named export identifiers from minified JS module source.
/// Handles: `export{a as b,c}`, `export function x`, `export const x`,
/// `export default`, and re-exports `export{x}from"..."`.
fn extract_export_names(js: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut seen = HashSet::new();

    // Match export{...} blocks (local exports and re-exports)
    if let Ok(re) = regex_lite::Regex::new(r"export\s*\{([^}]+)\}") {
        for cap in re.captures_iter(js) {
            let block = &cap[1];
            for item in block.split(',') {
                let item = item.trim();
                let exported = if let Some(pos) = item.rfind(" as ") {
                    item[pos + 4..].trim()
                } else {
                    item.trim()
                };
                let clean = exported.trim().trim_matches('"').trim_matches('\'');
                if !clean.is_empty() && clean != "default" && seen.insert(clean.to_string()) {
                    names.push(clean.to_string());
                }
            }
        }
    }

    // Match: export function|const|let|var|class NAME
    if let Ok(re) = regex_lite::Regex::new(
        r"export\s+(?:function|const|let|var|class)\s+([a-zA-Z_$][a-zA-Z0-9_$]*)"
    ) {
        for cap in re.captures_iter(js) {
            let name = cap[1].to_string();
            if seen.insert(name.clone()) {
                names.push(name);
            }
        }
    }

    // Check for default export
    let has_default = js.contains("export default")
        || (js.contains("export{") && js.contains("as default"));
    if has_default && seen.insert("default".to_string()) {
        names.push("default".to_string());
    }

    names
}

/// Generate a stub ES module that re-exports no-op values for each name.
/// Property access on any export returns a no-op function (handles chained calls
/// like `telemetry.instance.addFirstTiming(...)`).
fn generate_stub_module(export_names: &[String]) -> String {
    let mut parts = Vec::new();

    // _n: no-op function returning itself (chained calls: x.y.z())
    // _o: Proxy returning _n for any prop (x.instance.method())
    // .then=undefined prevents Promise detection by await/Promise.resolve
    parts.push("const _n=()=>_n;_n.then=undefined;const _o=new Proxy({},{get:(t,p)=>p==='then'?undefined:_n});".to_string());

    let mut export_items = Vec::new();
    for name in export_names {
        if name == "default" { continue; }
        parts.push(format!("const {}=_o;", name));
        export_items.push(name.clone());
    }

    if !export_items.is_empty() {
        parts.push(format!("export{{{}}};", export_items.join(",")));
    }

    parts.push("export default _o;".to_string());
    parts.join("")
}

struct NeoModuleLoader {
    store: Rc<RefCell<ScriptStore>>,
    code_cache: Option<Rc<V8CodeCache>>,
}

impl NeoModuleLoader {
    /// Build SourceCodeCacheInfo for a module: hash the source, look up cached bytecode.
    fn make_cache_info(&self, url: &str, source: &str) -> Option<SourceCodeCacheInfo> {
        let cache = self.code_cache.as_ref()?;
        let source_hash = V8CodeCache::hash_source(source);
        let cached_bytecode = cache.read(url, source_hash);
        Some(SourceCodeCacheInfo {
            hash: source_hash,
            data: cached_bytecode.map(|b| Cow::Owned(b)),
        })
    }
}

impl deno_core::ModuleLoader for NeoModuleLoader {
    fn resolve(
        &self,
        specifier: &str,
        referrer: &str,
        _kind: ResolutionKind,
    ) -> Result<ModuleSpecifier, AnyError> {
        match resolve_import(specifier, referrer) {
            Ok(s) => Ok(s),
            Err(e) => {
                eprintln!("[NEORENDER:RESOLVE] FAIL: spec={} ref={} err={}",
                    &specifier[..specifier.len().min(60)],
                    &referrer[..referrer.len().min(60)], e);
                Err(e.into())
            }
        }
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
                let short = url.rsplit('/').next().unwrap_or(&url);

                // ── Selective module stubbing ──
                // For heavy modules marked as non-essential for hydration,
                // extract their export names and return a lightweight stub.
                // This avoids V8 parsing multi-MB bundles (e.g. telemetry, app bundles).
                if store.stub_modules.contains(&url) {
                    let exports = extract_export_names(code);
                    let stub = generate_stub_module(&exports);
                    eprintln!("[NEORENDER:LOADER] STUB: {} ({}B → {}B, {} exports)",
                        short, code.len(), stub.len(), exports.len());
                    return ModuleLoadResponse::Sync(Ok(ModuleSource::new(
                        ModuleType::JavaScript,
                        ModuleSourceCode::String(stub.into()),
                        module_specifier,
                        None,
                    )));
                }

                eprintln!("[NEORENDER:LOADER] store: {} ({}B)", short, code.len());
                // Source-level transform: replace Promise.allSettled calls with inline
                // equivalents. Polyfill injection doesn't work in deno_core 0.311 module
                // evaluation contexts, so we rewrite the call sites directly.
                let patched = if code.contains("Promise.allSettled(") {
                    code.replace(
                        "Promise.allSettled(",
                        "((ps)=>Promise.all([...ps].map(p=>Promise.resolve(p).then(v=>({status:'fulfilled',value:v}),r=>({status:'rejected',reason:r})))))("
                    )
                } else {
                    code.clone()
                };
                // V8 bytecode cache: provide cached bytecode if available
                let cache_info = self.make_cache_info(&url, &patched);
                return ModuleLoadResponse::Sync(Ok(ModuleSource::new(
                    ModuleType::JavaScript,
                    ModuleSourceCode::String(patched.into()),
                    module_specifier,
                    cache_info,
                )));
            }
        }

        // Not in store — fetch on-demand (like a real browser would)
        let short = url.rsplit('/').next().unwrap_or(&url).to_string();
        // Skip URLs that previously returned non-JS content (HTML error pages, etc.)
        {
            let store = self.store.borrow();
            if store.failed_urls.contains(&url) {
                eprintln!("[NEORENDER:LOADER] skip: {} (cached failure)", short);
                return ModuleLoadResponse::Sync(Ok(ModuleSource::new(
                    ModuleType::JavaScript,
                    ModuleSourceCode::String("/* skipped: cached failure */".to_string().into()),
                    module_specifier,
                    None,
                )));
            }
        }
        // Skip empty URLs or non-JS URLs (HTML pages return '<' which causes parse errors)
        if short.is_empty() || url.ends_with('/') || (!url.contains(".js") && !url.contains(".mjs")) {
            eprintln!("[NEORENDER:LOADER] skip: {} (not a JS module)", short);
            return ModuleLoadResponse::Sync(Ok(ModuleSource::new(
                ModuleType::JavaScript,
                ModuleSourceCode::String("/* skipped: not a JS module */".to_string().into()),
                module_specifier,
                None,
            )));
        }
        eprintln!("[NEORENDER:LOADER] miss: {} — fetching on-demand", short);
        let store = self.store.clone();
        let code_cache = self.code_cache.clone();
        let spec = module_specifier.clone();
        let fetch_url = url.clone();
        ModuleLoadResponse::Async(Box::pin(async move {
            let client = rquest::Client::builder()
                .emulation(rquest_util::Emulation::Chrome136)
                .build()
                .map_err(|e| deno_core::anyhow::anyhow!("Client error: {e}"))?;
            match tokio::time::timeout(
                std::time::Duration::from_secs(2),
                client.get(&fetch_url).send(),
            ).await {
                Ok(Ok(resp)) => {
                    let code = resp.text().await.map_err(|e| deno_core::anyhow::anyhow!("Body error: {e}"))?;
                    eprintln!("[NEORENDER:LOADER] fetched: {} ({}B)", short, code.len());
                    // If response is HTML (not JS), cache as failed to skip future requests
                    if code.trim_start().starts_with('<') {
                        eprintln!("[NEORENDER:LOADER] skip: {} (HTML response, caching failure)", short);
                        store.borrow_mut().failed_urls.insert(fetch_url);
                        return Ok(ModuleSource::new(
                            ModuleType::JavaScript,
                            ModuleSourceCode::String("/* skipped: HTML response */".to_string().into()),
                            &spec,
                            None,
                        ));
                    }
                    store.borrow_mut().scripts.insert(fetch_url.clone(), code.clone());

                    // On-demand stub: if fetched module is very large and stub_modules
                    // threshold is active, stub it to avoid massive V8 parse.
                    // Uses 1MB default; NEOBROWSER_STUB_THRESHOLD=0 disables.
                    let stub_threshold: usize = std::env::var("NEOBROWSER_STUB_THRESHOLD")
                        .ok().and_then(|v| v.parse().ok()).unwrap_or(1_000_000);
                    if stub_threshold > 0 && code.len() >= stub_threshold {
                        let exports = extract_export_names(&code);
                        let stub = generate_stub_module(&exports);
                        eprintln!("[NEORENDER:LOADER] STUB (on-demand): {} ({}B → {}B, {} exports)",
                            short, code.len(), stub.len(), exports.len());
                        return Ok(ModuleSource::new(
                            ModuleType::JavaScript,
                            ModuleSourceCode::String(stub.into()),
                            &spec,
                            None,
                        ));
                    }

                    // Apply same Promise.allSettled source transform as pre-fetched modules
                    let patched = if code.contains("Promise.allSettled(") {
                        code.replace(
                            "Promise.allSettled(",
                            "((ps)=>Promise.all([...ps].map(p=>Promise.resolve(p).then(v=>({status:'fulfilled',value:v}),r=>({status:'rejected',reason:r})))))("
                        )
                    } else {
                        code
                    };
                    // V8 bytecode cache: check for cached bytecode from previous sessions
                    let cache_info = code_cache.as_ref().map(|cache| {
                        let source_hash = V8CodeCache::hash_source(&patched);
                        let cached_bytecode = cache.read(&fetch_url, source_hash);
                        SourceCodeCacheInfo {
                            hash: source_hash,
                            data: cached_bytecode.map(|b| Cow::Owned(b)),
                        }
                    });
                    Ok(ModuleSource::new(
                        ModuleType::JavaScript,
                        ModuleSourceCode::String(patched.into()),
                        &spec,
                        cache_info,
                    ))
                }
                Ok(Err(e)) => {
                    eprintln!("[NEORENDER:LOADER] fetch error: {} — {}", short, e);
                    store.borrow_mut().failed_urls.insert(fetch_url);
                    Ok(ModuleSource::new(
                        ModuleType::JavaScript,
                        ModuleSourceCode::String(format!("/* fetch error: {} */", e).to_string().into()),
                        &spec,
                        None,
                    ))
                }
                Err(_) => {
                    eprintln!("[NEORENDER:LOADER] fetch timeout: {}", short);
                    store.borrow_mut().failed_urls.insert(fetch_url);
                    Ok(ModuleSource::new(
                        ModuleType::JavaScript,
                        ModuleSourceCode::String("/* fetch timeout */".to_string().into()),
                        &spec,
                        None,
                    ))
                }
            }
        }))
    }

    fn code_cache_ready(
        &self,
        specifier: ModuleSpecifier,
        hash: u64,
        code_cache: &[u8],
    ) -> Pin<Box<dyn Future<Output = ()>>> {
        // Write compiled bytecode to disk for future sessions
        if let Some(cache) = &self.code_cache {
            let url = specifier.to_string();
            cache.write(&url, hash, code_cache);
        }
        async {}.boxed_local()
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
    let loader = NeoModuleLoader {
        store: store.clone(),
        code_cache: V8CodeCache::new().map(Rc::new),
    };

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

    // 2b. Crypto — full SubtleCrypto (digest, HMAC, importKey, etc.)
    let crypto_js: String = include_str!("../../js/crypto.js").to_string();
    runtime.execute_script("<neorender:crypto>", crypto_js)
        .map_err(|e| format!("Crypto load error: {e}"))?;

    // 3. Bootstrap — parses HTML via linkedom, sets up fetch/timers/globals
    let boot_js: String = include_str!("../../js/bootstrap.js").to_string();
    runtime.execute_script("<neorender:bootstrap>", boot_js)
        .map_err(|e| format!("Bootstrap error: {e}"))?;

    // 3b. Request interceptor — wraps fetch to log all network requests
    let intercept_js: String = include_str!("../../js/intercept.js").to_string();
    runtime.execute_script("<neorender:intercept>", intercept_js)
        .map_err(|e| format!("Intercept load error: {e}"))?;

    // 3c0. Noise removal — strip chat widgets, ad containers, popups BEFORE WOM extraction
    let noise_js: String = include_str!("../../js/noise.js").to_string();
    runtime.execute_script("<neorender:noise>", noise_js)
        .map_err(|e| format!("Noise filter load error: {e}"))?;

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

    // 3e1. Delta updates — only send what changed after interactions (__neo_take_snapshot, __neo_get_delta)
    let delta_js: String = include_str!("../../js/delta.js").to_string();
    runtime.execute_script("<neorender:delta>", delta_js)
        .map_err(|e| format!("Delta load error: {e}"))?;

    // 3e2. Smart prefetch — predict what the AI will want next (__neo_prefetch_hints)
    let prefetch_js: String = include_str!("../../js/prefetch.js").to_string();
    runtime.execute_script("<neorender:prefetch>", prefetch_js)
        .map_err(|e| format!("Prefetch load error: {e}"))?;

    // 3f. Browser bridge — event listeners + interaction API
    let browser_js: String = include_str!("../../js/browser.js").to_string();
    runtime.execute_script("<neorender:browser>", browser_js)
        .map_err(|e| format!("Browser bridge load error: {e}"))?;

    // 3f1. History API + Navigation events — pushState/replaceState/popstate for SPAs
    let history_js: String = include_str!("../../js/history.js").to_string();
    runtime.execute_script("<neorender:history>", history_js)
        .map_err(|e| format!("History API load error: {e}"))?;

    // 3f1b. Dynamic scripts — intercepts appendChild/insertBefore for <script> tags
    let dynscript_js: String = include_str!("../../js/dynamic_scripts.js").to_string();
    runtime.execute_script("<neorender:dynamic_scripts>", dynscript_js)
        .map_err(|e| format!("Dynamic scripts load error: {e}"))?;

    // 3f2. Iframes — intercepts iframe creation, fetches + parses nested documents
    let iframe_js: String = include_str!("../../js/iframe.js").to_string();
    runtime.execute_script("<neorender:iframe>", iframe_js)
        .map_err(|e| format!("Iframe load error: {e}"))?;

    // 3f3. Custom Elements (Web Components) — registry polyfill for GitHub, Twitch, etc.
    let custom_elements_js: String = include_str!("../../js/custom_elements.js").to_string();
    runtime.execute_script("<neorender:custom_elements>", custom_elements_js)
        .map_err(|e| format!("Custom Elements load error: {e}"))?;

    // 3f4. WebSocket — functional stub so sites don't break on WebSocket checks
    let websocket_js: String = include_str!("../../js/websocket.js").to_string();
    runtime.execute_script("<neorender:websocket>", websocket_js)
        .map_err(|e| format!("WebSocket load error: {e}"))?;

    // 3f5. EventSource (SSE) — fetches text/event-stream and parses SSE events
    let eventsource_js: String = include_str!("../../js/eventsource.js").to_string();
    runtime.execute_script("<neorender:eventsource>", eventsource_js)
        .map_err(|e| format!("EventSource load error: {e}"))?;

    // 3f6. Consent auto-accept — dismisses cookie dialogs after navigation
    let consent_js: String = include_str!("../../js/consent.js").to_string();
    runtime.execute_script("<neorender:consent>", consent_js)
        .map_err(|e| format!("Consent load error: {e}"))?;

    // 3g. Stealth patches (navigator.webdriver, plugins, screen)
    let stealth_js: String = include_str!("../../js/stealth.js").to_string();
    runtime.execute_script("<neorender:stealth>", stealth_js)
        .map_err(|e| format!("Stealth load error: {e}"))?;

    // 3g2. Web APIs — common APIs that sites check for (Permissions, Clipboard, matchMedia, etc.)
    let webapis_js: String = include_str!("../../js/webapis.js").to_string();
    runtime.execute_script("<neorender:webapis>", webapis_js)
        .map_err(|e| format!("WebAPIs load error: {e}"))?;

    // 3g3. Layout stubs — realistic fake dimensions for fingerprint checks (Turnstile, etc.)
    let layout_js: String = include_str!("../../js/layout.js").to_string();
    runtime.execute_script("<neorender:layout>", layout_js)
        .map_err(|e| format!("Layout load error: {e}"))?;

    // 3h. Auto-extraction (tables, articles, forms, structured data)
    let extract_js: String = include_str!("../../js/extract.js").to_string();
    runtime.execute_script("<neorender:extract>", extract_js)
        .map_err(|e| format!("Extract load error: {e}"))?;

    // 3h1. Semantic compression — prioritized text blocks for AI
    let compress_js: String = include_str!("../../js/compress.js").to_string();
    runtime.execute_script("<neorender:compress>", compress_js)
        .map_err(|e| format!("Compress load error: {e}"))?;

    // 3h2. Page classification — auto-detect page type
    let classify_js: String = include_str!("../../js/classify.js").to_string();
    runtime.execute_script("<neorender:classify>", classify_js)
        .map_err(|e| format!("Classify load error: {e}"))?;

    // 3i. Wait-for-condition helpers
    let wait_js: String = include_str!("../../js/wait.js").to_string();
    runtime.execute_script("<neorender:wait>", wait_js)
        .map_err(|e| format!("Wait load error: {e}"))?;

    // 3j. ChatGPT Sentinel — Turnstile VM + PoW solver
    let sentinel_js: String = include_str!("../../js/sentinel.js").to_string();
    runtime.execute_script("<neorender:sentinel>", sentinel_js)
        .map_err(|e| format!("Sentinel load error: {e}"))?;

    // 3k. IndexedDB — in-memory stub (prevents SPA crashes)
    let indexeddb_js: String = include_str!("../../js/indexeddb.js").to_string();
    runtime.execute_script("<neorender:indexeddb>", indexeddb_js)
        .map_err(|e| format!("IndexedDB load error: {e}"))?;

    // 3l. Shadow DOM — attachShadow polyfill
    let shadow_dom_js: String = include_str!("../../js/shadow_dom.js").to_string();
    runtime.execute_script("<neorender:shadow_dom>", shadow_dom_js)
        .map_err(|e| format!("Shadow DOM load error: {e}"))?;

    // 3m. Cache API — in-memory stub (Service Worker caching)
    let cache_api_js: String = include_str!("../../js/cache_api.js").to_string();
    runtime.execute_script("<neorender:cache_api>", cache_api_js)
        .map_err(|e| format!("Cache API load error: {e}"))?;

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

    let mod_id = match tokio::time::timeout(
        std::time::Duration::from_secs(10),
        runtime.load_main_es_module(&specifier),
    ).await {
        Ok(Ok(id)) => id,
        Ok(Err(e)) => return Some(format!("[{}] Module load: {}", name, first_line(&e.to_string()))),
        Err(_) => return Some(format!("[{}] Module load TIMEOUT (10s)", name)),
    };

    let eval_result = runtime.mod_evaluate(mod_id);

    // Run event loop to resolve imports and execute.
    // Errors from React/SSR hydration (e.g. stream .then() on null) are non-fatal.
    match tokio::time::timeout(
        std::time::Duration::from_secs(5),
        runtime.run_event_loop(PollEventLoopOptions::default()),
    ).await {
        Ok(Err(e)) => eprintln!("[NEORENDER] Module event loop error (non-fatal): {e}"),
        Err(_) => eprintln!("[NEORENDER] Module event loop timeout (5s)"),
        Ok(Ok(())) => {}
    }

    match tokio::time::timeout(std::time::Duration::from_secs(5), eval_result).await {
        Ok(Ok(())) => {
            eprintln!("[NEORENDER] Module eval OK: {name}");
            // Run event loop again — TLA dependencies may still be resolving
            tokio::time::timeout(
                std::time::Duration::from_secs(2),
                runtime.run_event_loop(PollEventLoopOptions::default()),
            ).await.ok();
            None
        },
        Ok(Err(e)) => {
            let msg = format!("[{}] {}", name, first_line(&e.to_string()));
            eprintln!("[NEORENDER] Module eval error: {msg}");
            Some(msg)
        }
        Err(_) => {
            eprintln!("[NEORENDER] Module eval TIMEOUT (15s): {name} — top-level await unresolved");
            None
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

    let mod_id = match tokio::time::timeout(
        std::time::Duration::from_secs(5),
        runtime.load_side_es_module(&specifier),
    ).await {
        Ok(Ok(id)) => id,
        Ok(Err(e)) => return Some(format!("[{}] Side module load: {}", name, first_line(&e.to_string()))),
        Err(_) => return Some(format!("[{}] Side module load TIMEOUT (5s)", name)),
    };

    let eval_result = runtime.mod_evaluate(mod_id);

    // Run event loop to resolve imports and execute.
    // Errors here are often from React internals (Suspense, lazy) that recover gracefully.
    // We log but don't fail — the module is still usable for imports.
    match tokio::time::timeout(
        std::time::Duration::from_secs(2),
        runtime.run_event_loop(PollEventLoopOptions::default()),
    ).await {
        Ok(Err(e)) => eprintln!("[NEORENDER] Side module event loop error (non-fatal): {e}"),
        Err(_) => eprintln!("[NEORENDER] Side module event loop timeout (2s)"),
        Ok(Ok(())) => {}
    }

    match tokio::time::timeout(std::time::Duration::from_secs(2), eval_result).await {
        Ok(Ok(())) => { eprintln!("[NEORENDER] Side module eval OK: {name}"); None },
        Ok(Err(e)) => {
            let msg = format!("[{}] {}", name, first_line(&e.to_string()));
            eprintln!("[NEORENDER] Side module eval error: {msg}");
            Some(msg)
        }
        Err(_) => {
            eprintln!("[NEORENDER] Side module eval TIMEOUT (2s): {name} — TLA unresolved");
            None
        }
    }
}

// extract_export_names and generate_stub_module are defined above (near ScriptStore)
// with regex support for minified JS + Proxy-based stubs for deep property access.

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
