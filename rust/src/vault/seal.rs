//! Sealing and opening: AES-256-GCM, with a fresh nonce every time.
//!
//! Two properties are load-bearing. The nonce is random per seal and never reused, because
//! nonce reuse under GCM does not degrade the ciphertext, it leaks the keystream. And the TTL
//! is checked *before* decrypting, so an expired secret is never briefly in memory in plain
//! form on its way to being rejected.

use std::path::Path;

use super::keys::vault_key;
use super::{now_unix, VaultError, FORMAT};
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use serde_json::{json, Value};

/// A 96-bit nonce, unique per write.
///
/// AES-GCM catastrophically loses confidentiality if a nonce repeats under the same
/// key, so this mixes a per-process counter with the wall clock and a random seed
/// rather than using either alone: a counter repeats across restarts, and a
/// second-resolution clock repeats within a second.
pub(super) fn fresh_nonce() -> [u8; 12] {
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write(&now_unix().to_le_bytes());
    h.write(&COUNTER.fetch_add(1, Ordering::Relaxed).to_le_bytes());
    let a = h.finish().to_le_bytes();
    let mut h2 = std::collections::hash_map::RandomState::new().build_hasher();
    h2.write(&a);
    let b = h2.finish().to_le_bytes();
    let mut out = [0u8; 12];
    out[..8].copy_from_slice(&a);
    out[8..].copy_from_slice(&b[..4]);
    out
}

/// Encrypt `plaintext` and write it to `path`, with an optional lifetime.
///
/// The metadata (`domains`, `created_at`, `expires_at`) is stored in the clear
/// because it is the audit record: which domains received session material and when.
/// The PRD is explicit that this list is recorded and the cookie values never are.
pub fn seal(
    path: &Path,
    plaintext: &str,
    domains: &[String],
    ttl_secs: Option<u64>,
) -> Result<(), VaultError> {
    let key = vault_key()?;
    let cipher = Aes256Gcm::new(&key);
    let nonce_bytes = fresh_nonce();
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|_| VaultError::Crypto)?;

    let created = now_unix();
    let envelope = json!({
        "format": FORMAT,
        "created_at": created,
        "expires_at": ttl_secs.map(|t| created + t),
        // Audit trail: domains yes, values never.
        "domains": domains,
        "nonce": base64::engine::general_purpose::STANDARD.encode(nonce_bytes),
        "ciphertext": base64::engine::general_purpose::STANDARD.encode(&ct),
    });
    // Reuse the atomic 0600 writer: defence in depth, since the ciphertext being
    // unreadable is not a reason to let it be world-readable.
    crate::sessions::write_private(path, &envelope.to_string())?;
    Ok(())
}

/// Read and decrypt `path`, enforcing the expiry.
pub fn open(path: &Path) -> Result<String, VaultError> {
    let text = std::fs::read_to_string(path)?;
    let env: Value = serde_json::from_str(&text).map_err(|e| VaultError::Corrupt(e.to_string()))?;
    if env.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return Err(VaultError::Corrupt(format!(
            "unexpected format marker (want {FORMAT})"
        )));
    }
    // Expiry is checked BEFORE decrypting: an expired entry must not have its
    // plaintext materialised in memory at all, not even to be discarded.
    if let Some(exp) = env.get("expires_at").and_then(Value::as_u64) {
        let now = now_unix();
        if now >= exp {
            return Err(VaultError::Expired {
                expired_at: exp,
                now,
            });
        }
    }
    let b64 = |k: &str| -> Result<Vec<u8>, VaultError> {
        base64::engine::general_purpose::STANDARD
            .decode(
                env.get(k)
                    .and_then(Value::as_str)
                    .ok_or_else(|| VaultError::Corrupt(format!("missing {k}")))?,
            )
            .map_err(|e| VaultError::Corrupt(format!("{k}: {e}")))
    };
    let nonce_bytes = b64("nonce")?;
    let ct = b64("ciphertext")?;
    if nonce_bytes.len() != 12 {
        return Err(VaultError::Corrupt("nonce must be 12 bytes".into()));
    }

    let key = vault_key()?;
    let cipher = Aes256Gcm::new(&key);
    let pt = cipher
        .decrypt(Nonce::from_slice(&nonce_bytes), ct.as_ref())
        // A GCM tag mismatch means the file was tampered with or the key is wrong.
        // Both are refusals, never a best-effort partial read.
        .map_err(|_| VaultError::Crypto)?;
    String::from_utf8(pt).map_err(|e| VaultError::Corrupt(e.to_string()))
}

/// The metadata of a vault file, without decrypting it.
///
/// Lets `session_info` and the evidence bundle report coverage and freshness without
/// ever touching the session material.
pub fn inspect(path: &Path) -> Result<Value, VaultError> {
    let text = std::fs::read_to_string(path)?;
    let env: Value = serde_json::from_str(&text).map_err(|e| VaultError::Corrupt(e.to_string()))?;
    let expires_at = env.get("expires_at").and_then(Value::as_u64);
    Ok(json!({
        "format": env.get("format").and_then(Value::as_str).unwrap_or("?"),
        "created_at": env.get("created_at").and_then(Value::as_u64),
        "expires_at": expires_at,
        "expired": expires_at.map(|e| now_unix() >= e).unwrap_or(false),
        "domains": env.get("domains").cloned().unwrap_or(json!([])),
        "encrypted": true,
    }))
}

/// Destroy a vault entry so it cannot be recovered from the file's old contents.
///
/// Overwrite-then-remove, rather than a bare `remove_file`. On a journalling or
/// copy-on-write filesystem an overwrite is not a guaranteed shred — that limit is
/// stated in the docs rather than papered over — but it does defeat the realistic
/// case of the bytes still sitting in a file that unlink left allocated.
pub fn revoke(path: &Path) -> Result<(), VaultError> {
    if !path.exists() {
        return Ok(());
    }
    if let Ok(meta) = std::fs::metadata(path) {
        use std::io::Write;
        let len = meta.len() as usize;
        if let Ok(mut f) = std::fs::OpenOptions::new().write(true).open(path) {
            let zeros = vec![0u8; len.min(1024 * 1024)];
            let mut written = 0;
            while written < len {
                let n = zeros.len().min(len - written);
                if f.write_all(&zeros[..n]).is_err() {
                    break;
                }
                written += n;
            }
            let _ = f.sync_all();
        }
    }
    std::fs::remove_file(path)?;
    Ok(())
}

/// Prove an entry is gone. Used by tests and by anything that must report deletion
/// rather than assume it.
pub fn is_revoked(path: &Path) -> bool {
    !path.exists()
}
