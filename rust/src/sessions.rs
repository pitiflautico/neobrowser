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
///
/// Create-then-chmod is not good enough here. `fs::write` followed by
/// `set_permissions` leaves the file world-readable (whatever the umask allows)
/// for the window between the two calls — and what is in it during that window is
/// session cookies. So the file is created 0600 from its first byte, written under
/// a temporary name, and renamed into place: readers see either the old file or
/// the complete new one, never a half-written or briefly-public one.
pub fn write_private(path: &std::path::Path, data: &str) -> std::io::Result<()> {
    use std::io::Write;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Same directory as the target, so the rename below stays within one
    // filesystem and is therefore atomic.
    let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(format!(".tmp-{}", std::process::id()));
    let tmp = path.with_file_name(tmp_name);

    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Applied by open(2) itself: the file never exists with wider bits, and
        // create_new refuses to follow a pre-planted symlink at this path.
        opts.mode(0o600);
    }

    let mut file = match opts.open(&tmp) {
        Ok(f) => f,
        // A crashed earlier run can leave its temp file behind; it is ours by
        // construction (the name carries our pid), so replacing it is safe.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            std::fs::remove_file(&tmp)?;
            opts.open(&tmp)?
        }
        Err(e) => return Err(e),
    };

    // Any failure from here on must not leave the temp file lying around.
    let write_result = file
        .write_all(data.as_bytes())
        .and_then(|_| file.sync_all());
    drop(file);
    if let Err(e) = write_result {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// Prove a directory is actually writable, rather than inferring it from permissions.
///
/// Used by `doctor --json`: a mode check can pass while the write still fails
/// (read-only mount, full disk, SIP, a container's overlay), so the only honest
/// answer comes from attempting a write and removing it again.
pub fn probe_writable(dir: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let probe = dir.join(format!(".write-probe-{}", std::process::id()));
    write_private(&probe, "")?;
    std::fs::remove_file(&probe)
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

/// The encrypted store for this profile's cookies. See [`crate::vault`].
fn cookies_vault_path() -> PathBuf {
    crate::vault::vault_path(&paths::cookies_base(), &profile_name())
}

/// Distinct domains present in a cookie list, for the vault's audit metadata.
///
/// Recorded so a user can answer "which sites did this snapshot carry sessions for?"
/// without the snapshot being decrypted — and the values themselves are never part
/// of that record.
fn cookie_domains(cookies: &[Value]) -> Vec<String> {
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

/// The three explicit session modes from the PRD, and what each one risks.
///
/// These existed implicitly as a combination of two environment variables, which meant
/// a user could not straightforwardly answer "is this browser holding my real
/// cookies?" — the most important question about the whole tool. Naming the modes
/// makes that answer a single field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileMode {
    /// Ephemeral profile, no credentials. The safest, and the default.
    Isolated,
    /// A persistent NeoBrowser profile the user logs into once. Credentials live in
    /// the agent's own profile, never copied from the user's browser.
    Agent,
    /// Driving a Chrome the user already has open. Their live session, their
    /// fingerprint, nothing cloned.
    Attached,
    /// Advanced: cookies decrypted out of the user's real Chrome profile.
    ImportedRealProfile,
}

impl ProfileMode {
    pub fn label(self) -> &'static str {
        match self {
            ProfileMode::Isolated => "isolated",
            ProfileMode::Agent => "agent",
            ProfileMode::Attached => "attached",
            ProfileMode::ImportedRealProfile => "imported_real_profile",
        }
    }
}

/// Which mode this session is in.
///
/// Attach is checked first because it is the strongest signal: with an attach port
/// set, NeoBrowser never launches or patches a browser at all, so nothing else about
/// profiles applies.
pub fn profile_mode() -> ProfileMode {
    if std::env::var("NEOBROWSER_ATTACH_PORT")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .is_some()
    {
        return ProfileMode::Attached;
    }
    if std::env::var("NEOBROWSER_REAL_PROFILE")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .is_some()
    {
        return ProfileMode::ImportedRealProfile;
    }
    // A named profile is a deliberate, reusable identity; the unnamed default is
    // treated as isolated because nothing has been logged into it on purpose.
    if std::env::var("NEOBROWSER_PROFILE")
        .ok()
        .filter(|v| !v.trim().is_empty() && v != "default")
        .is_some()
    {
        return ProfileMode::Agent;
    }
    ProfileMode::Isolated
}

/// A plain-language report of the active mode and its consequences.
pub async fn profile_mode_report(browser: &crate::browser::Browser) -> String {
    let mode = profile_mode();
    let (credentials, risk, advice) = match mode {
        ProfileMode::Isolated => (
            "none — this profile has never been logged in",
            "lowest: nothing here can act as you",
            "Set NEOBROWSER_PROFILE=<name> and log in once to keep a session between runs.",
        ),
        ProfileMode::Agent => (
            "whatever has been logged into this NeoBrowser profile",
            "contained: only the accounts you signed into here, and your own browser is untouched",
            "This is the recommended mode for ongoing work.",
        ),
        ProfileMode::Attached => (
            "your live browser session, in place",
            "your real session is being driven, but nothing is copied, so no duplicate-session signal is sent to providers",
            "Close the debug port when you are done; anything that can reach it can drive your browser.",
        ),
        ProfileMode::ImportedRealProfile => (
            "cookies decrypted from your real Chrome profile",
            "highest: a clone of your session now exists in a second browser, which providers may flag, and identity cookies for Google/LinkedIn/Microsoft are excluded so those will still need a login",
            "Prefer `attached` mode, or an agent profile, unless you specifically need a headless browser carrying imported cookies.",
        ),
    };
    let status = browser.status().await;
    json!({
        "mode": mode.label(),
        "credentials": credentials,
        "risk": risk,
        "advice": advice,
        "profile_dir": crate::paths::profile_dir().display().to_string(),
        "vault_available": crate::vault::available(),
        "browser": status,
    })
    .to_string()
}

/// `session_info` — persistence state plus a per-provider coverage report.
///
/// The coverage report exists because the README used to imply "already
/// authenticated everywhere" while the import deliberately excludes
/// Google/LinkedIn/Microsoft identity cookies. An agent needs to know which of those
/// it actually has, so it can log in once instead of looping on an auth wall.
///
/// Vault metadata is read WITHOUT decrypting: freshness and domain coverage are
/// answerable without materialising a single cookie value.
pub fn session_info() -> String {
    let dir = session_dir();
    let manifest_path = dir.join("manifest.json");
    let manifest = std::fs::read_to_string(&manifest_path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok());

    let cookies_vault = cookies_vault_path();
    let vault_meta = crate::vault::inspect(&cookies_vault).ok();
    let domains: Vec<String> = vault_meta
        .as_ref()
        .and_then(|m| m.get("domains"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    json!({
        "profile": profile_name(),
        "session_dir": dir.display().to_string(),
        "cookies_snapshot_exists": cookies_vault.exists() || cookies_path().exists(),
        "cookies_encrypted": cookies_vault.exists(),
        "vault_available": crate::vault::available(),
        "session_exists": manifest_path.exists(),
        "vault": vault_meta,
        "coverage": coverage_report(&domains),
        "manifest": manifest,
    })
    .to_string()
}

/// Providers whose identity cookies are deliberately never cloned, so an agent can be
/// told plainly that it must log into them rather than discovering it at a wall.
const EXCLUDED_PROVIDERS: &[(&str, &str)] = &[
    ("google.com", "Google"),
    ("linkedin.com", "LinkedIn"),
    ("microsoftonline.com", "Microsoft"),
    ("live.com", "Microsoft"),
];

/// Classify what the stored session actually covers.
fn coverage_report(domains: &[String]) -> Value {
    let mut needs_login = Vec::new();
    for (suffix, label) in EXCLUDED_PROVIDERS {
        if domains
            .iter()
            .any(|d| d == suffix || d.ends_with(&format!(".{suffix}")))
        {
            // The domain appears, but its session-identity cookies were filtered out
            // on import, so presence is not proof of an authenticated session.
            if !needs_login.contains(&label.to_string()) {
                needs_login.push(label.to_string());
            }
        }
    }
    let state = if domains.is_empty() {
        "no_session"
    } else if needs_login.is_empty() {
        "authenticated"
    } else {
        "partially_authenticated"
    };
    json!({
        "state": state,
        "domains": domains,
        "identity_excluded_providers_present": needs_login,
        "note": "Session-identity cookies for Google/LinkedIn/Microsoft are never cloned. \
                 If those providers appear above, expect to log in once inside this \
                 profile rather than assuming an active session.",
    })
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

    /// Overwriting a file that is already world-readable must not inherit those
    /// bits: the replacement is a fresh 0600 file renamed over the old one.
    #[test]
    fn write_private_tightens_perms_on_an_existing_public_file() {
        let _g = crate::env_test_guard();
        std::env::set_var("NEOBROWSER_HOME", "/tmp/nb-sessions-test-overwrite");
        let path = paths::cookies_base().join("was-public.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "old").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        }

        write_private(&path, "[\"new\"]").unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[\"new\"]");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "0644 must not survive a rewrite");
        }
        let _ = std::fs::remove_file(&path);
    }

    /// The temp file is an implementation detail and must never survive the call,
    /// or `~/.neobrowser` slowly fills with readable cookie leftovers.
    /// Regression: an earlier `revoke_session` listed only the legacy `.json`
    /// names, so it left the `.vault` files on disk and still reported `ok: true`.
    /// Deletion that does not happen must never be reported as success.
    #[test]
    fn revoke_session_destroys_vault_files_too() {
        let _g = crate::env_test_guard();
        std::env::set_var("NEOBROWSER_HOME", "/tmp/nb-revoke-test");
        std::env::set_var(
            "NEOBROWSER_VAULT_KEY",
            "dGVzdC12YXVsdC1rZXktdGhpcnR5LXR3by1ieXRlcyE=",
        );
        let dir = session_dir();
        std::fs::create_dir_all(&dir).unwrap();

        // Everything save_cookies / save_session can produce.
        let written = [
            cookies_vault_path(),
            crate::vault::vault_path(&dir, "cookies"),
            crate::vault::vault_path(&dir, "localStorage"),
        ];
        for p in &written {
            crate::vault::seal(p, "[]", &[], None).unwrap();
        }
        write_private(&dir.join("manifest.json"), "{}").unwrap();

        let report = revoke_session().unwrap();
        assert_eq!(report["ok"], true, "report: {report}");
        assert_eq!(report["remaining"].as_array().unwrap().len(), 0);
        for p in &written {
            assert!(crate::vault::is_revoked(p), "left behind: {}", p.display());
        }
        std::env::remove_var("NEOBROWSER_VAULT_KEY");
        let _ = std::fs::remove_dir_all("/tmp/nb-revoke-test");
    }

    #[test]
    fn coverage_report_flags_providers_whose_identity_cookies_are_excluded() {
        // A Google domain in the jar is NOT proof of an authenticated Google session,
        // because the identity cookies are filtered on import.
        let r = coverage_report(&["mail.google.com".into(), "example.com".into()]);
        assert_eq!(r["state"], "partially_authenticated");
        assert_eq!(r["identity_excluded_providers_present"][0], "Google");

        let r = coverage_report(&["example.com".into()]);
        assert_eq!(r["state"], "authenticated");
        assert_eq!(
            r["identity_excluded_providers_present"]
                .as_array()
                .unwrap()
                .len(),
            0
        );

        assert_eq!(coverage_report(&[])["state"], "no_session");
    }

    #[test]
    fn write_private_leaves_no_temp_file_behind() {
        let _g = crate::env_test_guard();
        std::env::set_var("NEOBROWSER_HOME", "/tmp/nb-sessions-test-tmp");
        let dir = paths::cookies_base();
        let path = dir.join("clean.json");
        write_private(&path, "[]").unwrap();
        // Twice, to also cover the path where a previous temp name is reused.
        write_private(&path, "[]").unwrap();

        let leftovers: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "temp files left: {leftovers:?}");
        let _ = std::fs::remove_file(&path);
    }
}
