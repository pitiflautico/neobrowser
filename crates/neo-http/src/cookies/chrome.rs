//! Chrome cookie importer for macOS.
//!
//! Extracts cookies from a Chrome profile's SQLite DB, decrypts them
//! using the encryption key stored in the macOS Keychain, and returns
//! plain-text [`neo_types::Cookie`] values ready for import.

use crate::HttpError;
use neo_types::Cookie;
use std::path::PathBuf;

/// Error type for Chrome cookie import operations.
#[derive(Debug, thiserror::Error)]
pub enum CookieImportError {
    #[error("keychain error: {0}")]
    Keychain(String),
    #[error("chrome db error: {0}")]
    ChromeDb(String),
    #[error("decryption error: {0}")]
    Decrypt(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

impl From<CookieImportError> for HttpError {
    fn from(e: CookieImportError) -> Self {
        HttpError::CookieStore(e.to_string())
    }
}

/// Imports cookies from a Chrome profile on macOS.
pub struct ChromeCookieImporter {
    profile_name: String,
    domain_filter: Option<String>,
}

impl ChromeCookieImporter {
    /// Create a new importer for the given Chrome profile.
    ///
    /// If `domain` is provided, only cookies matching that domain are imported.
    pub fn new(profile: &str, domain: Option<&str>) -> Self {
        Self {
            profile_name: profile.to_string(),
            domain_filter: domain.map(|d| d.to_string()),
        }
    }

    /// Import and decrypt cookies from the Chrome profile.
    ///
    /// This will:
    /// 1. Copy the Chrome Cookies DB to a temp file (safe while Chrome runs)
    /// 2. Read the encryption key from macOS Keychain (may prompt user)
    /// 3. Derive the AES key via PBKDF2
    /// 4. Decrypt each cookie value
    pub fn import(&self) -> Result<Vec<Cookie>, CookieImportError> {
        let chrome_db_path = self.chrome_cookies_path()?;
        if !chrome_db_path.exists() {
            return Err(CookieImportError::ChromeDb(format!(
                "Chrome Cookies DB not found at {}",
                chrome_db_path.display()
            )));
        }

        // Copy DB to temp file so we don't conflict with Chrome's lock.
        let tmp = std::env::temp_dir().join("neorender_chrome_cookies.db");
        std::fs::copy(&chrome_db_path, &tmp)?;

        let encryption_key = get_chrome_keychain_password()?;
        let aes_key = derive_aes_key(&encryption_key);

        let cookies = self.read_and_decrypt(&tmp, &aes_key)?;

        // Clean up temp file (best-effort).
        let _ = std::fs::remove_file(&tmp);

        Ok(cookies)
    }

    /// Path to the Chrome Cookies SQLite DB for this profile.
    fn chrome_cookies_path(&self) -> Result<PathBuf, CookieImportError> {
        let home = std::env::var("HOME")
            .map_err(|_| CookieImportError::Io(std::io::Error::other("HOME not set")))?;
        Ok(PathBuf::from(format!(
            "{home}/Library/Application Support/Google/Chrome/{}/Cookies",
            self.profile_name
        )))
    }

    /// Read cookies from the copied DB and decrypt their values.
    fn read_and_decrypt(
        &self,
        db_path: &PathBuf,
        aes_key: &[u8; 16],
    ) -> Result<Vec<Cookie>, CookieImportError> {
        let conn = rusqlite::Connection::open_with_flags(
            db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(|e| CookieImportError::ChromeDb(e.to_string()))?;

        let (sql, params_owned): (String, Vec<String>) = match &self.domain_filter {
            Some(domain) => {
                let dot_domain = format!(".{domain}");
                let like_domain = format!("%.{domain}");
                (
                    "SELECT host_key, name, encrypted_value, path, expires_utc, \
                     is_httponly, is_secure, samesite \
                     FROM cookies WHERE host_key = ?1 OR host_key = ?2 OR host_key LIKE ?3"
                        .to_string(),
                    vec![domain.clone(), dot_domain, like_domain],
                )
            }
            None => (
                "SELECT host_key, name, encrypted_value, path, expires_utc, \
                 is_httponly, is_secure, samesite FROM cookies"
                    .to_string(),
                vec![],
            ),
        };

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| CookieImportError::ChromeDb(e.to_string()))?;

        let params_refs: Vec<&dyn rusqlite::types::ToSql> = params_owned
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();

        let rows = stmt
            .query_map(params_refs.as_slice(), |row| {
                Ok(RawChromeRow {
                    host_key: row.get(0)?,
                    name: row.get(1)?,
                    encrypted_value: row.get(2)?,
                    path: row.get(3)?,
                    expires_utc: row.get(4)?,
                    is_httponly: row.get(5)?,
                    is_secure: row.get(6)?,
                    samesite: row.get(7)?,
                })
            })
            .map_err(|e| CookieImportError::ChromeDb(e.to_string()))?;

        let mut cookies = Vec::new();
        for row in rows {
            let row = row.map_err(|e| CookieImportError::ChromeDb(e.to_string()))?;
            let value = decrypt_cookie_value(&row.encrypted_value, aes_key);
            cookies.push(Cookie {
                name: row.name,
                value,
                domain: row.host_key,
                path: row.path,
                expires: chrome_expires_to_unix(row.expires_utc),
                http_only: row.is_httponly != 0,
                secure: row.is_secure != 0,
                same_site: chrome_samesite_to_string(row.samesite),
            });
        }

        Ok(cookies)
    }
}

/// Raw row from Chrome's cookies table.
struct RawChromeRow {
    host_key: String,
    name: String,
    encrypted_value: Vec<u8>,
    path: String,
    expires_utc: i64,
    is_httponly: i32,
    is_secure: i32,
    samesite: i32,
}

/// Get Chrome's encryption password from macOS Keychain.
fn get_chrome_keychain_password() -> Result<String, CookieImportError> {
    let output = std::process::Command::new("security")
        .args(["find-generic-password", "-s", "Chrome Safe Storage", "-w"])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(CookieImportError::Keychain(format!(
            "security command failed: {stderr}"
        )));
    }

    let password = String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_string();
    if password.is_empty() {
        return Err(CookieImportError::Keychain(
            "empty password from Keychain".into(),
        ));
    }
    Ok(password)
}

/// Derive AES-128 key from Chrome's keychain password using PBKDF2-SHA1.
///
/// Parameters: salt = "saltysalt", iterations = 1003, key_len = 16.
fn derive_aes_key(password: &str) -> [u8; 16] {
    use hmac::Hmac;
    use pbkdf2::pbkdf2;
    use sha1::Sha1;

    let mut key = [0u8; 16];
    pbkdf2::<Hmac<Sha1>>(password.as_bytes(), b"saltysalt", 1003, &mut key)
        .expect("PBKDF2 derivation failed");
    key
}

/// Decrypt a single Chrome cookie value.
///
/// Chrome on macOS uses AES-128-CBC with:
/// - First 3 bytes: "v10" version prefix (skipped)
/// - IV: 16 bytes of 0x20 (space)
/// - Padding: PKCS7
fn decrypt_cookie_value(encrypted: &[u8], key: &[u8; 16]) -> String {
    // Unencrypted cookies have no "v10" prefix.
    if encrypted.len() < 3 {
        return String::from_utf8_lossy(encrypted).to_string();
    }
    if &encrypted[..3] != b"v10" {
        return String::from_utf8_lossy(encrypted).to_string();
    }

    let ciphertext = &encrypted[3..];
    if ciphertext.is_empty() {
        return String::new();
    }

    use aes::cipher::{BlockDecryptMut, KeyIvInit};
    type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

    let iv = [0x20u8; 16];
    let mut buf = ciphertext.to_vec();

    match Aes128CbcDec::new(key.into(), &iv.into()).decrypt_padded_mut::<aes::cipher::block_padding::Pkcs7>(&mut buf) {
        Ok(plaintext) => {
            // Chrome on recent macOS versions prepends a 32-byte binary header
            // to the decrypted cookie value. The header varies per cookie but
            // is always exactly 32 bytes of non-UTF8 binary data followed by
            // the actual cookie value. Detect by checking if byte 32+ is valid
            // UTF-8 while the prefix is not.
            let start = if plaintext.len() > 32
                && std::str::from_utf8(&plaintext[..32]).is_err()
                && std::str::from_utf8(&plaintext[32..]).is_ok()
            {
                32
            } else if plaintext.len() > 32 && std::str::from_utf8(plaintext).is_err() {
                // Fallback: find first run of valid UTF-8 starting at 32.
                32
            } else {
                0
            };
            String::from_utf8_lossy(&plaintext[start..]).to_string()
        }
        Err(_) => {
            // Decryption failed — return empty rather than garbage.
            String::new()
        }
    }
}

/// Convert Chrome's expires_utc (microseconds since 1601-01-01) to Unix epoch seconds.
///
/// Chrome uses WebKit/Windows epoch. 0 means session cookie.
fn chrome_expires_to_unix(chrome_us: i64) -> Option<i64> {
    if chrome_us == 0 {
        return None; // Session cookie.
    }
    // Microseconds between 1601-01-01 and 1970-01-01.
    const WEBKIT_EPOCH_OFFSET_US: i64 = 11_644_473_600_000_000;
    let unix_us = chrome_us - WEBKIT_EPOCH_OFFSET_US;
    if unix_us <= 0 {
        return None;
    }
    Some(unix_us / 1_000_000)
}

/// Map Chrome's samesite integer to a string value.
fn chrome_samesite_to_string(val: i32) -> Option<String> {
    match val {
        0 => None,          // unspecified
        1 => Some("None".to_string()),
        2 => Some("Lax".to_string()),
        3 => Some("Strict".to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chrome_expires_conversion() {
        // 0 = session cookie
        assert_eq!(chrome_expires_to_unix(0), None);
        // Known value: 2025-01-01 00:00:00 UTC
        // Unix: 1735689600
        // Chrome: (1735689600 * 1_000_000) + 11_644_473_600_000_000
        let chrome_val = 1_735_689_600_000_000i64 + 11_644_473_600_000_000i64;
        assert_eq!(chrome_expires_to_unix(chrome_val), Some(1_735_689_600));
    }

    #[test]
    fn test_chrome_samesite_mapping() {
        assert_eq!(chrome_samesite_to_string(0), None);
        assert_eq!(chrome_samesite_to_string(1), Some("None".to_string()));
        assert_eq!(chrome_samesite_to_string(2), Some("Lax".to_string()));
        assert_eq!(chrome_samesite_to_string(3), Some("Strict".to_string()));
        assert_eq!(chrome_samesite_to_string(99), None);
    }

    #[test]
    fn test_decrypt_unencrypted_value() {
        // No "v10" prefix = plain text passthrough.
        let plain = b"hello_world";
        let key = [0u8; 16];
        assert_eq!(decrypt_cookie_value(plain, &key), "hello_world");
    }

    #[test]
    fn test_decrypt_empty() {
        let key = [0u8; 16];
        assert_eq!(decrypt_cookie_value(b"", &key), "");
        assert_eq!(decrypt_cookie_value(b"v10", &key), "");
    }
}
