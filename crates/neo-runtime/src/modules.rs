//! ES module loader — serves scripts from in-memory store with page origin fallback.

use deno_core::{
    ModuleLoadOptions, ModuleLoadResponse, ModuleSource, ModuleSourceCode, ModuleSpecifier,
    ModuleType, ResolutionKind, SourceCodeCacheInfo,
};
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use crate::code_cache::V8CodeCache;
use crate::neo_trace;
use crate::trace_events::{ModulePhase, TraceBuffer};
use neo_http::{HttpClient, HttpRequest, RequestContext, RequestKind};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

// Re-export import extraction from the dedicated module.
pub use crate::imports::extract_es_imports;

/// Tracks the number of in-flight module loads (requested but not yet fully loaded).
///
/// Shared between NeoModuleLoader and the settle loop so Rust can accurately
/// know when dynamic imports are still in progress — even before JS gets a
/// chance to update `__pendingModules`.
#[derive(Clone, Default)]
pub struct ModuleTracker {
    inner: Arc<ModuleTrackerInner>,
}

#[derive(Default)]
struct ModuleTrackerInner {
    /// Modules currently being fetched / evaluated.
    pending: AtomicUsize,
    /// Total modules requested (lifetime counter, never decremented).
    total_requested: AtomicUsize,
    /// Total modules that completed successfully.
    total_loaded: AtomicUsize,
    /// Total modules that failed to load.
    total_failed: AtomicUsize,
}

impl ModuleTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark a module as requested (about to fetch/load).
    pub fn on_requested(&self, url: &str) {
        self.inner.pending.fetch_add(1, Ordering::SeqCst);
        self.inner.total_requested.fetch_add(1, Ordering::SeqCst);
        neo_trace!("[MODULE-TRACK] requested: {url} (pending={})", self.pending());
    }

    /// Mark a module as successfully loaded.
    pub fn on_loaded(&self, url: &str) {
        let prev = self.inner.pending.fetch_sub(1, Ordering::SeqCst);
        self.inner.total_loaded.fetch_add(1, Ordering::SeqCst);
        neo_trace!("[MODULE-TRACK] loaded: {url} (pending={})", prev.saturating_sub(1));
    }

    /// Mark a module as failed.
    pub fn on_failed(&self, url: &str) {
        let prev = self.inner.pending.fetch_sub(1, Ordering::SeqCst);
        self.inner.total_failed.fetch_add(1, Ordering::SeqCst);
        neo_trace!("[MODULE-TRACK] failed: {url} (pending={})", prev.saturating_sub(1));
    }

    /// Number of modules currently in-flight.
    pub fn pending(&self) -> usize {
        self.inner.pending.load(Ordering::SeqCst)
    }

    /// Total modules requested since creation.
    pub fn total_requested(&self) -> usize {
        self.inner.total_requested.load(Ordering::SeqCst)
    }

    /// Total modules loaded successfully.
    pub fn total_loaded(&self) -> usize {
        self.inner.total_loaded.load(Ordering::SeqCst)
    }

    /// Total modules that failed.
    pub fn total_failed(&self) -> usize {
        self.inner.total_failed.load(Ordering::SeqCst)
    }

    /// Reset all counters (between page loads).
    pub fn reset(&self) {
        self.inner.pending.store(0, Ordering::SeqCst);
        self.inner.total_requested.store(0, Ordering::SeqCst);
        self.inner.total_loaded.store(0, Ordering::SeqCst);
        self.inner.total_failed.store(0, Ordering::SeqCst);
    }
}

/// Maximum number of on-demand module fetches per page load.
/// Raised from 50 to 200 for modern SPAs with 100+ ES modules
/// (e.g. Vite-based apps with fine-grained code splitting).
const ON_DEMAND_FETCH_BUDGET: usize = 200;

/// Pre-fetched script contents keyed by URL.
#[derive(Default)]
pub struct ScriptStore {
    pub scripts: HashMap<String, String>,
    pub failed_urls: HashSet<String>,
    pub stub_modules: HashSet<String>,
}

pub type ScriptStoreHandle = Rc<RefCell<ScriptStore>>;
pub type PageOriginHandle = Rc<RefCell<String>>;

/// Import map parsed from `<script type="importmap">`.
///
/// Supports the "imports" field (bare specifier → URL mapping).
/// Scopes are not yet supported.
#[derive(Default, Clone, Debug)]
pub struct ImportMap {
    /// Bare specifier → resolved URL.
    pub imports: HashMap<String, String>,
}

impl ImportMap {
    /// Parse an import map from JSON text.
    ///
    /// Returns `None` if the JSON is invalid or doesn't contain "imports".
    pub fn parse(json: &str) -> Option<Self> {
        let value: serde_json::Value = serde_json::from_str(json).ok()?;
        let imports_obj = value.get("imports")?.as_object()?;
        let mut imports = HashMap::new();
        for (key, val) in imports_obj {
            if let Some(url) = val.as_str() {
                imports.insert(key.clone(), url.to_string());
            }
        }
        Some(Self { imports })
    }

    /// Resolve a specifier against the import map.
    ///
    /// Returns `None` if the specifier is not in the map.
    pub fn resolve(&self, specifier: &str) -> Option<&str> {
        // Exact match first.
        if let Some(url) = self.imports.get(specifier) {
            return Some(url);
        }
        // Prefix match: "lodash/" → "https://cdn.../lodash-es/"
        // The longest matching prefix wins.
        let mut best: Option<(&str, &str)> = None;
        for (prefix, target) in &self.imports {
            if prefix.ends_with('/') && specifier.starts_with(prefix.as_str()) {
                let is_longer = best.is_none_or(|(bp, _)| prefix.len() > bp.len());
                if is_longer {
                    best = Some((prefix.as_str(), target.as_str()));
                }
            }
        }
        if let Some((prefix, target)) = best {
            // Can't return a constructed string from &self, so only exact prefix.
            // For prefix matches, the caller must construct the full URL.
            // We only support exact matches in this simple implementation.
            let _ = (prefix, target);
        }
        None
    }
}

pub type ImportMapHandle = Rc<RefCell<Option<ImportMap>>>;

/// Module loader that serves pre-fetched scripts as ES modules.
pub struct NeoModuleLoader {
    pub store: ScriptStoreHandle,
    pub code_cache: Option<Rc<V8CodeCache>>,
    /// Page origin for resolving `/path` specifiers. Shared with DenoRuntime.
    pub page_origin: PageOriginHandle,
    /// Import map from `<script type="importmap">`. Shared with engine.
    pub import_map: ImportMapHandle,
    /// HTTP client for on-demand module fetching (dynamic imports not in store).
    pub http_client: Option<Arc<dyn HttpClient>>,
    /// Counter for on-demand fetches (budget: max ON_DEMAND_FETCH_BUDGET per page).
    pub on_demand_count: RefCell<usize>,
    /// Tracks module lifecycle (pending/loaded/failed) for quiescence detection.
    pub module_tracker: ModuleTracker,
    /// Structured trace buffer shared with OpState for unified event collection.
    pub trace_buffer: TraceBuffer,
}

impl NeoModuleLoader {
    /// Resolve an absolute path against the page origin.
    fn resolve_with_origin(&self, specifier: &str) -> Option<ModuleSpecifier> {
        let origin = self.page_origin.borrow();
        if origin.is_empty() {
            return None;
        }
        ModuleSpecifier::parse(&format!("{}{}", origin, specifier)).ok()
    }

    /// Build cache info for a module: hash source, look up cached bytecode.
    fn make_cache_info(&self, url: &str, source: &str) -> Option<SourceCodeCacheInfo> {
        let cache = self.code_cache.as_ref()?;
        let source_hash = V8CodeCache::hash_source(source);
        let cached = cache.read(url, source_hash);
        Some(SourceCodeCacheInfo {
            hash: source_hash,
            data: cached.map(Cow::Owned),
        })
    }

    /// Check if a specifier is a "bare" specifier (not a URL, not relative, not absolute path).
    fn is_bare_specifier(specifier: &str) -> bool {
        !specifier.starts_with('/')
            && !specifier.starts_with("./")
            && !specifier.starts_with("../")
            && !specifier.starts_with("http://")
            && !specifier.starts_with("https://")
            && !specifier.starts_with("file://")
    }
}

impl deno_core::ModuleLoader for NeoModuleLoader {
    /// R7d: Resolve with import map, page origin fallback for absolute paths.
    fn resolve(
        &self,
        specifier: &str,
        referrer: &str,
        _kind: ResolutionKind,
    ) -> Result<ModuleSpecifier, deno_error::JsErrorBox> {
        // 1. Check import map FIRST (bare specifiers like "react", "lodash/fp").
        if let Some(ref map) = *self.import_map.borrow() {
            if let Some(resolved) = map.resolve(specifier) {
                if let Ok(spec) = ModuleSpecifier::parse(resolved) {
                    neo_trace!("[MODULE] resolve {specifier} -> {resolved} (import-map)");
                    self.trace_buffer.module_event(specifier, ModulePhase::Resolve, Some(resolved));
                    return Ok(spec);
                }
            }
        }

        // 2. Absolute paths from non-http referrer (e.g. `<eval>`) -> page origin.
        if specifier.starts_with('/') && !referrer.starts_with("http") {
            if let Some(spec) = self.resolve_with_origin(specifier) {
                neo_trace!("[MODULE] resolve {specifier} -> {spec} (origin-fallback)");
                self.trace_buffer.module_event(specifier, ModulePhase::Resolve, Some(&spec.to_string()));
                return Ok(spec);
            }
        }

        // 3. Standard resolution (relative against referrer, full URLs as-is).
        if let Ok(spec) = deno_core::resolve_import(specifier, referrer) {
            neo_trace!("[MODULE] resolve {specifier} -> {spec}");
            self.trace_buffer.module_event(specifier, ModulePhase::Resolve, Some(&spec.to_string()));
            return Ok(spec);
        }

        // 4. Absolute paths fallback when standard resolve fails.
        if specifier.starts_with('/') {
            if let Some(spec) = self.resolve_with_origin(specifier) {
                neo_trace!("[MODULE] resolve {specifier} -> {spec} (origin-fallback)");
                self.trace_buffer.module_event(specifier, ModulePhase::Resolve, Some(&spec.to_string()));
                return Ok(spec);
            }
        }

        // 5. Bare specifier: check if pre-registered in store (e.g. Node.js compat stubs).
        if Self::is_bare_specifier(specifier) {
            let store = self.store.borrow();
            if store.scripts.contains_key(specifier) {
                // Resolve bare specifier to a synthetic URL that load() will find in the store.
                let synthetic = format!("neo:node/{specifier}");
                // Also register the synthetic URL in store so load() finds it.
                let source = store.scripts.get(specifier).cloned().unwrap_or_default();
                drop(store);
                self.store.borrow_mut().scripts.insert(synthetic.clone(), source);
                let spec = ModuleSpecifier::parse(&synthetic)
                    .map_err(|e| deno_error::JsErrorBox::from_err(e))?;
                neo_trace!("[MODULE] resolve bare '{specifier}' -> {synthetic} (node-compat stub)");
                self.trace_buffer.module_event(specifier, ModulePhase::Resolve, Some(&synthetic));
                return Ok(spec);
            }
            let err_msg = format!(
                "bare specifier '{specifier}' not found in import map (referrer: '{referrer}'). \
                 Add a <script type=\"importmap\"> to map '{specifier}' to a URL."
            );
            self.trace_buffer.module_event(specifier, ModulePhase::Resolve, Some(&err_msg));
            return Err(deno_error::JsErrorBox::generic(err_msg));
        }

        let err_msg = format!("cannot resolve '{specifier}' from '{referrer}'");
        self.trace_buffer.module_event(specifier, ModulePhase::Resolve, Some(&err_msg));
        Err(deno_error::JsErrorBox::generic(err_msg))
    }

    fn load(
        &self,
        module_specifier: &ModuleSpecifier,
        _maybe_referrer: Option<&deno_core::ModuleLoadReferrer>,
        options: ModuleLoadOptions,
    ) -> ModuleLoadResponse {
        let is_dyn_import = options.is_dynamic_import;
        let url = module_specifier.to_string();
        let store = self.store.borrow();
        let import_kind = if is_dyn_import { "dynamic" } else { "static" };

        // Track: module requested
        self.module_tracker.on_requested(&url);

        // Check pre-fetched store first.
        if let Some(code) = store.scripts.get(&url) {
            let size_kb = code.len() / 1024;
            // R4: Stub heavy modules with no-op re-exports.
            if store.stub_modules.contains(&url) {
                neo_trace!("[MODULE] load {url} -> stubbed ({size_kb}KB, {import_kind})");
                self.trace_buffer.module_event(&url, ModulePhase::Load, Some("stubbed"));
                let exports = extract_export_names(code);
                let stub = generate_stub_module(&exports);
                self.module_tracker.on_loaded(&url);
                return ModuleLoadResponse::Sync(Ok(ModuleSource::new(
                    ModuleType::JavaScript,
                    ModuleSourceCode::String(stub.into()),
                    module_specifier,
                    None,
                )));
            }

            neo_trace!("[MODULE] load {url} -> ok ({size_kb}KB, {import_kind})");
            self.trace_buffer.module_event(&url, ModulePhase::Load, Some(&format!("{size_kb}KB")));
            // Source transforms for browser compat:
            let patched = inject_buffer_concat_fix(code);
            let patched = rewrite_promise_all_settled(&patched);
            let patched = safe_getall_transform(&patched);
            let patched = ensure_promise_finally(&patched);
            let patched = inject_editor_view_capture(&patched);
            let cache_info = self.make_cache_info(&url, &patched);
            self.module_tracker.on_loaded(&url);
            return ModuleLoadResponse::Sync(Ok(ModuleSource::new(
                ModuleType::JavaScript,
                ModuleSourceCode::String(patched.into()),
                module_specifier,
                cache_info,
            )));
        }

        // Skip known failures.
        if store.failed_urls.contains(&url) {
            neo_trace!("[MODULE] load {url} -> failed (known-failure, {import_kind})");
            self.trace_buffer.module_event(&url, ModulePhase::Load, Some("known-failure"));
            self.module_tracker.on_failed(&url);
            return empty_module(module_specifier);
        }

        // Skip non-JS URLs — but allow CDN URLs that serve JS without .js extension
        // (e.g. esm.sh/react@18, cdn.skypack.dev/react, jspm.dev/react).
        if !url.contains(".js") && !url.contains(".mjs") && !is_esm_cdn_url(&url) {
            neo_trace!("[MODULE] load {url} -> empty (non-js, {import_kind})");
            self.trace_buffer.module_event(&url, ModulePhase::Load, Some("non-js, empty"));
            self.module_tracker.on_failed(&url);
            return empty_module(module_specifier);
        }

        // Not in store — try on-demand fetch for HTTP(S) JS URLs.
        // Fetch for BOTH static and dynamic imports — ESM modules trigger static
        // imports for their dependencies which also need on-demand fetching.
        if let Some(ref http_client) = self.http_client {
            if url.starts_with("http://") || url.starts_with("https://")
            {
                let count = *self.on_demand_count.borrow();
                if count >= ON_DEMAND_FETCH_BUDGET {
                    neo_trace!(
                        "[MODULE] load {url} -> empty (on-demand budget exhausted: {count}, {import_kind})"
                    );
                    self.trace_buffer.module_event(&url, ModulePhase::Load, Some("budget-exhausted"));
                    self.module_tracker.on_failed(&url);
                    return empty_module(module_specifier);
                }

                let http = http_client.clone();
                let url_clone = url.clone();

                neo_trace!("[MODULE] load {url} -> fetching on-demand ({import_kind})");

                let fetched = std::thread::spawn(move || {
                    let req = HttpRequest {
                        method: "GET".to_string(),
                        url: url_clone,
                        headers: std::collections::HashMap::new(),
                        body: None,
                        context: RequestContext {
                            kind: RequestKind::Subresource,
                            initiator: "dynamic-import".to_string(),
                            referrer: None,
                            frame_id: None,
                            top_level_url: None,
                        },
                        timeout_ms: 5000,
                    };
                    http.request(&req)
                })
                .join();

                *self.on_demand_count.borrow_mut() += 1;

                match fetched {
                    Ok(Ok(resp)) if resp.status < 400 => {
                        let code = resp.body;
                        let size_kb = code.len() / 1024;
                        neo_trace!(
                            "[MODULE] fetched on-demand {url} ({size_kb}KB, {import_kind})"
                        );
                        self.trace_buffer.module_event(&url, ModulePhase::Load, Some(&format!("on-demand {size_kb}KB")));

                        // Store for future use (other modules may import the same chunk).
                        drop(store); // release immutable borrow
                        self.store
                            .borrow_mut()
                            .scripts
                            .insert(url.clone(), code.clone());

                        // Source transforms for browser compat:
                        // 1. Promise.allSettled polyfill
                        let patched = rewrite_promise_all_settled(&code);
                        let patched = safe_getall_transform(&patched);
                        let patched = ensure_promise_finally(&patched);
                        let patched = inject_editor_view_capture(&patched);
                        let cache_info = self.make_cache_info(&url, &patched);
                        self.module_tracker.on_loaded(&url);
                        return ModuleLoadResponse::Sync(Ok(ModuleSource::new(
                            ModuleType::JavaScript,
                            ModuleSourceCode::String(patched.into()),
                            module_specifier,
                            cache_info,
                        )));
                    }
                    Ok(Ok(resp)) => {
                        neo_trace!(
                            "[MODULE] fetch failed {url} (status {}, {import_kind})",
                            resp.status
                        );
                        self.trace_buffer.module_event(&url, ModulePhase::Load, Some(&format!("fetch-failed status={}", resp.status)));
                        drop(store);
                        self.store.borrow_mut().failed_urls.insert(url.clone());
                        self.module_tracker.on_failed(&url);
                    }
                    Ok(Err(e)) => {
                        neo_trace!("[MODULE] fetch error {url}: {e} ({import_kind})");
                        self.trace_buffer.module_event(&url, ModulePhase::Load, Some(&format!("fetch-error: {e}")));
                        drop(store);
                        self.store.borrow_mut().failed_urls.insert(url.clone());
                        self.module_tracker.on_failed(&url);
                    }
                    Err(_) => {
                        neo_trace!("[MODULE] fetch thread panicked {url} ({import_kind})");
                        self.trace_buffer.module_event(&url, ModulePhase::Load, Some("fetch-thread-panic"));
                        drop(store);
                        self.store.borrow_mut().failed_urls.insert(url.clone());
                        self.module_tracker.on_failed(&url);
                    }
                }

                return empty_module(module_specifier);
            }
        }

        // Not in store and not fetchable — return empty placeholder.
        neo_trace!("[MODULE] load {url} -> empty (not-in-store, {import_kind})");
        self.trace_buffer.module_event(&url, ModulePhase::Load, Some("not-in-store, empty"));
        self.module_tracker.on_failed(&url);
        empty_module(module_specifier)
    }

    fn code_cache_ready(
        &self,
        specifier: ModuleSpecifier,
        hash: u64,
        code_cache: &[u8],
    ) -> Pin<Box<dyn Future<Output = ()>>> {
        if let Some(cache) = &self.code_cache {
            let url = specifier.to_string();
            cache.write(&url, hash, code_cache);
        }
        Box::pin(async {})
    }
}

/// Check if a URL points to a known ESM CDN that serves JS without .js/.mjs extension.
///
/// These CDNs return `application/javascript` for URLs like `esm.sh/react@18`,
/// `cdn.skypack.dev/react`, `jspm.dev/react`, etc.
fn is_esm_cdn_url(url: &str) -> bool {
    const ESM_CDN_HOSTS: &[&str] = &[
        "esm.sh",
        "esm.run",
        "cdn.skypack.dev",
        "jspm.dev",
        "cdn.jsdelivr.net/npm/",
        "unpkg.com",
        "ga.jspm.io",
    ];
    ESM_CDN_HOSTS.iter().any(|host| url.contains(host))
}

/// Return an empty JS module (comment-only).
fn empty_module(spec: &ModuleSpecifier) -> ModuleLoadResponse {
    ModuleLoadResponse::Sync(Ok(ModuleSource::new(
        ModuleType::JavaScript,
        ModuleSourceCode::String("/* not found */".to_string().into()),
        spec,
        None,
    )))
}

// ─── Source transforms for browser compat ───
//
// These functions patch module source code before V8 evaluation to work around
// missing APIs in deno_core 0.311 and defensive coding patterns in frameworks.

/// Inject Buffer.concat polyfill for Vite/Rollup bundles.
///
/// Vite bundles multiple copies of the `buffer` npm shim. Some use forward references
/// where `.concat` is called before it's defined in the bundle. We prepend a universal
/// fix that patches any Buffer-like function constructor missing `concat`.
fn inject_buffer_concat_fix(code: &str) -> String {
    // Only apply to large modules that contain Buffer patterns (isBuffer is the universal marker)
    if code.len() < 50_000 || !code.contains(".isBuffer") {
        return code.to_string();
    }

    // Prepend a fix that intercepts any Buffer-like constructor and adds concat if missing.
    // Uses a Proxy on globalThis to catch variable assignments would be too invasive.
    // Instead: override Function.prototype to auto-patch Buffer-like constructors.
    // Simplest: just prepend a self-executing block that sets up a MutationObserver-like
    // interceptor via Object.defineProperty on common Buffer identifiers.
    //
    // Actually, the simplest approach that works: prepend a concat implementation that
    // any Buffer$N variable can reference. We define it as a globalThis helper.
    let fix = r#"(function(){var __bc=globalThis.Buffer&&globalThis.Buffer.concat?globalThis.Buffer.concat:function(list,tl){if(!list||!list.length)return new Uint8Array(0);if(!tl){tl=0;for(var i=0;i<list.length;i++)tl+=(list[i].length||list[i].byteLength||0)}var r=new Uint8Array(tl),o=0;for(var i=0;i<list.length;i++){var b=list[i];if(b instanceof Uint8Array||b instanceof ArrayBuffer){var a=b instanceof ArrayBuffer?new Uint8Array(b):b;r.set(a,o);o+=a.length}}return r};var __ba=function(s){return new Uint8Array(s)};var __bl=function(s){return new TextEncoder().encode(s).length};var __origCreate=Object.create;Object.create=function(p,d){var o=__origCreate.call(Object,p,d);return o};var __pp=Function.prototype;var __origCall=__pp.call;})();
"#;

    // The REAL fix: for EVERY `.isBuffer=` occurrence, inject `.concat=` (with ||
    // guard) right after the semicolon. This ensures concat is available immediately
    // after each isBuffer assignment, even if the same identifier appears multiple
    // times or concat is called before a later definition in the file.
    //
    // We process the source left-to-right, collecting (insert_position, identifier)
    // pairs, then inject in reverse order so earlier insert positions stay valid.
    let mut injections: Vec<(usize, String)> = Vec::new();
    let mut i = 0;

    while i < code.len().saturating_sub(10) {
        if let Some(pos) = code[i..].find(".isBuffer=") {
            let abs_pos = i + pos;
            // Extract identifier before .isBuffer=
            let before = &code[..abs_pos];
            let ident_start = before
                .rfind(|c: char| !c.is_alphanumeric() && c != '_' && c != '$')
                .map(|p| p + 1)
                .unwrap_or(0);
            let ident = &code[ident_start..abs_pos];

            if !ident.is_empty() && ident != "constructor" && ident != "prototype" {
                // Find the semicolon that ends this statement
                if let Some(semi) = code[abs_pos..].find(';') {
                    let insert_pos = abs_pos + semi + 1;
                    injections.push((insert_pos, ident.to_string()));
                }
            }
            i = abs_pos + 10;
        } else {
            break;
        }
    }

    if injections.is_empty() {
        return code.to_string();
    }

    let mut result = code.to_string();
    // Inject in reverse order so byte offsets remain valid
    for (insert_pos, ident) in injections.iter().rev() {
        let fix = format!(
            "{id}.concat={id}.concat||function(list,tl){{if(!list||!list.length)return new Uint8Array(0);if(!tl){{tl=0;for(var i=0;i<list.length;i++)tl+=(list[i].length||list[i].byteLength||0)}}var r=new Uint8Array(tl),o=0;for(var i=0;i<list.length;i++){{var b=list[i];if(b instanceof Uint8Array||b instanceof ArrayBuffer){{var a=b instanceof ArrayBuffer?new Uint8Array(b):b;r.set(a,o);o+=a.length}}}}return r}};{id}.allocUnsafe={id}.allocUnsafe||function(s){{return new Uint8Array(s)}};{id}.byteLength={id}.byteLength||function(s){{return new TextEncoder().encode(s).length}};",
            id = ident
        );
        result.insert_str(*insert_pos, &fix);
        neo_trace!("[MODULE-TRANSFORM] injected {}.concat fix at byte {}", ident, insert_pos);
    }

    neo_trace!("[MODULE-TRANSFORM] patched {} .isBuffer= sites", injections.len());
    result
}

/// Make `.getAll()` calls safe for undefined/null receivers.
///
/// Replaces `.getAll(` with `?.getAll?.(` (optional chaining) and patches
/// the React Router pattern where `.getAll` results feed into `.flatMap(o=>o.split(...))`
/// by inserting `.filter(Boolean)` to remove nullish values.
fn safe_getall_transform(code: &str) -> String {
    if !code.contains(".getAll(") {
        return code.to_string();
    }
    // 1. Optional chain: .getAll( → ?.getAll?.(
    let patched = code.replace(".getAll(", "?.getAll?.(");
    // 2. Patch the specific React Router pattern:
    //    .flatMap(o=>e?.getAll?.(o)).flatMap(o=>o.split(","))
    //    The second flatMap receives undefined elements. Add .filter(Boolean) before it.
    //    Target: `).flatMap(o=>o.split` → `).filter(Boolean).flatMap(o=>o.split`
    let patched = patched.replace(").flatMap(o=>o.split", ").filter(Boolean).flatMap(o=>o.split");
    // 3. Also handle: `).flatMap(o=>o.trim` (similar pattern in same code)
    patched.replace(").flatMap(o=>o.trim", ").filter(Boolean).flatMap(o=>o.trim")
}

/// Ensure Promise.prototype.finally exists before module code runs.
/// deno_core 0.311 V8 lacks .finally, and page code can delete our polyfill.
/// Injected at the top of each module so it's always available.
fn ensure_promise_finally(code: &str) -> String {
    if !code.contains(".finally") && !code.contains("finally(") {
        return code.to_string();
    }
    let polyfill = "if(!Promise.prototype.finally)Promise.prototype.finally=function(f){return this.then(function(v){return Promise.resolve(f()).then(function(){return v})},function(r){return Promise.resolve(f()).then(function(){throw r})})};";
    format!("{polyfill}{code}")
}

/// Inject EditorView capture into ProseMirror code.
///
/// Patches `this.domObserver=` (which follows docView init in EditorView constructor)
/// to also store `this` globally for our editor bridge.
fn inject_editor_view_capture(code: &str) -> String {
    if !code.contains("this.domObserver=") || !code.contains("pmViewDesc") {
        return code.to_string();
    }
    // `this.domObserver=` only appears once in EditorView constructor.
    // Prepend our capture: `globalThis.__neo_pmView=this,this.domObserver=`
    code.replacen(
        "this.domObserver=",
        "globalThis.__neo_pmView=this,this.domObserver=",
        1, // only first occurrence
    )
}

/// Rewrite `Promise.allSettled(` calls with an inline polyfill.
///
/// deno_core 0.311 lacks `Promise.allSettled`. We rewrite call sites directly
/// because module scope doesn't support global polyfill injection.
pub fn rewrite_promise_all_settled(code: &str) -> String {
    if !code.contains("Promise.allSettled(") {
        return code.to_string();
    }
    code.replace(
        "Promise.allSettled(",
        "((ps)=>Promise.all([...ps].map(p=>Promise.resolve(p)\
         .then(v=>({status:'fulfilled',value:v}),\
         r=>({status:'rejected',reason:r})))))(",
    )
}

/// Extract named export identifiers from JS module source.
pub fn extract_export_names(js: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut seen = HashSet::new();

    for line in js.split("export") {
        let trimmed = line.trim_start();
        if trimmed.starts_with('{') {
            if let Some(end) = trimmed.find('}') {
                let block = &trimmed[1..end];
                for item in block.split(',') {
                    let item = item.trim();
                    let exported = if let Some(pos) = item.rfind(" as ") {
                        item[pos + 4..].trim()
                    } else {
                        item
                    };
                    let clean = exported.trim();
                    if !clean.is_empty() && clean != "default" && seen.insert(clean.to_string()) {
                        names.push(clean.to_string());
                    }
                }
            }
        }
        for kw in &["function ", "const ", "let ", "var ", "class "] {
            if let Some(rest) = trimmed.strip_prefix(kw) {
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
                    .collect();
                if !name.is_empty() && seen.insert(name.clone()) {
                    names.push(name);
                }
            }
        }
        if (trimmed.starts_with("default") || trimmed.starts_with(" default"))
            && seen.insert("default".to_string())
        {
            names.push("default".to_string());
        }
    }
    names
}

/// Generate a stub ES module with no-op exports (Proxy-based).
pub fn generate_stub_module(export_names: &[String]) -> String {
    let mut parts = Vec::new();
    parts.push(
        "const _n=()=>_n;_n.then=undefined;\
         const _o=new Proxy({},{get:(t,p)=>p==='then'?undefined:_n});"
            .to_string(),
    );

    let mut items = Vec::new();
    for name in export_names {
        if name == "default" {
            continue;
        }
        parts.push(format!("const {}=_o;", name));
        items.push(name.clone());
    }

    if !items.is_empty() {
        parts.push(format!("export{{{}}};", items.join(",")));
    }
    parts.push("export default _o;".to_string());
    parts.join("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_export_names() {
        let js = r#"export function foo() {} export const bar = 1; export default 42;"#;
        let names = extract_export_names(js);
        assert!(names.contains(&"foo".to_string()));
        assert!(names.contains(&"bar".to_string()));
        assert!(names.contains(&"default".to_string()));
    }

    #[test]
    fn test_rewrite_promise_all_settled() {
        let code = "Promise.allSettled([p1, p2])";
        let result = rewrite_promise_all_settled(code);
        assert!(!result.contains("Promise.allSettled("));
        assert!(result.contains("Promise.all"));
    }

    #[test]
    fn test_generate_stub_module() {
        let names = vec!["foo".to_string(), "default".to_string()];
        let stub = generate_stub_module(&names);
        assert!(stub.contains("const foo=_o;"));
        assert!(stub.contains("export default _o;"));
    }

    fn make_loader(origin: &str) -> NeoModuleLoader {
        NeoModuleLoader {
            store: Rc::new(RefCell::new(ScriptStore::default())),
            code_cache: None,
            page_origin: Rc::new(RefCell::new(origin.to_string())),
            import_map: Rc::new(RefCell::new(None)),
            http_client: None,
            on_demand_count: RefCell::new(0),
            module_tracker: ModuleTracker::new(),
            trace_buffer: TraceBuffer::new(),
        }
    }

    fn make_loader_with_import_map(origin: &str, map: ImportMap) -> NeoModuleLoader {
        NeoModuleLoader {
            store: Rc::new(RefCell::new(ScriptStore::default())),
            code_cache: None,
            page_origin: Rc::new(RefCell::new(origin.to_string())),
            import_map: Rc::new(RefCell::new(Some(map))),
            http_client: None,
            on_demand_count: RefCell::new(0),
            module_tracker: ModuleTracker::new(),
            trace_buffer: TraceBuffer::new(),
        }
    }

    #[test]
    fn test_resolve_absolute_path_with_origin() {
        use deno_core::ModuleLoader;
        let loader = make_loader("https://example.com");
        let r = loader.resolve("/cdn/bundle.js", "file:///<eval>", ResolutionKind::Import);
        assert_eq!(r.unwrap().to_string(), "https://example.com/cdn/bundle.js");
    }

    #[test]
    fn test_resolve_bare_specifier_fails() {
        use deno_core::ModuleLoader;
        let loader = make_loader("");
        let err = loader
            .resolve("react", "file:///<eval>", ResolutionKind::Import)
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("bare specifier"), "error should mention bare specifier: {msg}");
        assert!(msg.contains("import map"), "error should mention import map: {msg}");
    }

    #[test]
    fn test_import_map_resolution() {
        use deno_core::ModuleLoader;
        let map = ImportMap::parse(
            r#"{"imports": {"react": "https://esm.sh/react@18", "lodash": "https://esm.sh/lodash"}}"#,
        )
        .expect("valid import map");
        let loader = make_loader_with_import_map("https://example.com", map);
        let r = loader.resolve("react", "https://example.com/app.js", ResolutionKind::Import);
        assert_eq!(r.unwrap().to_string(), "https://esm.sh/react@18");
    }

    #[test]
    fn test_bare_specifier_without_map_errors() {
        use deno_core::ModuleLoader;
        let loader = make_loader("https://example.com");
        let result = loader.resolve("vue", "https://example.com/app.js", ResolutionKind::Import);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("bare specifier"));
        assert!(msg.contains("import map"));
    }

    #[test]
    fn test_relative_module_resolution() {
        use deno_core::ModuleLoader;
        let loader = make_loader("https://example.com");
        let r = loader.resolve(
            "./foo.js",
            "https://example.com/app/main.js",
            ResolutionKind::Import,
        );
        assert_eq!(
            r.unwrap().to_string(),
            "https://example.com/app/foo.js"
        );
    }

    #[test]
    fn test_import_map_parse() {
        let json = r#"{"imports": {"react": "https://esm.sh/react@18", "@/utils": "https://example.com/utils.js"}}"#;
        let map = ImportMap::parse(json).unwrap();
        assert_eq!(map.imports.get("react").unwrap(), "https://esm.sh/react@18");
        assert_eq!(map.imports.len(), 2);
    }

    #[test]
    fn test_import_map_parse_invalid() {
        assert!(ImportMap::parse("not json").is_none());
        assert!(ImportMap::parse(r#"{"scopes": {}}"#).is_none());
    }

    #[test]
    fn test_esm_cdn_url_detection() {
        assert!(is_esm_cdn_url("https://esm.sh/react@18"));
        assert!(is_esm_cdn_url("https://esm.sh/react-dom@18/client"));
        assert!(is_esm_cdn_url("https://cdn.skypack.dev/react"));
        assert!(is_esm_cdn_url("https://jspm.dev/react"));
        assert!(is_esm_cdn_url("https://unpkg.com/react@18/umd/react.production.min.js"));
        assert!(is_esm_cdn_url("https://cdn.jsdelivr.net/npm/react@18"));
        assert!(!is_esm_cdn_url("https://example.com/app"));
        assert!(!is_esm_cdn_url("https://cdn.example.com/lib"));
    }

    // ─── ImportMap edge cases ───

    #[test]
    fn test_import_map_parse_empty_imports() {
        let map = ImportMap::parse(r#"{"imports": {}}"#).unwrap();
        assert!(map.imports.is_empty());
    }

    #[test]
    fn test_import_map_parse_no_imports_key() {
        assert!(ImportMap::parse(r#"{"scopes": {}}"#).is_none());
    }

    #[test]
    fn test_import_map_parse_imports_not_object() {
        assert!(ImportMap::parse(r#"{"imports": "string"}"#).is_none());
    }

    #[test]
    fn test_import_map_parse_non_string_values_ignored() {
        let map = ImportMap::parse(r#"{"imports": {"a": "https://a.com/a.js", "b": 42}}"#).unwrap();
        assert_eq!(map.imports.len(), 1);
        assert!(map.imports.contains_key("a"));
    }

    #[test]
    fn test_import_map_resolve_exact_match() {
        let map = ImportMap::parse(
            r#"{"imports": {"react": "https://esm.sh/react@18"}}"#,
        )
        .unwrap();
        assert_eq!(map.resolve("react"), Some("https://esm.sh/react@18"));
    }

    #[test]
    fn test_import_map_resolve_no_match() {
        let map = ImportMap::parse(
            r#"{"imports": {"react": "https://esm.sh/react@18"}}"#,
        )
        .unwrap();
        assert_eq!(map.resolve("vue"), None);
    }

    #[test]
    fn test_import_map_resolve_prefix_match_returns_none() {
        // Current implementation only supports exact matches, not prefix resolution
        // (prefix match is detected but not returned because it can't construct the URL)
        let map = ImportMap::parse(
            r#"{"imports": {"lodash/": "https://cdn.example.com/lodash-es/"}}"#,
        )
        .unwrap();
        // "lodash/fp" has prefix match with "lodash/" but resolve returns None
        // because the implementation can't construct the full URL
        assert_eq!(map.resolve("lodash/fp"), None);
    }

    #[test]
    fn test_import_map_resolve_at_scoped_packages() {
        let map = ImportMap::parse(
            r#"{"imports": {"@/utils": "https://example.com/src/utils.js"}}"#,
        )
        .unwrap();
        assert_eq!(
            map.resolve("@/utils"),
            Some("https://example.com/src/utils.js")
        );
    }

    // ─── Module resolution edge cases ───

    #[test]
    fn test_resolve_full_http_url() {
        use deno_core::ModuleLoader;
        let loader = make_loader("https://example.com");
        let r = loader.resolve(
            "https://cdn.example.com/lib.js",
            "https://example.com/app.js",
            ResolutionKind::Import,
        );
        assert_eq!(
            r.unwrap().to_string(),
            "https://cdn.example.com/lib.js"
        );
    }

    #[test]
    fn test_resolve_parent_relative() {
        use deno_core::ModuleLoader;
        let loader = make_loader("https://example.com");
        let r = loader.resolve(
            "../shared/util.js",
            "https://example.com/app/sub/main.js",
            ResolutionKind::Import,
        );
        assert_eq!(
            r.unwrap().to_string(),
            "https://example.com/app/shared/util.js"
        );
    }

    #[test]
    fn test_resolve_absolute_path_from_http_referrer() {
        use deno_core::ModuleLoader;
        let loader = make_loader("https://example.com");
        let r = loader.resolve(
            "/assets/vendor.js",
            "https://example.com/app/main.js",
            ResolutionKind::Import,
        );
        // When referrer starts with "http", standard resolution handles absolute paths
        assert!(r.is_ok());
        assert!(r.unwrap().to_string().contains("vendor.js"));
    }

    #[test]
    fn test_resolve_import_map_takes_priority() {
        use deno_core::ModuleLoader;
        let map = ImportMap::parse(
            r#"{"imports": {"react": "https://esm.sh/react@18", "react-dom": "https://esm.sh/react-dom@18"}}"#,
        )
        .unwrap();
        let loader = make_loader_with_import_map("https://example.com", map);
        let r = loader.resolve(
            "react",
            "https://example.com/app.js",
            ResolutionKind::Import,
        );
        assert_eq!(r.unwrap().to_string(), "https://esm.sh/react@18");
        let r2 = loader.resolve(
            "react-dom",
            "https://example.com/app.js",
            ResolutionKind::Import,
        );
        assert_eq!(r2.unwrap().to_string(), "https://esm.sh/react-dom@18");
    }

    #[test]
    fn test_bare_specifier_without_import_map_errors_with_helpful_message() {
        use deno_core::ModuleLoader;
        let loader = make_loader("https://example.com");
        let err = loader
            .resolve("some-package", "https://example.com/app.js", ResolutionKind::Import)
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("bare specifier"), "should say bare specifier: {msg}");
        assert!(msg.contains("import map"), "should mention import map: {msg}");
        assert!(msg.contains("some-package"), "should include the specifier: {msg}");
    }

    #[test]
    fn test_is_bare_specifier() {
        assert!(NeoModuleLoader::is_bare_specifier("react"));
        assert!(NeoModuleLoader::is_bare_specifier("lodash/fp"));
        assert!(NeoModuleLoader::is_bare_specifier("@scope/pkg"));
        assert!(!NeoModuleLoader::is_bare_specifier("./relative.js"));
        assert!(!NeoModuleLoader::is_bare_specifier("../parent.js"));
        assert!(!NeoModuleLoader::is_bare_specifier("/absolute.js"));
        assert!(!NeoModuleLoader::is_bare_specifier("https://cdn.com/lib.js"));
        assert!(!NeoModuleLoader::is_bare_specifier("http://cdn.com/lib.js"));
        assert!(!NeoModuleLoader::is_bare_specifier("file:///local.js"));
    }

    // ─── ModuleTracker ───

    #[test]
    fn test_module_tracker_lifecycle() {
        let tracker = ModuleTracker::new();
        assert_eq!(tracker.pending(), 0);
        assert_eq!(tracker.total_requested(), 0);

        tracker.on_requested("https://example.com/a.js");
        assert_eq!(tracker.pending(), 1);
        assert_eq!(tracker.total_requested(), 1);

        tracker.on_requested("https://example.com/b.js");
        assert_eq!(tracker.pending(), 2);
        assert_eq!(tracker.total_requested(), 2);

        tracker.on_loaded("https://example.com/a.js");
        assert_eq!(tracker.pending(), 1);
        assert_eq!(tracker.total_loaded(), 1);

        tracker.on_failed("https://example.com/b.js");
        assert_eq!(tracker.pending(), 0);
        assert_eq!(tracker.total_failed(), 1);
        assert_eq!(tracker.total_loaded(), 1);
    }

    #[test]
    fn test_module_tracker_reset() {
        let tracker = ModuleTracker::new();
        tracker.on_requested("a");
        tracker.on_loaded("a");
        tracker.on_requested("b");
        tracker.on_failed("b");

        tracker.reset();
        assert_eq!(tracker.pending(), 0);
        assert_eq!(tracker.total_requested(), 0);
        assert_eq!(tracker.total_loaded(), 0);
        assert_eq!(tracker.total_failed(), 0);
    }

    // ─── Source transforms ───

    #[test]
    fn test_safe_getall_transform_no_getall() {
        let code = "const x = 1;";
        assert_eq!(safe_getall_transform(code), code);
    }

    #[test]
    fn test_safe_getall_transform_adds_optional_chaining() {
        let code = "params.getAll('key')";
        let result = safe_getall_transform(code);
        assert!(result.contains("?.getAll?.("), "should add optional chaining: {result}");
    }

    #[test]
    fn test_safe_getall_transform_patches_flatmap_split() {
        // The transform only activates when .getAll( is present in the source
        let code = "params.getAll('key')).flatMap(o=>o.split(\",\"))";
        let result = safe_getall_transform(code);
        assert!(
            result.contains(".filter(Boolean).flatMap(o=>o.split"),
            "should insert filter(Boolean): {result}"
        );
    }

    #[test]
    fn test_ensure_promise_finally_no_finally() {
        let code = "Promise.resolve(42)";
        let result = ensure_promise_finally(code);
        assert_eq!(result, code, "no .finally means no polyfill injection");
    }

    #[test]
    fn test_ensure_promise_finally_injects_polyfill() {
        let code = "p.finally(() => cleanup())";
        let result = ensure_promise_finally(code);
        assert!(result.starts_with("if(!Promise.prototype.finally)"), "should inject polyfill");
        assert!(result.ends_with(code), "original code should follow polyfill");
    }

    #[test]
    fn test_inject_editor_view_capture_no_prosemirror() {
        let code = "class MyView {}";
        assert_eq!(inject_editor_view_capture(code), code);
    }

    #[test]
    fn test_inject_editor_view_capture_needs_both_markers() {
        // Only domObserver, no pmViewDesc -> no injection
        let code = "this.domObserver=new Observer()";
        assert_eq!(inject_editor_view_capture(code), code);
    }

    #[test]
    fn test_inject_editor_view_capture_with_both_markers() {
        let code = "function pmViewDesc(){}; this.domObserver=new Observer()";
        let result = inject_editor_view_capture(code);
        assert!(result.contains("globalThis.__neo_pmView=this,this.domObserver="));
    }

    // ─── extract_export_names edge cases ───

    #[test]
    fn test_extract_export_names_export_block() {
        let js = "export { foo, bar, baz }";
        let names = extract_export_names(js);
        assert!(names.contains(&"foo".to_string()));
        assert!(names.contains(&"bar".to_string()));
        assert!(names.contains(&"baz".to_string()));
    }

    #[test]
    fn test_extract_export_names_alias() {
        let js = "export { internal as publicName }";
        let names = extract_export_names(js);
        assert!(names.contains(&"publicName".to_string()));
    }

    #[test]
    fn test_extract_export_names_class() {
        let js = "export class MyComponent {}";
        let names = extract_export_names(js);
        assert!(names.contains(&"MyComponent".to_string()));
    }

    #[test]
    fn test_extract_export_names_let_var() {
        let js = "export let x = 1; export var y = 2;";
        let names = extract_export_names(js);
        assert!(names.contains(&"x".to_string()));
        assert!(names.contains(&"y".to_string()));
    }

    #[test]
    fn test_extract_export_names_no_duplicates() {
        let js = "export const foo = 1; export { foo }";
        let names = extract_export_names(js);
        let foo_count = names.iter().filter(|n| *n == "foo").count();
        assert_eq!(foo_count, 1, "should not have duplicate 'foo'");
    }

    #[test]
    fn test_extract_export_names_default_in_braces_excluded() {
        // When "default" appears inside export { }, it's explicitly excluded
        // from named exports (handled separately as a default export)
        let js = "export { default } from './dep.js'";
        let names = extract_export_names(js);
        // The { default } block skips "default" — it must come from "export default" syntax
        assert!(
            !names.contains(&"default".to_string()),
            "default inside braces is excluded from named exports: {names:?}"
        );
    }

    #[test]
    fn test_extract_export_names_default_keyword() {
        // "export default" produces "default" in the names list
        let js = "export default 42;";
        let names = extract_export_names(js);
        assert!(names.contains(&"default".to_string()));
    }

    #[test]
    fn test_extract_export_names_empty_source() {
        let names = extract_export_names("");
        assert!(names.is_empty());
    }

    // ─── generate_stub_module ───

    #[test]
    fn test_stub_module_no_exports() {
        let stub = generate_stub_module(&[]);
        assert!(stub.contains("export default _o;"));
        assert!(!stub.contains("export{"));
    }

    #[test]
    fn test_stub_module_with_named_exports() {
        let names = vec!["foo".to_string(), "bar".to_string()];
        let stub = generate_stub_module(&names);
        assert!(stub.contains("const foo=_o;"));
        assert!(stub.contains("const bar=_o;"));
        assert!(stub.contains("export{foo,bar}"));
        assert!(stub.contains("export default _o;"));
    }

    #[test]
    fn test_stub_module_default_only() {
        let names = vec!["default".to_string()];
        let stub = generate_stub_module(&names);
        assert!(stub.contains("export default _o;"));
        // "default" should NOT create a "const default=_o;" (that's a syntax error)
        assert!(!stub.contains("const default=_o;"));
    }

    // ─── rewrite_promise_all_settled edge cases ───

    #[test]
    fn test_rewrite_allsettled_multiple_occurrences() {
        let code = "Promise.allSettled([a]); Promise.allSettled([b])";
        let result = rewrite_promise_all_settled(code);
        assert!(!result.contains("Promise.allSettled("));
        // Both occurrences should be replaced
    }

    #[test]
    fn test_rewrite_allsettled_idempotent() {
        let code = "Promise.allSettled([p1])";
        let once = rewrite_promise_all_settled(code);
        let twice = rewrite_promise_all_settled(&once);
        assert_eq!(once, twice, "double rewrite must be idempotent");
    }

    // ─── ScriptStore ───

    #[test]
    fn test_script_store_default_empty() {
        let store = ScriptStore::default();
        assert!(store.scripts.is_empty());
        assert!(store.failed_urls.is_empty());
        assert!(store.stub_modules.is_empty());
    }

    // ─── inject_buffer_concat_fix ───

    /// Helper: pad code to exceed 50KB threshold
    fn pad_to_50k(code: &str) -> String {
        let padding_needed = 50_001usize.saturating_sub(code.len());
        let padding = " ".repeat(padding_needed);
        format!("{}{}", padding, code)
    }

    #[test]
    fn test_buffer_fix_skips_small_modules() {
        let code = "Buffer$1.isBuffer=function(){};";
        assert_eq!(code.len() < 50_000, true);
        let result = inject_buffer_concat_fix(code);
        assert_eq!(result, code, "small module should be returned unchanged");
    }

    #[test]
    fn test_buffer_fix_skips_no_isbuffer() {
        let code = pad_to_50k("var x = Buffer.concat([a, b]);");
        let result = inject_buffer_concat_fix(&code);
        assert_eq!(result, code, "module without .isBuffer should be unchanged");
    }

    #[test]
    fn test_buffer_fix_injects_concat_simple() {
        let code = pad_to_50k("Buffer$1.isBuffer=function(b){return b._isBuffer};");
        let result = inject_buffer_concat_fix(&code);
        assert!(result.contains("Buffer$1.concat="), "should inject .concat for Buffer$1");
        assert!(result.contains("Buffer$1.allocUnsafe="), "should inject .allocUnsafe");
        assert!(result.contains("Buffer$1.byteLength="), "should inject .byteLength");
    }

    #[test]
    fn test_buffer_fix_injects_after_semicolon() {
        let inner = "Buffer$1.isBuffer=function(b){return !!b};var x=1;";
        let code = pad_to_50k(inner);
        let result = inject_buffer_concat_fix(&code);
        // The concat fix should be inserted right after the semicolon that ends the isBuffer statement
        let is_buffer_end = result.find("Buffer$1.isBuffer=").unwrap();
        let semi_after = result[is_buffer_end..].find(';').unwrap();
        let after_semi = &result[is_buffer_end + semi_after + 1..];
        assert!(after_semi.starts_with("Buffer$1.concat="), "fix should be injected right after isBuffer semicolon");
    }

    #[test]
    fn test_buffer_fix_handles_dollar_ident() {
        let code = pad_to_50k("Buffer$1$2.isBuffer=function(){return false};");
        let result = inject_buffer_concat_fix(&code);
        assert!(result.contains("Buffer$1$2.concat="), "should handle $ in identifiers");
    }

    #[test]
    fn test_buffer_fix_handles_short_ident() {
        let code = pad_to_50k("xye.isBuffer=function(){};");
        let result = inject_buffer_concat_fix(&code);
        assert!(result.contains("xye.concat="), "should handle short identifiers like xye");
    }

    #[test]
    fn test_buffer_fix_handles_alphanumeric_ident() {
        let code = pad_to_50k("y2.isBuffer=function(){};A1e.isBuffer=function(){};");
        let result = inject_buffer_concat_fix(&code);
        assert!(result.contains("y2.concat="), "should handle identifier y2");
        assert!(result.contains("A1e.concat="), "should handle identifier A1e");
    }

    #[test]
    fn test_buffer_fix_skips_constructor() {
        let code = pad_to_50k("constructor.isBuffer=function(){};");
        let result = inject_buffer_concat_fix(&code);
        assert!(!result.contains("constructor.concat="), "should skip 'constructor' identifier");
    }

    #[test]
    fn test_buffer_fix_skips_prototype() {
        let code = pad_to_50k("prototype.isBuffer=function(){};");
        let result = inject_buffer_concat_fix(&code);
        assert!(!result.contains("prototype.concat="), "should skip 'prototype' identifier");
    }

    #[test]
    fn test_buffer_fix_multiple_identifiers() {
        let code = pad_to_50k(
            "Buffer$1.isBuffer=function(){};something;Buffer$2.isBuffer=function(){};"
        );
        let result = inject_buffer_concat_fix(&code);
        assert!(result.contains("Buffer$1.concat="), "should fix first identifier");
        assert!(result.contains("Buffer$2.concat="), "should fix second identifier");
    }

    #[test]
    fn test_buffer_fix_injects_at_every_occurrence() {
        let code = pad_to_50k(
            "Buf.isBuffer=1;other();Buf.isBuffer=2;"
        );
        let result = inject_buffer_concat_fix(&code);
        // Should inject at EVERY .isBuffer= site, because .concat may be
        // called between the two definitions during async execution.
        let count = result.matches("Buf.concat=").count();
        assert_eq!(count, 2, "should inject concat at every .isBuffer= site");
    }

    #[test]
    fn test_buffer_fix_exact_threshold_below() {
        // 49_999 bytes should NOT trigger (< 50_000 check)
        let base = "Buf.isBuffer=1;";
        let code = format!("{}{}", " ".repeat(49_999 - base.len()), base);
        assert_eq!(code.len(), 49_999);
        let result = inject_buffer_concat_fix(&code);
        assert_eq!(result, code, "49999 bytes should not trigger fix");
    }

    #[test]
    fn test_buffer_fix_exact_threshold_at() {
        // Exactly 50_000 bytes SHOULD trigger (>= 50_000)
        let base = "Buf.isBuffer=1;";
        let code = format!("{}{}", " ".repeat(50_000 - base.len()), base);
        assert_eq!(code.len(), 50_000);
        let result = inject_buffer_concat_fix(&code);
        assert!(result.contains("Buf.concat="), "50000 bytes should trigger fix");
    }

    #[test]
    fn test_buffer_fix_preserves_original_code() {
        let inner = "var z=1;Buffer$1.isBuffer=function(){return true};var w=2;";
        let code = pad_to_50k(inner);
        let result = inject_buffer_concat_fix(&code);
        assert!(result.contains("var z=1;"), "original code before should be preserved");
        assert!(result.contains("var w=2;"), "original code after should be preserved");
        assert!(result.contains("Buffer$1.isBuffer="), "isBuffer assignment should remain");
    }

    #[test]
    fn test_buffer_fix_concat_available_at_first_isbuffer() {
        // Simulates the Factorial HR scenario: Buffer shim defined twice,
        // but .concat() called between the two definitions during event loop.
        let inner = "B$4.isBuffer=function(){};useConcat(B$4.concat);B$4.isBuffer=function(){};B$4.concat=realConcat;";
        let code = pad_to_50k(inner);
        let result = inject_buffer_concat_fix(&code);
        // The fix must appear after the FIRST .isBuffer= so that useConcat()
        // finds .concat already defined.
        let first_isbuffer = result.find("B$4.isBuffer=").unwrap();
        let first_semi = result[first_isbuffer..].find(';').unwrap();
        let after_first = &result[first_isbuffer + first_semi + 1..];
        assert!(
            after_first.starts_with("B$4.concat=B$4.concat||"),
            "concat fix must be injected right after the first .isBuffer= semicolon"
        );
    }

    #[test]
    fn test_buffer_fix_or_guard_preserves_existing_concat() {
        // When concat already exists on the identifier, the ||
        // guard ensures we don't overwrite it.
        let inner = "X.isBuffer=1;X.concat=original;";
        let code = pad_to_50k(inner);
        let result = inject_buffer_concat_fix(&code);
        // The injected code uses X.concat=X.concat||function...
        // so if X.concat already exists, it won't be replaced.
        assert!(result.contains("X.concat=X.concat||"), "should use || guard");
        assert!(result.contains("X.concat=original;"), "original concat should still be in the source");
    }

    // ─── Bare specifier -> store resolution (node-compat stubs) ───

    fn make_loader_with_store_scripts(origin: &str, scripts: Vec<(&str, &str)>) -> NeoModuleLoader {
        let mut store = ScriptStore::default();
        for (key, source) in scripts {
            store.scripts.insert(key.to_string(), source.to_string());
        }
        NeoModuleLoader {
            store: Rc::new(RefCell::new(store)),
            code_cache: None,
            page_origin: Rc::new(RefCell::new(origin.to_string())),
            import_map: Rc::new(RefCell::new(None)),
            http_client: None,
            on_demand_count: RefCell::new(0),
            module_tracker: ModuleTracker::new(),
            trace_buffer: TraceBuffer::new(),
        }
    }

    #[test]
    fn test_bare_specifier_buffer_resolves_from_store() {
        use deno_core::ModuleLoader;
        let loader = make_loader_with_store_scripts(
            "https://example.com",
            vec![("buffer", "export const Buffer = {};\n")],
        );
        let r = loader.resolve("buffer", "https://example.com/app.js", ResolutionKind::Import);
        assert_eq!(r.unwrap().to_string(), "neo:node/buffer");
    }

    #[test]
    fn test_bare_specifier_process_resolves_from_store() {
        use deno_core::ModuleLoader;
        let loader = make_loader_with_store_scripts(
            "https://example.com",
            vec![("process", "export default { env: {} };\n")],
        );
        let r = loader.resolve("process", "https://example.com/app.js", ResolutionKind::Import);
        assert_eq!(r.unwrap().to_string(), "neo:node/process");
    }

    #[test]
    fn test_bare_specifier_not_in_store_returns_error() {
        use deno_core::ModuleLoader;
        let loader = make_loader_with_store_scripts("https://example.com", vec![]);
        let err = loader
            .resolve("react", "https://example.com/app.js", ResolutionKind::Import)
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("bare specifier"), "should mention bare specifier: {msg}");
        assert!(msg.contains("import map"), "should mention import map: {msg}");
    }

    #[test]
    fn test_full_url_resolves_normally_even_with_store() {
        use deno_core::ModuleLoader;
        let loader = make_loader_with_store_scripts(
            "https://example.com",
            vec![("buffer", "export const Buffer = {};\n")],
        );
        let r = loader.resolve(
            "https://cdn.example.com/foo.js",
            "https://example.com/app.js",
            ResolutionKind::Import,
        );
        assert_eq!(r.unwrap().to_string(), "https://cdn.example.com/foo.js");
    }

    #[test]
    fn test_relative_specifier_resolves_normally_even_with_store() {
        use deno_core::ModuleLoader;
        let loader = make_loader_with_store_scripts(
            "https://example.com",
            vec![("buffer", "export const Buffer = {};\n")],
        );
        let r = loader.resolve(
            "./foo.js",
            "https://example.com/app/main.js",
            ResolutionKind::Import,
        );
        assert_eq!(r.unwrap().to_string(), "https://example.com/app/foo.js");
    }

    #[test]
    fn test_bare_specifier_store_populates_synthetic_url() {
        use deno_core::ModuleLoader;
        let source = "export const Buffer = { isBuffer: () => false };\n";
        let loader = make_loader_with_store_scripts(
            "https://example.com",
            vec![("buffer", source)],
        );
        // After resolve, the synthetic URL should be registered in the store
        let _ = loader.resolve("buffer", "https://example.com/app.js", ResolutionKind::Import);
        let store = loader.store.borrow();
        assert_eq!(
            store.scripts.get("neo:node/buffer").map(|s| s.as_str()),
            Some(source),
            "synthetic URL should be registered in store with original source"
        );
    }
}
