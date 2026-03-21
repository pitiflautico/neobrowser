//! ES module loader — serves scripts from in-memory store.
//!
//! NeoModuleLoader resolves imports from pre-fetched scripts,
//! fetches missing modules on-demand, applies source transforms,
//! and integrates with V8 bytecode cache for fast repeat loads.

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

// Re-export import extraction from the dedicated module.
pub use crate::imports::extract_es_imports;

/// Pre-fetched script contents keyed by URL.
#[derive(Default)]
pub struct ScriptStore {
    /// URL -> JavaScript source code.
    pub scripts: HashMap<String, String>,
    /// URLs that returned non-JS content (HTML) — skip on repeat.
    pub failed_urls: HashSet<String>,
    /// URLs of heavy modules to stub instead of parsing.
    pub stub_modules: HashSet<String>,
}

/// Shared handle to the script store.
pub type ScriptStoreHandle = Rc<RefCell<ScriptStore>>;

/// Module loader that serves pre-fetched scripts as ES modules.
pub struct NeoModuleLoader {
    /// Shared script store.
    pub store: ScriptStoreHandle,
    /// Optional V8 bytecode cache for compiled code.
    pub code_cache: Option<Rc<V8CodeCache>>,
}

impl NeoModuleLoader {
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
}

impl deno_core::ModuleLoader for NeoModuleLoader {
    fn resolve(
        &self,
        specifier: &str,
        referrer: &str,
        _kind: ResolutionKind,
    ) -> Result<ModuleSpecifier, deno_core::error::AnyError> {
        deno_core::resolve_import(specifier, referrer)
            .map_err(|e| deno_core::error::generic_error(e.to_string()))
    }

    fn load(
        &self,
        module_specifier: &ModuleSpecifier,
        _maybe_referrer: Option<&ModuleSpecifier>,
        _is_dyn_import: bool,
        _requested_module_type: RequestedModuleType,
    ) -> ModuleLoadResponse {
        let url = module_specifier.to_string();
        let store = self.store.borrow();

        // Check pre-fetched store first.
        if let Some(code) = store.scripts.get(&url) {
            // R4: Stub heavy modules with no-op re-exports.
            if store.stub_modules.contains(&url) {
                let exports = extract_export_names(code);
                let stub = generate_stub_module(&exports);
                return ModuleLoadResponse::Sync(Ok(ModuleSource::new(
                    ModuleType::JavaScript,
                    ModuleSourceCode::String(stub.into()),
                    module_specifier,
                    None,
                )));
            }

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
            return empty_module(module_specifier);
        }

        // Skip non-JS URLs.
        if !url.contains(".js") && !url.contains(".mjs") {
            return empty_module(module_specifier);
        }

        // Not in store — return empty placeholder.
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
///
/// Handles: `export{a as b,c}`, `export function x`,
/// `export const x`, `export default`, and re-exports.
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

/// Generate a stub ES module with no-op exports.
///
/// Property access on any export returns a no-op function
/// (handles chained calls like `telemetry.instance.addFirstTiming()`).
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
}
