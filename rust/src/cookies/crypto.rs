//! Decrypting the values Chrome stores, per platform.
//!
//! Chrome has used two schemes: AES-128-CBC with a PBKDF2-derived key (older, and still
//! what Linux uses with its well-known fallback password) and AES-256-GCM (current). Both
//! are here because a real profile on a real machine can contain cookies written by both.

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
