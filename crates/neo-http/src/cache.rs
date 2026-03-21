//! Disk-based HTTP cache with ETag/Last-Modified revalidation.

use crate::{CacheDecision, HttpCache, HttpError, HttpRequest};
use neo_types::HttpResponse;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

/// File-based HTTP response cache.
///
/// Stores responses as JSON files keyed by hash of method+URL.
/// Supports Cache-Control max-age freshness and ETag/Last-Modified revalidation.
/// Default directory: `~/.neorender/cache/http/`.
#[derive(Debug)]
pub struct DiskCache {
    dir: PathBuf,
}

/// Metadata stored alongside a cached response.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct CacheEntry {
    response: HttpResponse,
    etag: Option<String>,
    last_modified: Option<String>,
    max_age: Option<u64>,
    stored_at: u64,
}

impl DiskCache {
    /// Create a disk cache at the given directory.
    pub fn new(dir: &str) -> Result<Self, HttpError> {
        let path = PathBuf::from(dir);
        std::fs::create_dir_all(&path).map_err(|e| HttpError::Cache(e.to_string()))?;
        Ok(Self { dir: path })
    }

    /// Create a cache at the default location (`~/.neorender/cache/http/`).
    pub fn default_cache() -> Result<Self, HttpError> {
        let home = std::env::var("HOME").map_err(|_| HttpError::Cache("HOME not set".into()))?;
        Self::new(&format!("{home}/.neorender/cache/http"))
    }

    /// Compute the cache file path for a request.
    fn entry_path(&self, method: &str, url: &str) -> PathBuf {
        let key = cache_key(method, url);
        self.dir.join(format!("{key}.json"))
    }

    /// Read a cache entry from disk.
    fn read_entry(&self, method: &str, url: &str) -> Option<CacheEntry> {
        let path = self.entry_path(method, url);
        let data = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&data).ok()
    }

    /// Write a cache entry to disk.
    fn write_entry(&self, method: &str, url: &str, entry: &CacheEntry) -> Result<(), HttpError> {
        let path = self.entry_path(method, url);
        let json = serde_json::to_string(entry).map_err(|e| HttpError::Cache(e.to_string()))?;
        std::fs::write(&path, json).map_err(|e| HttpError::Cache(e.to_string()))
    }
}

/// Generate a hex hash key from method + URL.
fn cache_key(method: &str, url: &str) -> String {
    let mut hasher = DefaultHasher::new();
    method.hash(&mut hasher);
    url.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Current unix timestamp in seconds.
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Parse Cache-Control max-age from response headers.
fn parse_max_age(headers: &std::collections::HashMap<String, String>) -> Option<u64> {
    let cc = headers.get("cache-control")?;
    for directive in cc.split(',') {
        let d = directive.trim();
        if let Some(val) = d.strip_prefix("max-age=") {
            return val.trim().parse().ok();
        }
    }
    None
}

impl HttpCache for DiskCache {
    /// Look up a cached response.
    ///
    /// Returns `Fresh` if within max-age, `Stale` if expired but has
    /// revalidation headers, or `Miss` if not cached.
    fn lookup(&self, req: &HttpRequest) -> CacheDecision {
        let entry = match self.read_entry(&req.method, &req.url) {
            Some(e) => e,
            None => return CacheDecision::Miss,
        };
        let age = now_secs().saturating_sub(entry.stored_at);
        let fresh = entry.max_age.is_some_and(|ma| age < ma);
        if fresh {
            CacheDecision::Fresh(entry.response)
        } else {
            CacheDecision::Stale {
                response: entry.response,
                etag: entry.etag,
                last_modified: entry.last_modified,
            }
        }
    }

    /// Store a response in the cache.
    fn store(&self, req: &HttpRequest, response: &HttpResponse) {
        let entry = CacheEntry {
            etag: response.headers.get("etag").cloned(),
            last_modified: response.headers.get("last-modified").cloned(),
            max_age: parse_max_age(&response.headers),
            stored_at: now_secs(),
            response: response.clone(),
        };
        let _ = self.write_entry(&req.method, &req.url, &entry);
    }

    /// Invalidate cache entries whose URL contains the pattern.
    fn invalidate(&self, pattern: &str) {
        if let Ok(entries) = std::fs::read_dir(&self.dir) {
            for entry in entries.flatten() {
                if let Ok(data) = std::fs::read_to_string(entry.path()) {
                    if data.contains(pattern) {
                        let _ = std::fs::remove_file(entry.path());
                    }
                }
            }
        }
    }

    /// Check if a cached GET response for the URL is still fresh.
    fn is_fresh(&self, url: &str) -> bool {
        let entry = match self.read_entry("GET", url) {
            Some(e) => e,
            None => return false,
        };
        let age = now_secs().saturating_sub(entry.stored_at);
        entry.max_age.is_some_and(|ma| age < ma)
    }
}
