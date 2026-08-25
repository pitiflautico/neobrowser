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
//!
//! Split by responsibility: [`crypto`] decrypts the stored values, [`read`] pulls rows out
//! of Chrome's SQLite store and decodes its encodings, [`exclude`] decides what is
//! deliberately left behind, and [`keys`] obtains the key from the OS credential store. The
//! error and the `Cookie` shape stay here, since every part produces one or the other.

use thiserror::Error;

pub mod crypto;
pub mod exclude;
pub mod keys;
pub mod read;

pub use crypto::{decrypt_value_cbc, decrypt_value_gcm, derive_key_cbc};
pub use exclude::{is_session_auth_excluded, is_session_cookie_excluded};
pub use keys::{get_decrypt_key, DecryptKey};
pub use read::{chrome_epoch_to_unix, read_real_profile_cookies, real_profile_folder, same_site};

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

#[cfg(test)]
mod tests {
    use super::exclude::host_under_domain;
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

    /// Regression: the exclusion list is written with leading dots, so matching with a
    /// bare `ends_with` missed HOST-ONLY cookies — a `SID` scoped to exactly `google.com`
    /// was injected despite being the very thing the list exists to hold back.
    #[test]
    fn identity_exclusion_covers_host_only_cookies() {
        let _g = crate::env_test_guard();
        // The bug: no leading dot on the host_key.
        assert!(
            is_session_auth_excluded("google.com", "SID"),
            "a host-only Google SID cookie must be excluded"
        );
        assert!(is_session_auth_excluded("linkedin.com", "li_at"));
        assert!(is_session_auth_excluded("microsoftonline.com", "ESTSAUTH"));
        // The domain form and subdomains keep working.
        assert!(is_session_auth_excluded(".google.com", "SID"));
        assert!(is_session_auth_excluded("mail.google.com", "SID"));
        assert!(is_session_auth_excluded(".mail.google.com", "SID"));
    }

    /// The fix must not over-match: a lookalike domain an attacker controls is not Google.
    #[test]
    fn identity_exclusion_respects_label_boundaries() {
        let _g = crate::env_test_guard();
        for lookalike in [
            "evilgoogle.com",
            "google.com.evil.test",
            "notgoogle.com",
            "googlex.com",
        ] {
            assert!(
                !is_session_auth_excluded(lookalike, "SID"),
                "{lookalike} is not Google and must not be treated as it"
            );
        }
        // And a non-identity cookie on a listed domain is still fine to clone.
        assert!(!is_session_auth_excluded("google.com", "NID"));
    }

    #[test]
    fn host_under_domain_normalises_the_leading_dot_both_ways() {
        for (host, dom) in [
            ("google.com", ".google.com"),
            (".google.com", ".google.com"),
            ("google.com", "google.com"),
            (".google.com", "google.com"),
            ("mail.google.com", ".google.com"),
        ] {
            assert!(host_under_domain(host, dom), "{host} should be under {dom}");
        }
        for (host, dom) in [
            ("evilgoogle.com", ".google.com"),
            ("google.com.evil.test", ".google.com"),
            ("google.co", ".google.com"),
        ] {
            assert!(!host_under_domain(host, dom), "{host} is NOT under {dom}");
        }
    }

    #[test]
    fn session_auth_exclusions_block_identity_cookies() {
        // `is_session_auth_excluded` consults NEOBROWSER_INCLUDE_IDENTITY_COOKIES,
        // which the escape-hatch test below flips. Without this lock the two race and
        // this one fails intermittently under cargo's parallel threads.
        let _g = crate::env_test_guard();
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

    /// Session cookies are excluded from real-profile import by default: cloning a
    /// live session token into a second browser is exactly what triggers "new device"
    /// revocation on the real browser.
    #[test]
    fn session_cookies_excluded_by_default() {
        let _g = crate::env_test_guard();
        std::env::remove_var("NEOBROWSER_IMPORT_SESSION_COOKIES");
        assert!(is_session_cookie_excluded(0));
        assert!(is_session_cookie_excluded(-1));
        assert!(!is_session_cookie_excluded(1_700_000_000));
    }

    #[test]
    fn session_cookie_opt_in_requires_affirmative_value() {
        let _g = crate::env_test_guard();
        let prev = std::env::var("NEOBROWSER_IMPORT_SESSION_COOKIES").ok();

        std::env::remove_var("NEOBROWSER_IMPORT_SESSION_COOKIES");
        assert!(is_session_cookie_excluded(0), "unset");

        for off in ["0", "false", "no", "", "  "] {
            std::env::set_var("NEOBROWSER_IMPORT_SESSION_COOKIES", off);
            assert!(
                is_session_cookie_excluded(0),
                "{off:?} must NOT enable session-cookie import"
            );
        }
        for on in ["1", "true", "YES", " on "] {
            std::env::set_var("NEOBROWSER_IMPORT_SESSION_COOKIES", on);
            assert!(
                !is_session_cookie_excluded(0),
                "{on:?} must enable session-cookie import"
            );
        }

        match prev {
            Some(v) => std::env::set_var("NEOBROWSER_IMPORT_SESSION_COOKIES", v),
            None => std::env::remove_var("NEOBROWSER_IMPORT_SESSION_COOKIES"),
        }
    }

    /// Additional providers beyond Google/LinkedIn/Microsoft must also have their
    /// identity cookies held back by default.
    #[test]
    fn additional_identity_exclusions_are_active() {
        let _g = crate::env_test_guard();
        assert!(is_session_auth_excluded(".github.com", "user_session"));
        assert!(is_session_auth_excluded(
            "github.com",
            "__Host-user_session_same_site"
        ));
        assert!(is_session_auth_excluded(".x.com", "auth_token"));
        assert!(is_session_auth_excluded("twitter.com", "ct0"));
        assert!(is_session_auth_excluded(".reddit.com", "reddit_session"));
        assert!(is_session_auth_excluded(".facebook.com", "c_user"));
        assert!(is_session_auth_excluded(".instagram.com", "sessionid"));
        assert!(is_session_auth_excluded(".slack.com", "d"));
        assert!(is_session_auth_excluded(".discord.com", "token"));
        assert!(is_session_auth_excluded(".notion.so", "token_v2"));
        assert!(is_session_auth_excluded("www.notion.so", "notion_user_id"));
        assert!(is_session_auth_excluded(".figma.com", "figma.session"));
        assert!(is_session_auth_excluded(".linear.app", "linear"));
        assert!(is_session_auth_excluded(".vercel.com", "auth-token"));
        assert!(is_session_auth_excluded(".cloudflare.com", "cftoken"));
        assert!(is_session_auth_excluded(".stripe.com", "session"));
        assert!(is_session_auth_excluded(".dropbox.com", "bjarne"));
        assert!(is_session_auth_excluded(".apple.com", "aa"));
        assert!(is_session_auth_excluded(".amazon.com", "at-main"));
        assert!(is_session_auth_excluded(".spotify.com", "sp_dc"));
        assert!(is_session_auth_excluded(".zoom.us", "_zm_ssid"));
        assert!(is_session_auth_excluded(
            "myteam.atlassian.net",
            "cloud.session.token"
        ));
        assert!(is_session_auth_excluded(".gitlab.com", "_gitlab_session"));
        // Consent/preference cookies on the same domains should still be importable.
        assert!(!is_session_auth_excluded(".notion.so", "notion_consent"));
        assert!(!is_session_auth_excluded(".figma.com", "analytics"));
    }
}
