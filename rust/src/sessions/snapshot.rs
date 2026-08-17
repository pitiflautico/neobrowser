//! Capturing local storage alongside cookies, so a restored session is really restored.
//!
//! Cookies alone are often not enough. Plenty of applications keep the half of their session
//! state that matters — the selected workspace, the auth token a SPA reads on boot — in
//! localStorage, and restoring only cookies produces a session that is authenticated and
//! still behaves like a first visit.

//! Cookie/session snapshotting + a scripted login, over CDP.
//!
//! These are the *manual* snapshot paths (`save_cookies`/`restore_cookies`) plus a
//! full session save (cookies + localStorage) and a scripted `login`. They operate
//! on the live tab's cookies via `Network.getCookies`/`Network.setCookies` — no
//! SQLite needed (that is the separate real-profile auto-auth path, which reuses the
//! Phase-5 crypto in `cookies.rs`).

use serde_json::{json, Value};

use super::jar::{cookie_domains, get_all_cookies};
use super::{now_unix, profile_name, session_dir, write_private};
use crate::cdp::{CdpClient, CdpError};
use crate::page;

/// Capture the current origin's localStorage as an array of [key, value] pairs.
async fn get_local_storage(client: &CdpClient) -> Result<Value, CdpError> {
    page::eval_body(
        client,
        "return JSON.stringify(Object.keys(localStorage).map(function(k){return [k, localStorage.getItem(k)];}))",
    )
    .await
    .map(|v| match v {
        Value::String(s) => serde_json::from_str(&s).unwrap_or(Value::Array(vec![])),
        other => other,
    })
}

/// `save_session` — full save: cookies + localStorage + a manifest. Returns stats.
pub async fn save_session(client: &CdpClient) -> Result<String, CdpError> {
    let cookies = get_all_cookies(client).await?;
    let local_storage = get_local_storage(client).await?;
    let dir = session_dir();

    let domains = cookie_domains(&cookies);
    let ttl = crate::vault::default_ttl_secs();

    // Both cookies and localStorage are sealed: localStorage routinely holds bearer
    // tokens, which are no less a credential than a cookie.
    let cookies_json = serde_json::to_string(&cookies).unwrap_or_else(|_| "[]".into());
    crate::vault::seal(
        &crate::vault::vault_path(&dir, "cookies"),
        &cookies_json,
        &domains,
        ttl,
    )
    .map_err(|e| CdpError::Closed(format!("save_session cookies: {e}")))?;
    let ls_json = serde_json::to_string(&local_storage).unwrap_or_else(|_| "[]".into());
    crate::vault::seal(
        &crate::vault::vault_path(&dir, "localStorage"),
        &ls_json,
        &domains,
        ttl,
    )
    .map_err(|e| CdpError::Closed(format!("save_session localStorage: {e}")))?;
    // Destroy any plaintext files an older version left behind.
    for legacy in ["cookies.json", "localStorage.json"] {
        let p = dir.join(legacy);
        if p.exists() {
            let _ = crate::vault::revoke(&p);
        }
    }

    let ls_count = local_storage.as_array().map(|a| a.len()).unwrap_or(0);

    let manifest = json!({
        "profile": profile_name(),
        "cookies": cookies.len(),
        "domains": domains,
        "local_storage_keys": ls_count,
        "saved_at": now_unix(),
    });
    write_private(&dir.join("manifest.json"), &manifest.to_string())
        .map_err(|e| CdpError::Closed(format!("save_session manifest: {e}")))?;
    Ok(manifest.to_string())
}
