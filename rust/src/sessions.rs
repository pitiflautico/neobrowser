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

/// Profile name for on-disk snapshots. `NEOBROWSER_REAL_PROFILE` overrides "default",
/// but only after the same whitelist validation the cookie-decryption path applies
/// (`cookies::real_profile_folder`) — an unvalidated value like `../../x` would
/// otherwise let snapshots escape `~/.neobrowser`.
pub fn profile_name() -> String {
    crate::cookies::real_profile_folder().unwrap_or_else(|| "default".to_string())
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

/// Write a file readable/writable only by the owner (0600 on Unix). Shared with
/// the playbook store, whose files can contain credentials from recorded fills.
pub(crate) fn write_private(path: &std::path::Path, data: &str) -> std::io::Result<()> {
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

    // Submit the form that owns the password field we just filled — NOT the
    // first submit button in the document. Sites commonly ship a sign-in panel
    // in the header alongside the real form in the body, and a document-wide
    // querySelector picks the header one, submitting an empty form.
    page::js(
        client,
        r#"(function() {
            var pw = document.querySelector('input[type=password]');
            var form = pw && pw.form;
            var btn = form
                ? form.querySelector('button[type=submit],input[type=submit]')
                : document.querySelector('button[type=submit],input[type=submit]');
            if (btn) btn.click();
            else if (form) form.submit();
            else { var f = document.querySelector('form'); if (f) f.submit(); }
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

    // A leftover password field is a weak signal on its own: an account or
    // settings page legitimately has "old password" / "new password" inputs,
    // and a hidden sign-in panel keeps one in the DOM forever. Only count a
    // field that is actually VISIBLE.
    let visible_pw = page::js(
        client,
        r#"return (function() {
            return Array.from(document.querySelectorAll('input[type=password]'))
                .some(function(el) {
                    var r = el.getBoundingClientRect();
                    if (r.width === 0 || r.height === 0) return false;
                    var s = getComputedStyle(el);
                    return s.visibility !== 'hidden' && s.display !== 'none';
                });
        })()"#,
    )
    .await?
    .as_bool()
    .unwrap_or(false);

    // Cross-check with navigation: landing somewhere other than the login URL
    // is strong evidence the credentials were accepted.
    let url_unchanged = same_page(url, &final_url);
    let failed = visible_pw && url_unchanged;

    let mut out = json!({
        "ok": !failed,
        "url": final_url,
        "title": title,
        "still_has_password_field": visible_pw,
    });
    // When the signals disagree, say so instead of silently picking one.
    if !failed && visible_pw {
        out["confidence"] = json!("medium");
        out["note"] = json!(
            "navigated away from the login URL, but a visible password field is still \
             present (an account/settings page can legitimately have one) — verify if it matters"
        );
    }
    Ok(out.to_string())
}

/// Same page ignoring query string and fragment, so a `?returnTo=…` or `#` on
/// the post-submit URL doesn't read as a successful navigation.
fn same_page(a: &str, b: &str) -> bool {
    fn base(u: &str) -> &str {
        let u = u.split('#').next().unwrap_or(u);
        let u = u.split('?').next().unwrap_or(u);
        u.trim_end_matches('/')
    }
    base(a) == base(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The login success check compares where we landed against where we
    /// started. A redirect back to the login page carrying `?returnTo=…` is
    /// still the login page — treating it as "navigated away" would report a
    /// failed login as a success.
    #[test]
    fn same_page_ignores_query_and_fragment() {
        assert!(same_page(
            "https://x.com/account/login/",
            "https://x.com/account/login"
        ));
        assert!(same_page(
            "https://x.com/account/login/",
            "https://x.com/account/login/?returnTo=%2Faccount%2Fsettings"
        ));
        assert!(same_page(
            "https://x.com/login",
            "https://x.com/login#error"
        ));
        // A real navigation must NOT look like the same page.
        assert!(!same_page(
            "https://x.com/account/login/",
            "https://x.com/account/settings"
        ));
    }

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
    fn profile_name_rejects_traversal() {
        let _g = crate::env_test_guard();
        std::env::set_var("NEOBROWSER_HOME", "/tmp/nb-sessions-test-traversal");
        for bad in ["../../x", "../foo", "a/b", ".hidden", ""] {
            std::env::set_var("NEOBROWSER_REAL_PROFILE", bad);
            assert_eq!(
                profile_name(),
                "default",
                "{bad:?} must fall back to the default profile"
            );
            assert!(cookies_path().starts_with(paths::cookies_base()));
            assert!(session_dir().starts_with(paths::sessions_base()));
        }
        std::env::remove_var("NEOBROWSER_REAL_PROFILE");
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
