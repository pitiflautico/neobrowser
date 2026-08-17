//! Reading cookies out of a real Chrome profile's SQLite store, and decoding Chrome's
//! own encodings.
//!
//! Chrome holds a lock on its own cookie database, so this reads a copy rather than the
//! live file. The rows come back partially encrypted, with timestamps in Chrome's epoch and
//! `sameSite` as an integer, so the conversions live here too.

//! Cross-platform Chrome cookie decryption.
//!
//! The Python port only handled macOS. This closes the multi-OS gap the README
//! promises: the Safe Storage key is retrieved per-platform (macOS Keychain, Linux
//! secret-service, Windows DPAPI) and cookie values are decrypted with the scheme
//! each platform uses:
//!
//! | OS      | key source          | KDF (pbkdf2-hmac-sha1)      | cipher        |
//! |---------|---------------------|-----------------------------|---------------|
//! | macOS   | `security` Keychain | salt "saltysalt", 1003 iter | AES-128-CBC   |
//! | Linux   | secret-tool/peanuts | salt "saltysalt", 1 iter    | AES-128-CBC   |
//! | Windows | DPAPI + Local State | (raw 256-bit key)           | AES-256-GCM   |
//!
//! On every platform Chrome prepends a 32-byte owner hash to the CBC plaintext
//! before encrypting; GCM values are `nonce(12) || ciphertext || tag(16)`.

use super::exclude::{host_under_domain, is_session_auth_excluded};
use super::keys::get_decrypt_key;
use super::CookieError;

/// Convert a Chrome cookie epoch (microseconds since 1601-01-01) to Unix seconds.
pub fn chrome_epoch_to_unix(expires_utc: i64) -> i64 {
    if expires_utc <= 0 {
        return 0;
    }
    (expires_utc - 11_644_473_600_000_000) / 1_000_000
}

/// Map Chrome's sameSite integer to the CDP string.
pub fn same_site(v: i64) -> &'static str {
    match v {
        0 => "None",
        1 => "Lax",
        2 => "Strict",
        _ => "Unspecified",
    }
}

// --- real-profile auto-auth ----------------------------------------------------

/// The real Chrome profile subfolder to pull sessions from (default "Default").
/// Validated so it can't escape the profile directory.
pub fn real_profile_folder() -> Option<String> {
    let name = std::env::var("NEOBROWSER_REAL_PROFILE")
        .ok()
        .filter(|s| !s.is_empty())?;
    let ok = name.len() <= 64
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '_' | '-'));
    ok.then_some(name)
}

/// Read + decrypt the real Chrome profile's cookies into CDP-ready cookie objects,
/// skipping session-identity cookies (see `SESSION_AUTH_EXCLUSIONS`). Optionally
/// filter to `domains` (host_key suffix match). Returns [] if no profile/key/DB.
pub fn read_real_profile_cookies(
    domains: Option<&[String]>,
) -> Result<Vec<serde_json::Value>, CookieError> {
    let Some(folder) = real_profile_folder() else {
        return Ok(Vec::new());
    };
    let src = crate::paths::real_chrome_profile()
        .join(&folder)
        .join("Cookies");
    if !src.exists() {
        return Ok(Vec::new());
    }
    let key = get_decrypt_key()?;
    let rows =
        read_cookie_rows(&src).map_err(|e| CookieError::NoKey(format!("read Cookies DB: {e}")))?;

    let mut out = Vec::new();
    for r in rows {
        if r.name.is_empty() {
            continue;
        }
        if let Some(doms) = domains {
            // Same normalisation for the caller's domain filter: asking for
            // "google.com" must also match the `.google.com` cookies, or an import
            // silently returns nothing and looks like the profile had no cookies.
            if !doms.iter().any(|d| host_under_domain(&r.host_key, d)) {
                continue;
            }
        }
        if is_session_auth_excluded(&r.host_key, &r.name) {
            continue;
        }
        let value = if !r.encrypted_value.is_empty() {
            match key.decrypt(&r.encrypted_value) {
                Some(v) => v,
                None => continue, // undecryptable → drop, never store garbage
            }
        } else {
            r.value.clone()
        };
        let mut cookie = serde_json::json!({
            "name": r.name,
            "value": value,
            "domain": r.host_key,
            "path": if r.path.is_empty() { "/".to_string() } else { r.path },
            "secure": r.is_secure != 0,
            "httpOnly": r.is_httponly != 0,
            "sameSite": same_site(r.samesite),
        });
        let expires = chrome_epoch_to_unix(r.expires_utc);
        if expires > 0 {
            cookie["expires"] = serde_json::json!(expires);
        }
        out.push(cookie);
    }
    Ok(out)
}

struct CookieRow {
    host_key: String,
    name: String,
    value: String,
    path: String,
    expires_utc: i64,
    is_secure: i64,
    is_httponly: i64,
    samesite: i64,
    encrypted_value: Vec<u8>,
}

/// Copy the Cookies DB (+ WAL/SHM sidecars) to a private temp file and read all
/// rows, so we never touch the live file Chrome may be writing.
fn read_cookie_rows(src: &std::path::Path) -> Result<Vec<CookieRow>, rusqlite::Error> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let uniq = format!(
        "neobrowser_cookies_{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    );
    let tmp_dir = std::env::temp_dir().join(uniq);
    let _ = std::fs::create_dir_all(&tmp_dir);
    let tmp_db = tmp_dir.join("Cookies");
    // Copy main DB + sidecars so WAL writes are preserved.
    let _ = std::fs::copy(src, &tmp_db);
    for suffix in ["-wal", "-shm"] {
        let aux = src.with_file_name(format!("Cookies{suffix}"));
        if aux.exists() {
            let _ = std::fs::copy(&aux, tmp_db.with_file_name(format!("Cookies{suffix}")));
        }
    }

    let result = (|| {
        let conn = rusqlite::Connection::open(&tmp_db)?;
        let mut stmt = conn.prepare(
            "SELECT host_key, name, value, path, expires_utc, is_secure, is_httponly, samesite, encrypted_value FROM cookies",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(CookieRow {
                host_key: row.get(0)?,
                name: row.get(1)?,
                value: row.get(2)?,
                path: row.get(3)?,
                expires_utc: row.get(4)?,
                is_secure: row.get(5)?,
                is_httponly: row.get(6)?,
                samesite: row.get(7)?,
                encrypted_value: row.get(8)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
    })();

    let _ = std::fs::remove_dir_all(&tmp_dir);
    result
}

// --- platform key retrieval ----------------------------------------------------
