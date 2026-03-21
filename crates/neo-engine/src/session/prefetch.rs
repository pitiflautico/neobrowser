//! Module pre-fetch — depth-2 ES import crawl with disk cache.
//!
//! After extracting scripts from HTML, this module pre-fetches all
//! ES module imports (and their imports) so V8 never blocks on network.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use neo_http::{HttpClient, HttpRequest, RequestContext, RequestKind};
use neo_runtime::modules::extract_es_imports;
use neo_runtime::JsRuntime;
use neo_trace::Tracer;

use super::scripts::ScriptInfo;

/// Pre-fetch budget and limits.
const TOTAL_BUDGET: Duration = Duration::from_secs(3);
const PER_MODULE_TIMEOUT_MS: u64 = 1_500;
const MODULE_CAP: usize = 100;

/// Result of a pre-fetch pass.
#[allow(dead_code)]
pub(crate) struct PrefetchResult {
    /// Total modules inserted into the store.
    pub modules_fetched: usize,
    /// Modules loaded from disk cache.
    pub cache_hits: usize,
    /// Time spent pre-fetching.
    pub duration_ms: u64,
}

/// Pre-fetch ES module imports up to depth 2.
///
/// 1. Seed from module scripts and modulepreload links already in the store.
/// 2. Extract their imports, fetch missing ones (disk cache first, then HTTP).
/// 3. Repeat for depth 1 (imports of imports).
pub(crate) fn prefetch_modules(
    scripts: &[ScriptInfo],
    page_url: &str,
    rt: &mut dyn JsRuntime,
    http: &dyn HttpClient,
    tracer: &dyn Tracer,
    trace_id: &str,
) -> PrefetchResult {
    let start = Instant::now();
    let mut visited: HashSet<String> = HashSet::new();
    let mut cache_hits = 0usize;
    let mut total_fetched = 0usize;

    // Seed: collect (url, source) for modules/preloads already in the store.
    let mut to_scan: Vec<(String, String)> = Vec::new();
    for script in scripts {
        if !script.is_module() {
            continue;
        }
        if let Some(url) = script.url() {
            visited.insert(url.to_string());
            if let Some(source) = rt.get_module_source(url) {
                to_scan.push((url.to_string(), source));
            }
        }
    }
    // Also scan inline modules for imports.
    for script in scripts {
        if let ScriptInfo::Inline {
            content, is_module, ..
        } = script
        {
            if *is_module {
                to_scan.push((page_url.to_string(), content.clone()));
            }
        }
    }

    // Depth 0 and 1 (two iterations).
    for depth in 0..2 {
        if start.elapsed() >= TOTAL_BUDGET || to_scan.is_empty() {
            break;
        }

        let mut urls_to_fetch: Vec<String> = Vec::new();
        for (script_url, content) in &to_scan {
            let imports = extract_es_imports(content, script_url);
            for import_url in imports {
                if visited.contains(&import_url) {
                    continue;
                }
                if rt.has_module(&import_url) {
                    visited.insert(import_url);
                    continue;
                }
                if !urls_to_fetch.contains(&import_url) {
                    urls_to_fetch.push(import_url);
                }
            }
        }

        if urls_to_fetch.is_empty() {
            break;
        }

        // Cap to MODULE_CAP.
        if urls_to_fetch.len() > MODULE_CAP {
            urls_to_fetch.truncate(MODULE_CAP);
        }

        let mut next_round: Vec<(String, String)> = Vec::new();

        for url in urls_to_fetch {
            if start.elapsed() >= TOTAL_BUDGET {
                tracer.module_event(&url, "prefetch_budget_exhausted", trace_id);
                break;
            }
            visited.insert(url.clone());

            // 1. Check disk cache.
            if let Some(cached) = read_module_cache(&url) {
                tracer.module_event(&url, "cache_hit", trace_id);
                rt.insert_module(&url, &cached);
                cache_hits += 1;
                total_fetched += 1;
                next_round.push((url, cached));
                continue;
            }

            // 2. HTTP fetch.
            tracer.module_event(&url, "cache_miss", trace_id);
            match fetch_module(http, &url, page_url) {
                Ok(source) => {
                    tracer.module_event(&url, "prefetch_hit", trace_id);
                    write_module_cache(&url, &source);
                    rt.insert_module(&url, &source);
                    total_fetched += 1;
                    if depth < 1 {
                        next_round.push((url, source));
                    }
                }
                Err(_) => {
                    tracer.module_event(&url, "prefetch_miss", trace_id);
                }
            }
        }

        to_scan = next_round;
    }

    PrefetchResult {
        modules_fetched: total_fetched,
        cache_hits,
        duration_ms: start.elapsed().as_millis() as u64,
    }
}

/// Fetch a single module via HTTP.
fn fetch_module(
    http: &dyn HttpClient,
    url: &str,
    page_url: &str,
) -> Result<String, neo_http::HttpError> {
    let req = HttpRequest {
        method: "GET".to_string(),
        url: url.to_string(),
        headers: std::collections::HashMap::new(),
        body: None,
        context: RequestContext {
            kind: RequestKind::Subresource,
            initiator: "prefetch".to_string(),
            referrer: Some(page_url.to_string()),
            frame_id: None,
            top_level_url: Some(page_url.to_string()),
        },
        timeout_ms: PER_MODULE_TIMEOUT_MS,
    };
    let resp = http.request(&req)?;
    // Skip non-JS responses (HTML error pages).
    if resp.body.trim_start().starts_with('<') {
        return Err(neo_http::HttpError::Skipped {
            url: url.to_string(),
        });
    }
    Ok(resp.body)
}

// ─── Disk cache ───

/// Compute cache path: `~/.neorender/cache/modules/{hash}.js`.
fn module_cache_path(url: &str) -> Option<std::path::PathBuf> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    url.hash(&mut hasher);
    let home = dirs::home_dir()?;
    Some(
        home.join(".neorender")
            .join("cache")
            .join("modules")
            .join(format!("{:016x}.js", hasher.finish())),
    )
}

/// Read module from disk cache.
fn read_module_cache(url: &str) -> Option<String> {
    let path = module_cache_path(url)?;
    std::fs::read_to_string(path).ok()
}

/// Write module to disk cache (best-effort).
fn write_module_cache(url: &str, source: &str) {
    if let Some(path) = module_cache_path(url) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::write(path, source).ok();
    }
}
