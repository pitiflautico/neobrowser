//! Saving, restoring and revoking the cookie jar.
//!
//! Cookies are the whole reason a persistent profile is worth the trouble: they are what
//! makes "log in once" true. They are also the most sensitive thing this tool touches, so
//! they are sealed rather than written as JSON, and `revoke_session` has to actually remove
//! every file — it once reported success while leaving the sealed copy behind, because its
//! target list named only the legacy `.json` path.

//! Cookie/session snapshotting + a scripted login, over CDP.
//!
//! These are the *manual* snapshot paths (`save_cookies`/`restore_cookies`) plus a
//! full session save (cookies + localStorage) and a scripted `login`. They operate
//! on the live tab's cookies via `Network.getCookies`/`Network.setCookies` — no
//! SQLite needed (that is the separate real-profile auto-auth path, which reuses the
//! Phase-5 crypto in `cookies.rs`).

use std::path::PathBuf;

use serde_json::{json, Value};

use super::{cookies_path, profile_name, session_dir};
use crate::cdp::{CdpClient, CdpError};
use crate::paths;

/// All browser cookies via CDP.
pub(super) async fn get_all_cookies(client: &CdpClient) -> Result<Vec<Value>, CdpError> {
    let result = client.send("Network.getCookies", json!({})).await?;
    Ok(result
        .get("cookies")
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default())
}

/// The encrypted store for this profile's cookies. See [`crate::vault`].
pub(super) fn cookies_vault_path() -> PathBuf {
    crate::vault::vault_path(&paths::cookies_base(), &profile_name())
}

/// Distinct domains present in a cookie list, for the vault's audit metadata.
///
/// Recorded so a user can answer "which sites did this snapshot carry sessions for?"
/// without the snapshot being decrypted — and the values themselves are never part
/// of that record.
pub(super) fn cookie_domains(cookies: &[Value]) -> Vec<String> {
    let mut domains: Vec<String> = cookies
        .iter()
        .filter_map(|c| c.get("domain").and_then(Value::as_str))
        .map(|d| d.trim_start_matches('.').to_ascii_lowercase())
        .collect();
    domains.sort();
    domains.dedup();
    domains
}

/// `save_cookies` — snapshot the live tab's cookies into the encrypted vault.
///
/// Previously this wrote plaintext JSON at 0600. Permissions keep out other users;
/// they do not keep out anything running as this user, nor a backup that copies the
/// home directory somewhere else. Cookies are credentials, so they are encrypted with
/// a key from the OS credential store.
pub async fn save_cookies(client: &CdpClient) -> Result<usize, CdpError> {
    let cookies = get_all_cookies(client).await?;
    let json_str = serde_json::to_string(&cookies).unwrap_or_else(|_| "[]".into());
    crate::vault::seal(
        &cookies_vault_path(),
        &json_str,
        &cookie_domains(&cookies),
        crate::vault::default_ttl_secs(),
    )
    .map_err(|e| CdpError::Closed(format!("save_cookies: {e}")))?;
    // A leftover plaintext snapshot from an older version would be the weakest link,
    // so migrating means destroying it, not just stopping writing to it.
    let legacy = cookies_path();
    if legacy.exists() {
        let _ = crate::vault::revoke(&legacy);
        tracing::info!("migrated a plaintext cookie snapshot into the vault and destroyed it");
    }
    Ok(cookies.len())
}

/// `restore_cookies` — inject a saved snapshot into the tab via `Network.setCookies`.
///
/// Reads the vault first, then falls back to a legacy plaintext file so an existing
/// install keeps working across the upgrade. An expired vault entry yields zero
/// cookies rather than stale ones.
pub async fn restore_cookies(client: &CdpClient) -> Result<usize, CdpError> {
    let text = match crate::vault::open(&cookies_vault_path()) {
        Ok(t) => t,
        Err(crate::vault::VaultError::Expired { expired_at, .. }) => {
            tracing::warn!(
                expired_at,
                "session snapshot has expired; not restoring. Log in again, or raise \
                 NEOBROWSER_SESSION_TTL_DAYS"
            );
            return Ok(0);
        }
        Err(_) => {
            // Legacy plaintext path, for one upgrade cycle.
            let legacy = cookies_path();
            match std::fs::read_to_string(&legacy) {
                Ok(t) => {
                    tracing::warn!(
                        "restored from a legacy PLAINTEXT cookie snapshot; run save_cookies \
                         to move it into the encrypted vault"
                    );
                    t
                }
                Err(_) => return Ok(0),
            }
        }
    };
    let cookies: Vec<Value> = serde_json::from_str(&text).unwrap_or_default();
    if cookies.is_empty() {
        return Ok(0);
    }
    client
        .send("Network.setCookies", json!({ "cookies": cookies }))
        .await?;
    Ok(cookies.len())
}

/// `revoke_session` — destroy this profile's stored session material, verifiably.
pub fn revoke_session() -> Result<Value, CdpError> {
    let mut destroyed = Vec::new();
    let mut remaining = Vec::new();
    let dir = session_dir();
    // Must list every path any version of `save_*` may have written, vault and
    // legacy plaintext alike. An earlier revision of this list named only the old
    // `.json` files, so it destroyed the manifest, left the `.vault` files sitting
    // there, and still reported `ok: true` — a deletion that did not happen.
    let targets = [
        cookies_vault_path(),
        cookies_path(),
        crate::vault::vault_path(&dir, "cookies"),
        crate::vault::vault_path(&dir, "localStorage"),
        dir.join("cookies.json"),
        dir.join("localStorage.json"),
        dir.join("manifest.json"),
    ];
    for t in targets {
        if !t.exists() {
            continue;
        }
        let label = t.display().to_string();
        match crate::vault::revoke(&t) {
            Ok(()) if crate::vault::is_revoked(&t) => destroyed.push(label),
            // Reported rather than swallowed: a deletion that did not happen must not
            // be presented as one.
            _ => remaining.push(label),
        }
    }
    Ok(json!({
        "ok": remaining.is_empty(),
        "destroyed": destroyed,
        "remaining": remaining,
    }))
}
