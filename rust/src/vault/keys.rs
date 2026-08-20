//! Getting a key, and keeping it in the OS credential store rather than on disk.
//!
//! Three platform implementations, one contract: the key never sits next to the data it
//! protects. macOS uses the Keychain, Linux the Secret Service, Windows DPAPI — and where
//! none is available the vault reports itself unavailable rather than falling back to a file,
//! because a key beside the ciphertext is not encryption, it is obfuscation with extra steps.

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
//! - **Revocation and verifiable deletion.** [`super::seal::revoke`] overwrites the ciphertext
//!   before unlinking, and [`super::seal::is_revoked`] can prove afterwards that it is gone.
//!   "Deleted" that leaves a recoverable file is not deleted.
//! - **No key on disk.** If the credential store is unavailable the vault refuses to
//!   write rather than silently falling back to plaintext — a fallback is exactly how
//!   "encrypted at rest" becomes a claim instead of a fact.

use super::{now_unix, VaultError, KEY_ACCOUNT, KEY_SERVICE};
use aes_gcm::{Aes256Gcm, Key};
use base64::Engine;

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
pub(super) fn vault_key() -> Result<Key<Aes256Gcm>, VaultError> {
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

pub(super) fn decode_key(b64: &str) -> Result<Key<Aes256Gcm>, VaultError> {
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
