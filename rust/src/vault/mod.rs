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
//!
//! Split into [`keys`] (obtaining a key from the OS credential store) and [`seal`] (the
//! AES-256-GCM envelope). The error type, the format tag and the paths live here.

use std::path::{Path, PathBuf};

use thiserror::Error;

pub mod keys;
pub mod seal;

pub use keys::available;
pub use seal::{inspect, is_revoked, open, revoke, seal};

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
    use serde_json::{json, Value};

    use super::keys::decode_key;
    use super::seal::fresh_nonce;

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
