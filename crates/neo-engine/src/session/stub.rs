//! Module stubbing — replace heavy non-essential modules with no-op skeletons.
//!
//! After pre-fetch, scans the store for modules that are:
//! - Larger than `stub_threshold_bytes` (default 1MB)
//! - NOT referenced in HTML (not in <script src>, <link rel=modulepreload>,
//!   or inline module imports)
//! - NOT direct imports of HTML-referenced modules (depth 1 protection)

use std::collections::HashSet;

use neo_runtime::modules::extract_es_imports;
use neo_runtime::JsRuntime;
use neo_trace::Tracer;

use super::scripts::ScriptInfo;

/// Result of the stubbing pass.
#[allow(dead_code)]
pub(crate) struct StubResult {
    /// Number of modules stubbed.
    pub stubbed: usize,
    /// Total bytes saved by stubbing.
    pub bytes_saved: usize,
}

/// Scan the module store and stub heavy non-essential modules.
pub(crate) fn stub_heavy_modules(
    scripts: &[ScriptInfo],
    page_url: &str,
    threshold: usize,
    rt: &mut dyn JsRuntime,
    tracer: &dyn Tracer,
    trace_id: &str,
) -> StubResult {
    if threshold == 0 {
        return StubResult {
            stubbed: 0,
            bytes_saved: 0,
        };
    }

    // Build set of essential URLs: HTML-declared scripts + preloads.
    let mut essential: HashSet<String> = HashSet::new();
    for script in scripts {
        if let Some(url) = script.url() {
            essential.insert(url.to_string());
        }
    }

    // Protect depth-1 imports of essential modules.
    let mut depth1_protected: Vec<String> = Vec::new();
    for url in &essential {
        if let Some(source) = rt.get_module_source(url) {
            for imp in extract_es_imports(&source, url) {
                depth1_protected.push(imp);
            }
        }
    }
    // Also protect imports from inline modules.
    for script in scripts {
        if let ScriptInfo::Inline {
            content,
            is_module: true,
        } = script
        {
            for imp in extract_es_imports(content, page_url) {
                depth1_protected.push(imp);
            }
        }
    }
    essential.extend(depth1_protected);

    // Find heavy, non-essential modules.
    let all_urls = rt.module_urls();
    let mut to_stub: Vec<(String, usize)> = Vec::new();

    for url in &all_urls {
        if essential.contains(url) {
            continue;
        }
        if let Some(source) = rt.get_module_source(url) {
            if source.len() >= threshold {
                to_stub.push((url.clone(), source.len()));
            }
        }
    }

    let mut bytes_saved = 0usize;
    for (url, size) in &to_stub {
        rt.mark_stub(url);
        tracer.module_event(url, "stubbed", trace_id);
        bytes_saved += size;
    }

    StubResult {
        stubbed: to_stub.len(),
        bytes_saved,
    }
}
