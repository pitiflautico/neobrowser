//! Which profile is in use, what that means, and which providers were held back.
//!
//! This is the module that has to tell the truth out loud. Whether the real Chrome profile
//! is attached, whether the renderer sandbox is on, and which session-identity cookies were
//! deliberately excluded are all things a user must be able to read back rather than infer —
//! a tool that quietly attached to someone's live profile would be indistinguishable, from
//! the outside, from one that did not.

//! Cookie/session snapshotting + a scripted login, over CDP.
//!
//! These are the *manual* snapshot paths (`save_cookies`/`restore_cookies`) plus a
//! full session save (cookies + localStorage) and a scripted `login`. They operate
//! on the live tab's cookies via `Network.getCookies`/`Network.setCookies` — no
//! SQLite needed (that is the separate real-profile auto-auth path, which reuses the
//! Phase-5 crypto in `cookies.rs`).

use serde_json::{json, Value};

use super::jar::cookies_vault_path;
use super::{cookies_path, profile_name, session_dir};

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
            "persistent cookies decrypted from your real Chrome profile, but only for the domains listed in NEOBROWSER_REAL_PROFILE_DOMAINS",
            "highest: a clone of your session now exists in a second browser, which providers may flag; identity and fingerprint cookies for Google/Gmail, Microsoft, LinkedIn and other high-risk providers are excluded, session (non-persistent) cookies are skipped by default, and import is opt-in per domain to reduce the chance the real browser is logged out",
            "If your real browser gets logged out, stop using real-profile import for that provider and switch to attached mode or a logged-in agent profile.",
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
pub(super) fn coverage_report(domains: &[String]) -> Value {
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
