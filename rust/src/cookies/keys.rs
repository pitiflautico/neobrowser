//! Getting the key Chrome used, from the OS credential store.
//!
//! Each platform hides it differently: macOS in the Keychain, Linux behind Secret Service
//! (with a documented fallback password when no keyring is present), Windows under DPAPI.
//! The DPAPI path is implemented directly against the Win32 API because the alternative is a
//! dependency that does the same three calls.

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

use super::crypto::{decrypt_value_cbc, decrypt_value_gcm, derive_key_cbc};
use super::CookieError;

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
    // SAFETY: `input` borrows `data` for the duration of the call and Windows does not
    // retain it. `output` is zeroed beforehand, and DPAPI either fills it and returns
    // non-zero or leaves it untouched and returns zero — which is checked immediately
    // below before anything reads it. The four null pointers are the documented "no
    // entropy, no prompt, no reserved" arguments.
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
    // SAFETY: reached only when `ok != 0`, which is DPAPI's contract for "pb_data points
    // to cb_data valid bytes that I allocated". `.to_vec()` copies them out before the
    // `LocalFree` below, so the slice never outlives the allocation.
    let out =
        unsafe { std::slice::from_raw_parts(output.pb_data, output.cb_data as usize).to_vec() };
    // SAFETY: DPAPI allocates `pb_data` with LocalAlloc, so LocalFree is the matching
    // deallocator. It runs exactly once, after the copy above, and `output` is not used
    // again — so no double free and no use-after-free.
    unsafe {
        LocalFree(output.pb_data as *mut core::ffi::c_void);
    }
    Ok(out)
}
