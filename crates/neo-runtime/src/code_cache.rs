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
    pub fn new(cache_dir: &PathBuf) -> Result<Self, std::io::Error> {
        std::fs::create_dir_all(cache_dir)?;
        Ok(Self {
            cache_dir: cache_dir.clone(),
        })
    }

    /// Hash source code for cache invalidation.
    pub fn hash_source(code: &str) -> u64 {
        let mut hasher = DefaultHasher::new();
        code.hash(&mut hasher);
        hasher.finish()
    }

    /// Try to read cached bytecode. Returns None if missing or stale.
    pub fn read(&self, url: &str, source_hash: u64) -> Option<Vec<u8>> {
        let path = self.cache_path(url);
        let data = std::fs::read(&path).ok()?;
        if data.len() < 8 {
            return None;
        }
        let stored = u64::from_le_bytes(data[..8].try_into().ok()?);
        if stored != source_hash {
            return None;
        }
        Some(data[8..].to_vec())
    }

    /// Write bytecode to disk with source hash prefix.
    pub fn write(&self, url: &str, source_hash: u64, bytecode: &[u8]) {
        let path = self.cache_path(url);
        let mut data = Vec::with_capacity(8 + bytecode.len());
        data.extend_from_slice(&source_hash.to_le_bytes());
        data.extend_from_slice(bytecode);
        let _ = std::fs::write(&path, &data);
    }

    /// Deterministic filename from URL.
    fn cache_path(&self, url: &str) -> PathBuf {
        let mut hasher = DefaultHasher::new();
        url.hash(&mut hasher);
        self.cache_dir
            .join(format!("{:016x}.v8cache", hasher.finish()))
    }
}
