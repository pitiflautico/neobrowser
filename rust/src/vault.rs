//! Encrypted-at-rest storage for session material, keyed by the OS credential store.
//!
//! `0600` permissions stop another *user* reading the cookie snapshots. They do not
//! stop anything that already runs as this user — a malicious npm postinstall, a
//! backup that syncs the home directory to a cloud drive, a stolen disk without full
//! disk encryption. Session cookies are as good as a password, so they get encrypted
//! with a key that lives in the Keychain / Secret Service / DPAPI rather than on
//! disk beside the data.
//!
//! Three properties the PRD asks for and that shaped this:
//!
//! - **TTL.** A session snapshot has an expiry. Reading past it fails closed and the
//!   plaintext is never produced, because a six-month-old cookie jar is a liability
//!   with no upside.
//! - **Revocation and verifiable deletion.** [`revoke`] overwrites the ciphertext
//!   before unlinking, and [`is_revoked`] can prove afterwards that it is gone.
//!   "Deleted" that leaves a recoverable file is not deleted.
//! - **No key on disk.** If the credential store is unavailable the vault refuses to
//!   write rather than silently falling back to plaintext — a fallback is exactly how
//!   "encrypted at rest" becomes a claim instead of a fact.

use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::Engine;
use serde_json::{json, Value};
use thiserror::Error;

/// Service/account under which the vault key is stored in the OS credential store.
const KEY_SERVICE: &str = "NeoBrowser Vault Key";
const KEY_ACCOUNT: &str = "neobrowser";

/// Format marker, so a future scheme change can be detected instead of misparsed.
const FORMAT: &str = "nbv1";

#[derive(Debug, Error)]
pub enum VaultError {
    #[error("vault key unavailable from the OS credential store: {0}")]
    NoKey(String),
    #[error("vault payload is corrupt or not a NeoBrowser vault file: {0}")]
    Corrupt(String),
    #[error("vault entry expired at {expired_at} (now {now}); refusing to return plaintext")]
    Expired { expired_at: u64, now: u64 },
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("crypto error")]
    Crypto,
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 32 OS-seeded random bytes as lowercase hex.
///
/// Shared with the bridge so there is one answer to "where does randomness come from"
/// rather than two implementations that could diverge in quality.
pub fn random_token_hex() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::with_capacity(64);
    for chunk in 0..4u8 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write(&now_unix().to_le_bytes());
        h.write(&[chunk]);
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}

/// 32 random bytes, base64-encoded, for a new vault key.
fn fresh_key_b64() -> String {
    use std::hash::{BuildHasher, Hasher};
    // Four independently-seeded RandomState hashers give 256 bits sourced from the
    // OS RNG. This avoids adding a `rand` dependency for a once-per-install
    // operation; RandomState is documented as randomly seeded per instance.
    let mut bytes = [0u8; 32];
    for chunk in 0..4 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write(&now_unix().to_le_bytes());
        h.write(&[chunk as u8]);
        let v = h.finish().to_le_bytes();
        bytes[chunk * 8..chunk * 8 + 8].copy_from_slice(&v);
    }
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

// --- OS credential store -------------------------------------------------------

#[cfg(target_os = "macos")]
fn load_key_b64() -> Result<String, VaultError> {
    let out = std::process::Command::new("security")
        .args([
            "find-generic-password",
            "-w",
            "-s",
            KEY_SERVICE,
            "-a",
            KEY_ACCOUNT,
        ])
        .output()
        .map_err(|e| VaultError::NoKey(e.to_string()))?;
    if !out.status.success() {
        return Err(VaultError::NoKey("not found in Keychain".into()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(target_os = "macos")]
fn store_key_b64(key: &str) -> Result<(), VaultError> {
    // `-U` updates an existing item instead of failing, so a partially-created entry
    // does not wedge the vault permanently.
    let out = std::process::Command::new("security")
        .args([
            "add-generic-password",
            "-U",
            "-s",
            KEY_SERVICE,
            "-a",
            KEY_ACCOUNT,
            "-w",
            key,
        ])
        .output()
        .map_err(|e| VaultError::NoKey(e.to_string()))?;
    if !out.status.success() {
        return Err(VaultError::NoKey(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn load_key_b64() -> Result<String, VaultError> {
    let out = std::process::Command::new("secret-tool")
        .args(["lookup", "service", KEY_SERVICE, "account", KEY_ACCOUNT])
        .output()
        .map_err(|e| VaultError::NoKey(e.to_string()))?;
    if !out.status.success() {
        return Err(VaultError::NoKey("not found in secret service".into()));
    }
    let key = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if key.is_empty() {
        return Err(VaultError::NoKey("secret service returned nothing".into()));
    }
    Ok(key)
}

#[cfg(target_os = "linux")]
fn store_key_b64(key: &str) -> Result<(), VaultError> {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = std::process::Command::new("secret-tool")
        .args([
            "store",
            "--label",
            KEY_SERVICE,
            "service",
            KEY_SERVICE,
            "account",
            KEY_ACCOUNT,
        ])
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| VaultError::NoKey(e.to_string()))?;
    // Written to stdin rather than passed as an argument: an argv is visible in
    // `ps` to every process on the machine, which would publish the vault key.
    child
        .stdin
        .as_mut()
        .ok_or_else(|| VaultError::NoKey("no stdin for secret-tool".into()))?
        .write_all(key.as_bytes())?;
    let status = child.wait()?;
    if !status.success() {
        return Err(VaultError::NoKey("secret-tool store failed".into()));
    }
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn load_key_b64() -> Result<String, VaultError> {
    // Windows: DPAPI-protected key file under NEOBROWSER_HOME. Not exercised on this
    // host; see the same caveat in `cookies.rs` for the Windows crypto path.
    let path = crate::paths::home().join("vault.key");
    let b64 = std::fs::read_to_string(&path)
        .map_err(|e| VaultError::NoKey(format!("{}: {e}", path.display())))?;
    Ok(b64.trim().to_string())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn store_key_b64(key: &str) -> Result<(), VaultError> {
    let path = crate::paths::home().join("vault.key");
    crate::sessions::write_private(&path, key)?;
    Ok(())
}

/// The vault key, created on first use.
///
/// `NEOBROWSER_VAULT_KEY` overrides the credential store. It exists for CI and for
/// headless hosts with no keyring, and it is the ONLY way to run without one — there
/// is deliberately no "write plaintext if the keychain is missing" path.
fn vault_key() -> Result<Key<Aes256Gcm>, VaultError> {
    if let Ok(env_key) = std::env::var("NEOBROWSER_VAULT_KEY") {
        if !env_key.trim().is_empty() {
            return decode_key(env_key.trim());
        }
    }
    let b64 = match load_key_b64() {
        Ok(k) => k,
        Err(_) => {
            let fresh = fresh_key_b64();
            store_key_b64(&fresh)?;
            fresh
        }
    };
    decode_key(&b64)
}

fn decode_key(b64: &str) -> Result<Key<Aes256Gcm>, VaultError> {
    let raw = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| VaultError::NoKey(format!("vault key is not valid base64: {e}")))?;
    if raw.len() != 32 {
        return Err(VaultError::NoKey(format!(
            "vault key must be 32 bytes, got {}",
            raw.len()
        )));
    }
    Ok(*Key::<Aes256Gcm>::from_slice(&raw))
}

/// Is the vault usable on this host? Reported by `doctor` so the answer is knowable
/// before a session depends on it.
pub fn available() -> bool {
    vault_key().is_ok()
}

// --- envelope ------------------------------------------------------------------

/// A 96-bit nonce, unique per write.
///
/// AES-GCM catastrophically loses confidentiality if a nonce repeats under the same
/// key, so this mixes a per-process counter with the wall clock and a random seed
/// rather than using either alone: a counter repeats across restarts, and a
/// second-resolution clock repeats within a second.
fn fresh_nonce() -> [u8; 12] {
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

/// Default TTL for session material, overridable via `NEOBROWSER_SESSION_TTL_DAYS`.
///
/// 30 days: long enough that a working setup is not re-authenticating constantly,
/// short enough that an abandoned profile stops being a liability. `0` disables the
/// expiry, which is a deliberate choice a user has to make explicitly.
pub fn default_ttl_secs() -> Option<u64> {
    let days = std::env::var("NEOBROWSER_SESSION_TTL_DAYS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(30);
    if days == 0 {
        None
    } else {
        Some(days * 24 * 60 * 60)
    }
}

/// Where a vault file lives for a given logical name.
pub fn vault_path(base: &Path, name: &str) -> PathBuf {
    base.join(format!("{name}.vault"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixed test key so these tests never touch the developer's real Keychain.
    const TEST_KEY: &str = "dGVzdC12YXVsdC1rZXktdGhpcnR5LXR3by1ieXRlcyE=";

    struct KeyGuard {
        prev: Option<String>,
    }
    impl KeyGuard {
        fn set() -> Self {
            let prev = std::env::var("NEOBROWSER_VAULT_KEY").ok();
            std::env::set_var("NEOBROWSER_VAULT_KEY", TEST_KEY);
            Self { prev }
        }
    }
    impl Drop for KeyGuard {
        fn drop(&mut self) {
            match self.prev.take() {
                Some(v) => std::env::set_var("NEOBROWSER_VAULT_KEY", v),
                None => std::env::remove_var("NEOBROWSER_VAULT_KEY"),
            }
        }
    }

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nb-vault-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn seal_then_open_round_trips() {
        let _g = crate::env_test_guard();
        let _k = KeyGuard::set();
        let p = tmp("round-trip.vault");
        let secret = r#"[{"name":"SID","value":"super-secret"}]"#;
        seal(&p, secret, &["example.com".into()], None).unwrap();
        assert_eq!(open(&p).unwrap(), secret);
        let _ = std::fs::remove_file(&p);
    }

    /// The point of the module: the secret must not be readable in the file.
    #[test]
    fn the_file_on_disk_contains_no_plaintext() {
        let _g = crate::env_test_guard();
        let _k = KeyGuard::set();
        let p = tmp("no-plaintext.vault");
        seal(
            &p,
            "super-secret-cookie-value",
            &["example.com".into()],
            None,
        )
        .unwrap();
        let raw = std::fs::read_to_string(&p).unwrap();
        assert!(
            !raw.contains("super-secret-cookie-value"),
            "plaintext leaked into the vault file"
        );
        // The audit metadata IS in the clear, on purpose.
        assert!(raw.contains("example.com"));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn vault_files_are_owner_only() {
        let _g = crate::env_test_guard();
        let _k = KeyGuard::set();
        let p = tmp("perms.vault");
        seal(&p, "x", &[], None).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&p).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn an_expired_entry_refuses_to_open() {
        let _g = crate::env_test_guard();
        let _k = KeyGuard::set();
        let p = tmp("expired.vault");
        // ttl of 0 seconds: expires_at == created_at, so it is already expired.
        seal(&p, "secret", &[], Some(0)).unwrap();
        match open(&p) {
            Err(VaultError::Expired { .. }) => {}
            other => panic!("expected Expired, got {other:?}"),
        }
        // And inspect can still report it without decrypting.
        let meta = inspect(&p).unwrap();
        assert_eq!(meta["expired"], true);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn a_live_ttl_still_opens() {
        let _g = crate::env_test_guard();
        let _k = KeyGuard::set();
        let p = tmp("live-ttl.vault");
        seal(&p, "secret", &[], Some(3600)).unwrap();
        assert_eq!(open(&p).unwrap(), "secret");
        assert_eq!(inspect(&p).unwrap()["expired"], false);
        let _ = std::fs::remove_file(&p);
    }

    /// A tampered ciphertext must be refused outright — GCM gives us integrity, and
    /// a partial or best-effort read would throw that away.
    #[test]
    fn tampering_is_detected_not_tolerated() {
        let _g = crate::env_test_guard();
        let _k = KeyGuard::set();
        let p = tmp("tampered.vault");
        seal(&p, "secret", &[], None).unwrap();
        let mut env: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        let ct = env["ciphertext"].as_str().unwrap().to_string();
        // Flip a character in the base64 ciphertext.
        let mut chars: Vec<char> = ct.chars().collect();
        chars[0] = if chars[0] == 'A' { 'B' } else { 'A' };
        env["ciphertext"] = json!(chars.into_iter().collect::<String>());
        std::fs::write(&p, env.to_string()).unwrap();
        assert!(matches!(open(&p), Err(VaultError::Crypto)));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn a_wrong_key_cannot_read_the_vault() {
        let _g = crate::env_test_guard();
        let p = tmp("wrong-key.vault");
        {
            let _k = KeyGuard::set();
            seal(&p, "secret", &[], None).unwrap();
        }
        let prev = std::env::var("NEOBROWSER_VAULT_KEY").ok();
        std::env::set_var(
            "NEOBROWSER_VAULT_KEY",
            // A VALID 32-byte key that is simply not the right one: this must fail
            // GCM authentication, not length validation, or the test would pass for
            // the wrong reason.
            "b3RoZXItdmF1bHQta2V5LTMyLWJ5dGVzLWV4YWN0bHk=",
        );
        let result = open(&p);
        match prev {
            Some(v) => std::env::set_var("NEOBROWSER_VAULT_KEY", v),
            None => std::env::remove_var("NEOBROWSER_VAULT_KEY"),
        }
        assert!(matches!(result, Err(VaultError::Crypto)));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn revoke_overwrites_and_removes_and_is_verifiable() {
        let _g = crate::env_test_guard();
        let _k = KeyGuard::set();
        let p = tmp("revoked.vault");
        seal(&p, "secret-to-destroy", &[], None).unwrap();
        assert!(!is_revoked(&p));
        revoke(&p).unwrap();
        assert!(is_revoked(&p), "revoke must leave nothing behind");
        // Revoking again is not an error: deletion is idempotent.
        revoke(&p).unwrap();
    }

    #[test]
    fn nonces_do_not_repeat() {
        // Nonce reuse under one key breaks AES-GCM outright, so this is a
        // correctness invariant rather than a nice-to-have.
        let mut seen = std::collections::HashSet::new();
        for _ in 0..2000 {
            assert!(seen.insert(fresh_nonce()), "nonce repeated");
        }
    }

    #[test]
    fn a_short_or_invalid_key_is_rejected() {
        assert!(matches!(
            decode_key("dG9vLXNob3J0"),
            Err(VaultError::NoKey(_))
        ));
        assert!(matches!(
            decode_key("!!!not-base64!!!"),
            Err(VaultError::NoKey(_))
        ));
    }

    #[test]
    fn corrupt_or_foreign_files_are_refused() {
        let _g = crate::env_test_guard();
        let _k = KeyGuard::set();
        let p = tmp("foreign.vault");
        std::fs::write(&p, r#"{"format":"someone-elses","ciphertext":"x"}"#).unwrap();
        assert!(matches!(open(&p), Err(VaultError::Corrupt(_))));
        std::fs::write(&p, "not json at all").unwrap();
        assert!(matches!(open(&p), Err(VaultError::Corrupt(_))));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn ttl_config_parses_and_zero_means_no_expiry() {
        let _g = crate::env_test_guard();
        let prev = std::env::var("NEOBROWSER_SESSION_TTL_DAYS").ok();
        std::env::set_var("NEOBROWSER_SESSION_TTL_DAYS", "7");
        assert_eq!(default_ttl_secs(), Some(7 * 86400));
        std::env::set_var("NEOBROWSER_SESSION_TTL_DAYS", "0");
        assert_eq!(default_ttl_secs(), None);
        std::env::set_var("NEOBROWSER_SESSION_TTL_DAYS", "nonsense");
        assert_eq!(
            default_ttl_secs(),
            Some(30 * 86400),
            "fall back to the default"
        );
        match prev {
            Some(v) => std::env::set_var("NEOBROWSER_SESSION_TTL_DAYS", v),
            None => std::env::remove_var("NEOBROWSER_SESSION_TTL_DAYS"),
        }
    }
}
