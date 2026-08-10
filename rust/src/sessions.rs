//! Cookie/session snapshotting + a scripted login, over CDP.
//!
//! These are the *manual* snapshot paths (`save_cookies`/`restore_cookies`) plus a
//! full session save (cookies + localStorage) and a scripted `login`. They operate
//! on the live tab's cookies via `Network.getCookies`/`Network.setCookies` — no
//! SQLite needed (that is the separate real-profile auto-auth path, which reuses the
//! Phase-5 crypto in `cookies.rs`).

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::cdp::{CdpClient, CdpError};
use crate::page;
use crate::paths;

/// Profile name for on-disk snapshots. `NEOBROWSER_REAL_PROFILE` overrides "default".
pub fn profile_name() -> String {
    std::env::var("NEOBROWSER_REAL_PROFILE")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "default".to_string())
}

fn cookies_path() -> PathBuf {
    paths::cookies_base().join(format!("{}.json", profile_name()))
}

fn session_dir() -> PathBuf {
    paths::sessions_base().join(profile_name())
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Write a file readable/writable only by the owner (0600 on Unix).
fn write_private(path: &std::path::Path, data: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, data)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// All browser cookies via CDP.
async fn get_all_cookies(client: &CdpClient) -> Result<Vec<Value>, CdpError> {
    let result = client.send("Network.getCookies", json!({})).await?;
    Ok(result
        .get("cookies")
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default())
}

/// `save_cookies` — snapshot the live tab's cookies to `~/.neobrowser/cookies/{profile}.json` (0600).
pub async fn save_cookies(client: &CdpClient) -> Result<usize, CdpError> {
    let cookies = get_all_cookies(client).await?;
    let path = cookies_path();
    let json_str = serde_json::to_string_pretty(&cookies).unwrap_or_else(|_| "[]".into());
    write_private(&path, &json_str)
        .map_err(|e| CdpError::Closed(format!("save_cookies write failed: {e}")))?;
    Ok(cookies.len())
}

/// `restore_cookies` — inject a saved snapshot into the tab via `Network.setCookies`.
pub async fn restore_cookies(client: &CdpClient) -> Result<usize, CdpError> {
    let path = cookies_path();
    if !path.exists() {
        return Ok(0);
    }
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return Ok(0),
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

/// Capture the current origin's localStorage as an array of [key, value] pairs.
async fn get_local_storage(client: &CdpClient) -> Result<Value, CdpError> {
    page::js(
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

    let cookies_json = serde_json::to_string_pretty(&cookies).unwrap_or_else(|_| "[]".into());
    write_private(&dir.join("cookies.json"), &cookies_json)
        .map_err(|e| CdpError::Closed(format!("save_session cookies: {e}")))?;
    let ls_json = serde_json::to_string_pretty(&local_storage).unwrap_or_else(|_| "[]".into());
    write_private(&dir.join("localStorage.json"), &ls_json)
        .map_err(|e| CdpError::Closed(format!("save_session localStorage: {e}")))?;

    // Distinct cookie domains for the manifest.
    let mut domains: Vec<String> = cookies
        .iter()
        .filter_map(|c| c.get("domain").and_then(|d| d.as_str()).map(String::from))
        .collect();
    domains.sort();
    domains.dedup();
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

/// `session_info` — persistence state: manifest contents + file existence.
pub fn session_info() -> String {
    let dir = session_dir();
    let manifest_path = dir.join("manifest.json");
    let manifest = std::fs::read_to_string(&manifest_path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok());
    json!({
        "profile": profile_name(),
        "session_dir": dir.display().to_string(),
        "cookies_snapshot_exists": cookies_path().exists(),
        "session_exists": manifest_path.exists(),
        "manifest": manifest,
    })
    .to_string()
}

/// `login` — navigate to an https login page, fill email + password, submit, and
/// report an honest success signal (a lingering password field means it didn't work).
pub async fn login(
    client: &CdpClient,
    url: &str,
    email: &str,
    password: &str,
) -> Result<String, CdpError> {
    if !url.starts_with("https://") {
        return Ok(json!({ "ok": false, "error": "login requires an https:// URL" }).to_string());
    }
    page::navigate(client, url, 3.0).await?;
    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

    let email_js = format!(
        r#"(function() {{
            var el = document.querySelector('input[type=email],input[name=email],input[name=username],input[id*=email],input[id*=user]');
            if (!el) return;
            var setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value');
            if (setter && setter.set) setter.set.call(el, {v}); else el.value = {v};
            el.dispatchEvent(new Event('input', {{bubbles:true}}));
            el.dispatchEvent(new Event('change', {{bubbles:true}}));
        }})()"#,
        v = serde_json::to_string(email).unwrap()
    );
    page::js(client, &email_js).await?;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let pw_js = format!(
        r#"(function() {{
            var el = document.querySelector('input[type=password]');
            if (!el) return;
            var setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value');
            if (setter && setter.set) setter.set.call(el, {v}); else el.value = {v};
            el.dispatchEvent(new Event('input', {{bubbles:true}}));
            el.dispatchEvent(new Event('change', {{bubbles:true}}));
        }})()"#,
        v = serde_json::to_string(password).unwrap()
    );
    page::js(client, &pw_js).await?;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    page::js(
        client,
        r#"(function() {
            var btn = document.querySelector('button[type=submit],input[type=submit]');
            if (btn) btn.click();
            else { var f = document.querySelector('form'); if(f) f.submit(); }
        })()"#,
    )
    .await?;
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    let final_url = page::current_url(client).await.unwrap_or_default();
    let title = page::js(client, "return document.title")
        .await?
        .as_str()
        .unwrap_or("")
        .to_string();
    let still_login = page::js(
        client,
        "return !!document.querySelector('input[type=password]')",
    )
    .await?
    .as_bool()
    .unwrap_or(false);
    Ok(json!({
        "ok": !still_login,
        "url": final_url,
        "title": title,
        "still_has_password_field": still_login,
    })
    .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_name_defaults() {
        let _g = crate::env_test_guard();
        let prev = std::env::var("NEOBROWSER_REAL_PROFILE").ok();
        std::env::remove_var("NEOBROWSER_REAL_PROFILE");
        assert_eq!(profile_name(), "default");
        std::env::set_var("NEOBROWSER_REAL_PROFILE", "Profile 1");
        assert_eq!(profile_name(), "Profile 1");
        match prev {
            Some(v) => std::env::set_var("NEOBROWSER_REAL_PROFILE", v),
            None => std::env::remove_var("NEOBROWSER_REAL_PROFILE"),
        }
    }

    #[test]
    fn session_info_reports_absence_cleanly() {
        let _g = crate::env_test_guard();
        std::env::set_var("NEOBROWSER_HOME", "/tmp/nb-sessions-test-absent");
        std::env::set_var("NEOBROWSER_REAL_PROFILE", "nobody");
        let info: Value = serde_json::from_str(&session_info()).unwrap();
        assert_eq!(info["session_exists"], false);
        assert_eq!(info["manifest"], Value::Null);
        std::env::remove_var("NEOBROWSER_REAL_PROFILE");
    }

    #[test]
    fn write_private_sets_owner_only_perms() {
        let _g = crate::env_test_guard();
        std::env::set_var("NEOBROWSER_HOME", "/tmp/nb-sessions-test-perms");
        let path = paths::cookies_base().join("perm-test.json");
        write_private(&path, "[]").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let _ = std::fs::remove_file(&path);
    }
}
