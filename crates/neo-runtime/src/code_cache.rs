//! V8 bytecode cache — stores compiled bytecode on disk for faster re-execution.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

/// Disk-backed V8 compiled bytecode cache.
///
/// Stores compiled bytecode keyed by URL hash.
/// File format: `[8 bytes source_hash LE] [V8 bytecode...]`
pub struct V8CodeCache {
    cache_dir: PathBuf,
}

impl V8CodeCache {
    /// Create a new code cache at the given directory.
    ///
    /// Creates the directory if it doesn't exist and logs the cache size.
    pub fn new(cache_dir: &PathBuf) -> Result<Self, std::io::Error> {
        std::fs::create_dir_all(cache_dir)?;
        let cache = Self {
            cache_dir: cache_dir.clone(),
        };
        cache.log_cache_size();
        Ok(cache)
    }

    /// Hash source code for cache invalidation.
    pub fn hash_source(code: &str) -> u64 {
        let mut hasher = DefaultHasher::new();
        code.hash(&mut hasher);
        hasher.finish()
    }

    /// Try to read cached bytecode. Returns None if missing or stale.
    ///
    /// If the source hash doesn't match (stale entry), the cache file
    /// is deleted to avoid repeated mismatches on future reads.
    pub fn read(&self, url: &str, source_hash: u64) -> Option<Vec<u8>> {
        let path = self.cache_path(url);
        let data = match std::fs::read(&path) {
            Ok(d) => d,
            Err(_) => return None,
        };
        if data.len() < 8 {
            let _ = std::fs::remove_file(&path);
            return None;
        }
        let stored = u64::from_le_bytes(match data[..8].try_into() {
            Ok(b) => b,
            Err(_) => return None,
        });
        if stored != source_hash {
            eprintln!(
                "[V8CACHE] Stale: {} (hash mismatch, invalidating)",
                short_name(url)
            );
            let _ = std::fs::remove_file(&path);
            return None;
        }
        let bytecode = data[8..].to_vec();
        eprintln!(
            "[V8CACHE] Hit: {} ({}B bytecode)",
            short_name(url),
            bytecode.len()
        );
        Some(bytecode)
    }

    /// Write bytecode to disk with source hash prefix.
    ///
    /// IO errors are logged but never fatal.
    pub fn write(&self, url: &str, source_hash: u64, bytecode: &[u8]) {
        let path = self.cache_path(url);
        let mut data = Vec::with_capacity(8 + bytecode.len());
        data.extend_from_slice(&source_hash.to_le_bytes());
        data.extend_from_slice(bytecode);
        match std::fs::write(&path, &data) {
            Ok(()) => eprintln!("[V8CACHE] Wrote: {} ({}B)", short_name(url), bytecode.len()),
            Err(e) => eprintln!("[V8CACHE] Write error for {}: {e}", short_name(url)),
        }
    }

    /// Deterministic filename from URL.
    fn cache_path(&self, url: &str) -> PathBuf {
        let mut hasher = DefaultHasher::new();
        url.hash(&mut hasher);
        self.cache_dir
            .join(format!("{:016x}.v8cache", hasher.finish()))
    }

    /// Log the total size of all cache files on startup.
    fn log_cache_size(&self) {
        let mut total_bytes: u64 = 0;
        let mut file_count: u64 = 0;
        if let Ok(entries) = std::fs::read_dir(&self.cache_dir) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata() {
                    if meta.is_file() {
                        total_bytes += meta.len();
                        file_count += 1;
                    }
                }
            }
        }
        let size_kb = total_bytes / 1024;
        eprintln!(
            "[V8CACHE] Dir: {} ({} files, {}KB)",
            self.cache_dir.display(),
            file_count,
            size_kb
        );
    }
}

/// Extract short filename from URL for log readability.
fn short_name(url: &str) -> &str {
    url.rsplit('/').next().unwrap_or(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_cache() -> (V8CodeCache, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let cache = V8CodeCache::new(&dir.path().to_path_buf()).unwrap();
        (cache, dir)
    }

    #[test]
    fn test_cache_write_read() {
        let (cache, _dir) = temp_cache();
        let url = "https://example.com/app.js";
        let source_hash = V8CodeCache::hash_source("const x = 1;");
        let bytecode = b"fake-v8-bytecode-data";

        cache.write(url, source_hash, bytecode);
        let result = cache.read(url, source_hash);

        assert_eq!(result, Some(bytecode.to_vec()));
    }

    #[test]
    fn test_cache_miss_wrong_hash() {
        let (cache, _dir) = temp_cache();
        let url = "https://example.com/app.js";
        let hash_a = V8CodeCache::hash_source("version A");
        let hash_b = V8CodeCache::hash_source("version B");
        let bytecode = b"compiled-for-version-a";

        cache.write(url, hash_a, bytecode);
        let result = cache.read(url, hash_b);

        assert_eq!(result, None);
        // Stale file should be deleted.
        assert!(cache.read(url, hash_a).is_none());
    }

    #[test]
    fn test_cache_miss_no_file() {
        let (cache, _dir) = temp_cache();
        let result = cache.read("https://example.com/nonexistent.js", 42);
        assert_eq!(result, None);
    }

    #[test]
    fn test_cache_rewrite_changes_hash() {
        let source = "Promise.allSettled([p1])";
        let rewritten = crate::modules::rewrite_promise_all_settled(source);

        let hash_original = V8CodeCache::hash_source(source);
        let hash_rewritten = V8CodeCache::hash_source(&rewritten);

        // Rewriting changes the hash — different cache entries.
        assert_ne!(hash_original, hash_rewritten);

        // Write bytecode for the rewritten version (what modules.rs does).
        let (cache, _dir) = temp_cache();
        let url = "https://example.com/app.js";
        cache.write(url, hash_rewritten, b"bytecode-rewritten");

        // Reading with the rewritten hash hits.
        assert_eq!(
            cache.read(url, hash_rewritten),
            Some(b"bytecode-rewritten".to_vec())
        );

        // Reading with the original (pre-rewrite) hash misses — stale.
        // This proves the cache key includes the rewritten source.
        cache.write(url, hash_rewritten, b"bytecode-rewritten");
        assert_eq!(cache.read(url, hash_original), None);
    }
}
