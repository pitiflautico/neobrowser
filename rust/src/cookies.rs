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

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CookieError {
    #[error("could not obtain Chrome Safe Storage key: {0}")]
    NoKey(String),
    #[error("unsupported platform for cookie decryption")]
    Unsupported,
}

/// A decrypted cookie ready to hand to `Network.setCookie`.
#[derive(Debug, Clone, PartialEq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: String,
    pub expires_unix: i64,
}

/// Derive an AES-128 key from a Safe Storage password (macOS/Linux path).
pub fn derive_key_cbc(password: &[u8], iterations: u32) -> [u8; 16] {
    use hmac::Hmac;
    use sha1::Sha1;
    let mut key = [0u8; 16];
    pbkdf2::pbkdf2::<Hmac<Sha1>>(password, b"saltysalt", iterations, &mut key)
        .expect("pbkdf2 into 16-byte buffer never fails");
    key
}

/// Decrypt an AES-128-CBC (`v10`) cookie value (macOS/Linux).
///
/// Returns the plaintext, or `None` on a bad decrypt (wrong key / corruption) so a
/// garbage value is never stored as if real. Legacy unencrypted values pass through.
pub fn decrypt_value_cbc(encrypted: &[u8], key: &[u8; 16]) -> Option<String> {
    use aes::Aes128;
    use cbc::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};

    if encrypted.is_empty() {
        return None;
    }
    let ciphertext = if encrypted.starts_with(b"v10") || encrypted.starts_with(b"v11") {
        &encrypted[3..]
    } else {
        // Unencrypted legacy value.
        return String::from_utf8(encrypted.to_vec()).ok();
    };
    if ciphertext.is_empty() || ciphertext.len() % 16 != 0 {
        return None;
    }
    let iv = [b' '; 16];
    type Dec = cbc::Decryptor<Aes128>;
    let dec = Dec::new(key.into(), &iv.into());
    let mut buf = ciphertext.to_vec();
    let plaintext = dec.decrypt_padded_mut::<Pkcs7>(&mut buf).ok()?;
    // Chrome prepends a 32-byte owner hash to the plaintext before encrypting.
    if plaintext.len() < 32 {
        return None;
    }
    Some(String::from_utf8_lossy(&plaintext[32..]).into_owned())
}

/// Decrypt an AES-256-GCM (`v10`) cookie value (Windows).
///
/// Layout: `"v10" || nonce(12) || ciphertext || tag(16)`.
pub fn decrypt_value_gcm(encrypted: &[u8], key: &[u8; 32]) -> Option<String> {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Nonce};

    if encrypted.len() < 3 + 12 + 16 {
        // Not a v10 GCM blob; may be a legacy value.
        if !encrypted.starts_with(b"v10") {
            return String::from_utf8(encrypted.to_vec()).ok();
        }
        return None;
    }
    if !encrypted.starts_with(b"v10") {
        return String::from_utf8(encrypted.to_vec()).ok();
    }
    let nonce = &encrypted[3..15];
    let ct_and_tag = &encrypted[15..];
    let cipher = Aes256Gcm::new(key.into());
    let plaintext = cipher.decrypt(Nonce::from_slice(nonce), ct_and_tag).ok()?;
    Some(String::from_utf8_lossy(&plaintext).into_owned())
}

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

/// Session-identity cookies that must NOT be injected into the ghost browser:
/// Google/LinkedIn/Microsoft log the real browser out when they detect a duplicate
/// session. Preference/consent cookies for the same domains are safe and kept.
/// `(domain suffixes, auth cookie names)`.
const SESSION_AUTH_EXCLUSIONS: &[(&[&str], &[&str])] = &[
    (
        &[
            ".google.com",
            ".google.es",
            ".googleapis.com",
            ".gstatic.com",
            ".youtube.com",
            ".accounts.google.com",
            ".gmail.com",
        ],
        &[
            "SID",
            "HSID",
            "SSID",
            "APISID",
            "SAPISID",
            "__Secure-1PSID",
            "__Secure-3PSID",
            "__Secure-1PAPISID",
            "__Secure-3PAPISID",
            "__Secure-1PSIDCC",
            "__Secure-3PSIDCC",
            "__Secure-1PSIDTS",
            "__Secure-3PSIDTS",
            "SIDCC",
            "LSID",
        ],
    ),
    (&[".linkedin.com"], &["li_at", "JSESSIONID"]),
    (
        &[
            ".login.microsoftonline.com",
            ".login.live.com",
            ".login.windows.net",
            ".microsoftonline.com",
        ],
        &["ESTSAUTH", "ESTSAUTHPERSISTENT", "ESTSAUTHLIGHT", "buid"],
    ),
];

/// True if this cookie is a session-identity cookie we must not inject.
///
/// Escape hatch: `NEOBROWSER_INCLUDE_IDENTITY_COOKIES=1` disables the exclusion,
/// injecting Google/LinkedIn/Microsoft session-identity cookies too. Risk: the
/// provider may flag the duplicate session and log the real browser out. Opt-in,
/// use sparingly (e.g. one automated action per day), never the default.
pub fn is_session_auth_excluded(host_key: &str, name: &str) -> bool {
    if identity_cookies_opt_in() {
        return false;
    }
    SESSION_AUTH_EXCLUSIONS.iter().any(|(domains, names)| {
        names.contains(&name) && domains.iter().any(|d| host_key.ends_with(d))
    })
}

/// True only for an explicit affirmative value.
///
/// Presence alone must NOT be enough: with a bare `is_some()`, spelling out
/// `NEOBROWSER_INCLUDE_IDENTITY_COOKIES=0` to disable the escape hatch would
/// switch it ON — silently injecting the identity cookies the exclusion list
/// exists to hold back, and risking a logout of the user's real browser.
fn identity_cookies_opt_in() -> bool {
    match std::env::var("NEOBROWSER_INCLUDE_IDENTITY_COOKIES") {
        Ok(v) => matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => false,
    }
}

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

/// Explicit allow-list of domains to import cookies for when `NEOBROWSER_REAL_PROFILE`
/// is set. Defaults to `None`, which means **no real-profile cookies are injected**.
/// This prevents platforms from detecting the cloned session and logging the real
/// browser out. To opt in, set e.g.
/// `NEOBROWSER_REAL_PROFILE_DOMAINS=x.com,twitter.com,reddit.com`.
///
/// Domains are matched as host_key suffixes, so `x.com` also covers `api.x.com`.
/// Empty or all-whitespace entries are ignored.
pub fn real_profile_domains() -> Option<Vec<String>> {
    let raw = std::env::var("NEOBROWSER_REAL_PROFILE_DOMAINS").ok()?;
    let domains: Vec<String> = raw
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        })
        .collect();
    if domains.is_empty() {
        None
    } else {
        Some(domains)
    }
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
            if !doms.iter().any(|d| r.host_key.ends_with(d)) {
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

/// The Safe Storage key for the current platform.
pub fn get_decrypt_key() -> Result<DecryptKey, CookieError> {
    #[cfg(target_os = "macos")]
    {
        let pw = macos_safe_storage_password()?;
        Ok(DecryptKey::Cbc(derive_key_cbc(pw.as_bytes(), 1003)))
    }
    #[cfg(target_os = "linux")]
    {
        let pw = linux_safe_storage_password();
        Ok(DecryptKey::Cbc(derive_key_cbc(pw.as_bytes(), 1)))
    }
    #[cfg(target_os = "windows")]
    {
        let key = windows_master_key()?;
        Ok(DecryptKey::Gcm(key))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        Err(CookieError::Unsupported)
    }
}

/// A platform-appropriate decryption key + the cipher it implies.
pub enum DecryptKey {
    Cbc([u8; 16]),
    Gcm([u8; 32]),
}

impl DecryptKey {
    /// Decrypt a cookie value with the right cipher for this key.
    pub fn decrypt(&self, encrypted: &[u8]) -> Option<String> {
        match self {
            DecryptKey::Cbc(k) => decrypt_value_cbc(encrypted, k),
            DecryptKey::Gcm(k) => decrypt_value_gcm(encrypted, k),
        }
    }
}

#[cfg(target_os = "macos")]
fn macos_safe_storage_password() -> Result<String, CookieError> {
    let out = std::process::Command::new("security")
        .args([
            "find-generic-password",
            "-w",
            "-s",
            "Chrome Safe Storage",
            "-a",
            "Chrome",
        ])
        .output()
        .map_err(|e| CookieError::NoKey(e.to_string()))?;
    if !out.status.success() {
        return Err(CookieError::NoKey(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(target_os = "linux")]
fn linux_safe_storage_password() -> String {
    // Try the freedesktop secret service via secret-tool; fall back to the well-known
    // "peanuts" password Chrome uses when no keyring is available.
    let out = std::process::Command::new("secret-tool")
        .args(["lookup", "application", "chrome"])
        .output();
    if let Ok(out) = out {
        if out.status.success() {
            let pw = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !pw.is_empty() {
                return pw;
            }
        }
    }
    "peanuts".to_string()
}

/// Windows: DPAPI-decrypt the base64 `os_crypt.encrypted_key` from Local State.
///
/// NOTE: this path compiles only on Windows and has not been exercised on this
/// (macOS) host — it is written to Chrome's documented format and the AES-256-GCM
/// value decryption it feeds IS covered by tests.
#[cfg(target_os = "windows")]
fn windows_master_key() -> Result<[u8; 32], CookieError> {
    use base64::Engine;

    let local_state = crate::paths::real_chrome_profile().join("Local State");
    let text = std::fs::read_to_string(&local_state)
        .map_err(|e| CookieError::NoKey(format!("read Local State: {e}")))?;
    let json: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| CookieError::NoKey(e.to_string()))?;
    let b64 = json
        .get("os_crypt")
        .and_then(|o| o.get("encrypted_key"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| CookieError::NoKey("no os_crypt.encrypted_key".into()))?;
    let mut blob = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| CookieError::NoKey(e.to_string()))?;
    // Strip the 5-byte "DPAPI" prefix.
    if blob.starts_with(b"DPAPI") {
        blob.drain(0..5);
    }
    let key = dpapi_unprotect(&blob)?;
    key.try_into()
        .map_err(|_| CookieError::NoKey("DPAPI key not 32 bytes".into()))
}

#[cfg(target_os = "windows")]
fn dpapi_unprotect(data: &[u8]) -> Result<Vec<u8>, CookieError> {
    #[repr(C)]
    struct DataBlob {
        cb_data: u32,
        pb_data: *mut u8,
    }
    #[link(name = "crypt32")]
    extern "system" {
        fn CryptUnprotectData(
            p_data_in: *const DataBlob,
            ppsz_desc: *mut *mut u16,
            p_opt_entropy: *const DataBlob,
            p_reserved: *mut core::ffi::c_void,
            p_prompt: *mut core::ffi::c_void,
            dw_flags: u32,
            p_data_out: *mut DataBlob,
        ) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn LocalFree(h_mem: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
    }
    let mut input = DataBlob {
        cb_data: data.len() as u32,
        pb_data: data.as_ptr() as *mut u8,
    };
    let mut output = DataBlob {
        cb_data: 0,
        pb_data: core::ptr::null_mut(),
    };
    let ok = unsafe {
        CryptUnprotectData(
            &mut input,
            core::ptr::null_mut(),
            core::ptr::null(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            0,
            &mut output,
        )
    };
    if ok == 0 {
        return Err(CookieError::NoKey("CryptUnprotectData failed".into()));
    }
    let out =
        unsafe { std::slice::from_raw_parts(output.pb_data, output.cb_data as usize).to_vec() };
    unsafe {
        LocalFree(output.pb_data as *mut core::ffi::c_void);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::Aes128;
    use cbc::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};

    /// Build a v10 CBC blob the way Chrome does: encrypt (32-byte prefix || value).
    fn make_cbc_blob(value: &str, key: &[u8; 16]) -> Vec<u8> {
        let iv = [b' '; 16];
        let mut plaintext = vec![0u8; 32];
        plaintext.extend_from_slice(value.as_bytes());
        type Enc = cbc::Encryptor<Aes128>;
        let enc = Enc::new(key.into(), &iv.into());
        let mut buf = plaintext.clone();
        let pad_len = 16 - (buf.len() % 16);
        buf.resize(buf.len() + pad_len, 0);
        let ct = enc
            .encrypt_padded_mut::<Pkcs7>(&mut buf, plaintext.len())
            .unwrap();
        let mut out = b"v10".to_vec();
        out.extend_from_slice(ct);
        out
    }

    #[test]
    fn cbc_round_trip_recovers_value_and_strips_prefix() {
        let key = derive_key_cbc(b"hunter2", 1003);
        let blob = make_cbc_blob("session=abc123; secure", &key);
        let out = decrypt_value_cbc(&blob, &key).unwrap();
        assert_eq!(out, "session=abc123; secure");
    }

    #[test]
    fn cbc_wrong_key_is_rejected_not_garbage() {
        let good = derive_key_cbc(b"correct", 1003);
        let bad = derive_key_cbc(b"wrong", 1003);
        let blob = make_cbc_blob("value", &good);
        // Wrong key → bad PKCS7 padding almost always → None (never a bogus string).
        assert!(decrypt_value_cbc(&blob, &bad).is_none());
    }

    #[test]
    fn cbc_legacy_unencrypted_passes_through() {
        let key = [0u8; 16];
        assert_eq!(
            decrypt_value_cbc(b"plainvalue", &key).as_deref(),
            Some("plainvalue")
        );
    }

    #[test]
    fn gcm_round_trip() {
        use aes_gcm::aead::{Aead, KeyInit};
        use aes_gcm::{Aes256Gcm, Nonce};
        let key = [7u8; 32];
        let nonce = [3u8; 12];
        let cipher = Aes256Gcm::new((&key).into());
        let ct = cipher
            .encrypt(Nonce::from_slice(&nonce), b"tok=xyz".as_ref())
            .unwrap();
        let mut blob = b"v10".to_vec();
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&ct);
        assert_eq!(decrypt_value_gcm(&blob, &key).as_deref(), Some("tok=xyz"));
    }

    #[test]
    fn chrome_epoch_conversion() {
        // 13335770000000000 µs since 1601 → a 2024-ish Unix timestamp.
        assert_eq!(chrome_epoch_to_unix(0), 0);
        let unix = chrome_epoch_to_unix(13_335_770_000_000_000);
        assert!(unix > 1_600_000_000 && unix < 2_000_000_000, "got {unix}");
    }

    #[test]
    fn same_site_mapping() {
        assert_eq!(same_site(0), "None");
        assert_eq!(same_site(1), "Lax");
        assert_eq!(same_site(2), "Strict");
        assert_eq!(same_site(-1), "Unspecified");
    }

    #[test]
    fn pbkdf2_iterations_differ_between_macos_and_linux() {
        // Same password, different iteration counts → different keys (mac=1003, linux=1).
        assert_ne!(derive_key_cbc(b"pw", 1003), derive_key_cbc(b"pw", 1));
    }

    #[test]
    fn session_auth_exclusions_block_identity_cookies() {
        assert!(is_session_auth_excluded(".google.com", "SID"));
        assert!(is_session_auth_excluded(
            "mail.google.com",
            "__Secure-1PSID"
        ));
        assert!(is_session_auth_excluded(".linkedin.com", "li_at"));
        assert!(is_session_auth_excluded(".login.live.com", "ESTSAUTH"));
        // Preference/consent cookies for the same domains are NOT excluded.
        assert!(!is_session_auth_excluded(".google.com", "NID"));
        assert!(!is_session_auth_excluded(".linkedin.com", "lang"));
        // Same cookie name on an unrelated domain is not excluded.
        assert!(!is_session_auth_excluded(".example.com", "SID"));
    }

    /// The escape hatch must need an explicit yes. Anything else — absent, empty,
    /// or a spelled-out "0"/"false" — keeps the identity cookies held back.
    #[test]
    fn identity_cookie_escape_hatch_requires_an_affirmative_value() {
        let _g = crate::env_test_guard();
        let prev = std::env::var("NEOBROWSER_INCLUDE_IDENTITY_COOKIES").ok();

        std::env::remove_var("NEOBROWSER_INCLUDE_IDENTITY_COOKIES");
        assert!(is_session_auth_excluded(".google.com", "SID"), "unset");

        for off in ["0", "false", "no", "", "  "] {
            std::env::set_var("NEOBROWSER_INCLUDE_IDENTITY_COOKIES", off);
            assert!(
                is_session_auth_excluded(".google.com", "SID"),
                "{off:?} must NOT enable the hatch"
            );
        }
        for on in ["1", "true", "YES", " on "] {
            std::env::set_var("NEOBROWSER_INCLUDE_IDENTITY_COOKIES", on);
            assert!(
                !is_session_auth_excluded(".google.com", "SID"),
                "{on:?} must enable the hatch"
            );
        }

        match prev {
            Some(v) => std::env::set_var("NEOBROWSER_INCLUDE_IDENTITY_COOKIES", v),
            None => std::env::remove_var("NEOBROWSER_INCLUDE_IDENTITY_COOKIES"),
        }
    }

    #[test]
    fn real_profile_folder_validation() {
        let _g = crate::env_test_guard();
        let prev = std::env::var("NEOBROWSER_REAL_PROFILE").ok();
        std::env::set_var("NEOBROWSER_REAL_PROFILE", "Profile 1");
        assert_eq!(real_profile_folder().as_deref(), Some("Profile 1"));
        std::env::set_var("NEOBROWSER_REAL_PROFILE", "../evil");
        assert_eq!(real_profile_folder(), None); // path-traversal rejected
        std::env::remove_var("NEOBROWSER_REAL_PROFILE");
        assert_eq!(real_profile_folder(), None);
        if let Some(v) = prev {
            std::env::set_var("NEOBROWSER_REAL_PROFILE", v);
        }
    }

    #[test]
    fn real_profile_domains_requires_explicit_allowlist() {
        let _g = crate::env_test_guard();
        let prev = std::env::var("NEOBROWSER_REAL_PROFILE_DOMAINS").ok();

        std::env::remove_var("NEOBROWSER_REAL_PROFILE_DOMAINS");
        assert_eq!(real_profile_domains(), None, "unset must opt out");

        for empty in ["", "  ", ",,", " , "] {
            std::env::set_var("NEOBROWSER_REAL_PROFILE_DOMAINS", empty);
            assert_eq!(real_profile_domains(), None, "{empty:?} must be ignored");
        }

        std::env::set_var("NEOBROWSER_REAL_PROFILE_DOMAINS", "x.com, twitter.com");
        assert_eq!(
            real_profile_domains(),
            Some(vec!["x.com".into(), "twitter.com".into()])
        );

        std::env::set_var("NEOBROWSER_REAL_PROFILE_DOMAINS", "API.Example.COM");
        assert_eq!(real_profile_domains(), Some(vec!["api.example.com".into()]));

        // Path traversal / injection attempts are rejected.
        std::env::set_var("NEOBROWSER_REAL_PROFILE_DOMAINS", "../../etc/passwd");
        assert_eq!(real_profile_domains(), None);

        match prev {
            Some(v) => std::env::set_var("NEOBROWSER_REAL_PROFILE_DOMAINS", v),
            None => std::env::remove_var("NEOBROWSER_REAL_PROFILE_DOMAINS"),
        }
    }
}
