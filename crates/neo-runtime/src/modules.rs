//! ES module loader — serves scripts from in-memory store with page origin fallback.

use deno_core::{
    ModuleLoadResponse, ModuleSource, ModuleSourceCode, ModuleSpecifier, ModuleType,
    RequestedModuleType, ResolutionKind, SourceCodeCacheInfo,
};
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use crate::code_cache::V8CodeCache;
use crate::neo_trace;
use neo_http::{HttpClient, HttpRequest, RequestContext, RequestKind};
use std::sync::Arc;

// Re-export import extraction from the dedicated module.
pub use crate::imports::extract_es_imports;

/// Maximum number of on-demand module fetches per page load.
const ON_DEMAND_FETCH_BUDGET: usize = 50;

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
    ) -> Result<ModuleSpecifier, deno_core::error::AnyError> {
        // 1. Check import map FIRST (bare specifiers like "react", "lodash/fp").
        if let Some(ref map) = *self.import_map.borrow() {
            if let Some(resolved) = map.resolve(specifier) {
                if let Ok(spec) = ModuleSpecifier::parse(resolved) {
                    neo_trace!("[MODULE] resolve {specifier} -> {resolved} (import-map)");
                    return Ok(spec);
                }
            }
        }

        // 2. Absolute paths from non-http referrer (e.g. `<eval>`) -> page origin.
        if specifier.starts_with('/') && !referrer.starts_with("http") {
            if let Some(spec) = self.resolve_with_origin(specifier) {
                neo_trace!("[MODULE] resolve {specifier} -> {spec} (origin-fallback)");
                return Ok(spec);
            }
        }

        // 3. Standard resolution (relative against referrer, full URLs as-is).
        if let Ok(spec) = deno_core::resolve_import(specifier, referrer) {
            neo_trace!("[MODULE] resolve {specifier} -> {spec}");
            return Ok(spec);
        }

        // 4. Absolute paths fallback when standard resolve fails.
        if specifier.starts_with('/') {
            if let Some(spec) = self.resolve_with_origin(specifier) {
                neo_trace!("[MODULE] resolve {specifier} -> {spec} (origin-fallback)");
                return Ok(spec);
            }
        }

        // 5. Bare specifier without import map -> clear error.
        if Self::is_bare_specifier(specifier) {
            return Err(deno_core::error::generic_error(format!(
                "bare specifier '{specifier}' not found in import map (referrer: '{referrer}'). \
                 Add a <script type=\"importmap\"> to map '{specifier}' to a URL."
            )));
        }

        Err(deno_core::error::generic_error(format!(
            "cannot resolve '{specifier}' from '{referrer}'"
        )))
    }

    fn load(
        &self,
        module_specifier: &ModuleSpecifier,
        _maybe_referrer: Option<&ModuleSpecifier>,
        is_dyn_import: bool,
        _requested_module_type: RequestedModuleType,
    ) -> ModuleLoadResponse {
        let url = module_specifier.to_string();
        let store = self.store.borrow();
        let import_kind = if is_dyn_import { "dynamic" } else { "static" };

        // Check pre-fetched store first.
        if let Some(code) = store.scripts.get(&url) {
            let size_kb = code.len() / 1024;
            // R4: Stub heavy modules with no-op re-exports.
            if store.stub_modules.contains(&url) {
                neo_trace!("[MODULE] load {url} -> stubbed ({size_kb}KB, {import_kind})");
                let exports = extract_export_names(code);
                let stub = generate_stub_module(&exports);
                return ModuleLoadResponse::Sync(Ok(ModuleSource::new(
                    ModuleType::JavaScript,
                    ModuleSourceCode::String(stub.into()),
                    module_specifier,
                    None,
                )));
            }

            neo_trace!("[MODULE] load {url} -> ok ({size_kb}KB, {import_kind})");
            // R5: Rewrite Promise.allSettled before serving.
            let patched = rewrite_promise_all_settled(code);
            let cache_info = self.make_cache_info(&url, &patched);
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
            return empty_module(module_specifier);
        }

        // Skip non-JS URLs — but allow CDN URLs that serve JS without .js extension
        // (e.g. esm.sh/react@18, cdn.skypack.dev/react, jspm.dev/react).
        if !url.contains(".js") && !url.contains(".mjs") && !is_esm_cdn_url(&url) {
            neo_trace!("[MODULE] load {url} -> empty (non-js, {import_kind})");
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

                        // Store for future use (other modules may import the same chunk).
                        drop(store); // release immutable borrow
                        self.store
                            .borrow_mut()
                            .scripts
                            .insert(url.clone(), code.clone());

                        let patched = rewrite_promise_all_settled(&code);
                        let cache_info = self.make_cache_info(&url, &patched);
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
                        drop(store);
                        self.store.borrow_mut().failed_urls.insert(url);
                    }
                    Ok(Err(e)) => {
                        neo_trace!("[MODULE] fetch error {url}: {e} ({import_kind})");
                        drop(store);
                        self.store.borrow_mut().failed_urls.insert(url);
                    }
                    Err(_) => {
                        neo_trace!("[MODULE] fetch thread panicked {url} ({import_kind})");
                        drop(store);
                        self.store.borrow_mut().failed_urls.insert(url);
                    }
                }

                return empty_module(module_specifier);
            }
        }

        // Not in store and not fetchable — return empty placeholder.
        neo_trace!("[MODULE] load {url} -> empty (not-in-store, {import_kind})");
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

/// Rewrite `Promise.allSettled(` calls with inline equivalent.
///
/// deno_core module scope doesn't support the polyfill injection pattern,
/// so we rewrite call sites directly.
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
}
