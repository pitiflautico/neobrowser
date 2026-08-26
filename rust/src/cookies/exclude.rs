//! The cookies that are deliberately not imported, and why.
//!
//! Importing a real Chrome profile means importing whatever is in it, including the cookies
//! that *are* the user's identity at Google, GitHub, Slack. Those are held back by default:
//! the automation only needs to be logged into the site under test, and a leaked trace or a
//! prompt-injected page reaching a full Google session is a categorically worse outcome than
//! a failed login.
//!
//! Matching is label-aware — exact host or a dotted suffix, never a prefix — because
//! `ends_with("google.com")` also matches `notgoogle.com`, and a matcher that fails open
//! here silently imports the thing the list exists to hold back.

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

/// Session-identity cookies that must NOT be injected into the ghost browser:
/// major providers log the real browser out when they detect a duplicate
/// session. Preference/consent cookies for the same domains are safe and kept.
/// `(domain suffixes, auth cookie names)`.
const SESSION_AUTH_EXCLUSIONS: &[(&[&str], &[&str])] = &[
    (
        &[
            ".google.com",
            ".google.es",
            ".googleapis.com",
            ".gstatic.com",
            ".youtube.com",
            ".accounts.google.com",
            ".gmail.com",
        ],
        &[
            // Classic Google session cookies.
            "SID",
            "HSID",
            "SSID",
            "APISID",
            "SAPISID",
            "__Secure-1PSID",
            "__Secure-3PSID",
            "__Secure-1PAPISID",
            "__Secure-3PAPISID",
            "__Secure-1PSIDCC",
            "__Secure-3PSIDCC",
            "__Secure-1PSIDTS",
            "__Secure-3PSIDTS",
            "SIDCC",
            "LSID",
            // Gmail / Google Account tokens that also authenticate the user.
            // Cloning these into a headless browser is the fastest way to get the
            // real browser logged out by Google's anti-session-cloning defences.
            "OSID",
            "GMAIL_AT",
            "GAUSR",
            "ACCOUNT_CHOOSER",
            "SMSV",
            "COMPASS",
            "SNID",
            "__Host-GAPS",
            "__Host-3PLSID",
            "__Host-GMAIL_RTT",
            "__Host-GMAIL_SUA",
            "__Host-GMAIL_IMP",
            "GMAIL_RTT",
            "GMAIL_SUA",
            "GMAIL_IMP",
            "GMAIL_LOGIN",
        ],
    ),
    (
        &[".linkedin.com"],
        &["li_at", "JSESSIONID", "li_rm", "liap", "sl"],
    ),
    (
        &[
            ".login.microsoftonline.com",
            ".login.live.com",
            ".login.windows.net",
            ".microsoftonline.com",
        ],
        &[
            "ESTSAUTH",
            "ESTSAUTHPERSISTENT",
            "ESTSAUTHLIGHT",
            "buid",
            "MSPRequ",
            "MSCC",
            "PPAuth",
            "WLSSC",
            "MSPOK",
            "MSFPC",
            "OParams",
            "LOpt",
            "CLIM",
        ],
    ),
    // Additional providers that aggressively invalidate duplicate sessions.
    (
        &[".github.com"],
        &[
            "user_session",
            "__Host-user_session_same_site",
            "_device_id",
            "has_recent_activity",
        ],
    ),
    (
        &[".twitter.com", ".x.com"],
        &[
            "auth_token",
            "ct0",
            "twid",
            "kdt",
            "des_opt_in",
            "twll",
            "dnt",
        ],
    ),
    (
        &[".reddit.com"],
        &[
            "reddit_session",
            "token_v2",
            "session_tracker",
            "recent_srdi",
        ],
    ),
    (&[".facebook.com"], &["c_user", "xs", "fr", "sb", "datr"]),
    (
        &[".instagram.com"],
        &["sessionid", "ds_user_id", "csrftoken"],
    ),
    (&[".slack.com"], &["d", "d-s"]),
    (&[".discord.com"], &["token", "__dcfduid", "__sdcfduid"]),
    // More providers that aggressively revoke duplicate / cloned sessions.
    (
        &[".notion.so", ".notion.site"],
        &[
            "token_v2",
            "notion_user_id",
            "notion_experiment_device_id",
            "notion_device_id",
        ],
    ),
    (
        &[".figma.com"],
        &["figma.auth", "figma.session", "figma.fs", "figma_user"],
    ),
    (&[".linear.app"], &["linear", "linear_session"]),
    (
        &[".vercel.com"],
        &["auth-token", "authorization", "session"],
    ),
    (
        &[".cloudflare.com"],
        &["cftoken", "cf_clearance", "session"],
    ),
    (&[".stripe.com"], &["session", "stripe.csrf"]),
    (&[".dropbox.com"], &["bjarne", "jar", "t", "uc_session"]),
    (
        &[".apple.com", ".icloud.com"],
        &["aa", "acn01", "Des", "csa"],
    ),
    (
        &[".amazon.com", ".amazon.es", ".amazon.co.uk"],
        &[
            "session-id",
            "ubid-main",
            "at-main",
            "sess-at-main",
            "x-main",
        ],
    ),
    (&[".spotify.com"], &["sp_dc", "sp_key"]),
    (&[".zoom.us"], &["_zm_ssid", "_zm_csp"]),
    (
        &[".atlassian.net", ".atlassian.com", ".jira.com"],
        &["ajs_user_id", "tenant.session", "cloud.session.token"],
    ),
    (&[".gitlab.com"], &["_gitlab_session", "known_sign_in"]),
    (&[".bitbucket.org"], &["csrftoken", "sessionid"]),
];

/// Does a cookie's `host_key` fall under a domain from the exclusion list?
///
/// The leading dot has to be normalised on both sides, and getting this wrong fails in the
/// dangerous direction. Chrome stores a *domain* cookie as `.google.com` and a *host-only*
/// cookie as `google.com`. The exclusion entries are written with the dot, so a bare
/// `host_key.ends_with(".google.com")` misses the host-only form entirely — a `SID` cookie
/// scoped to exactly `google.com` would sail through the exclusion and be injected into the
/// agent's browser, which is the specific outcome this list exists to prevent.
///
/// Matching is still label-aware: `evilgoogle.com` must not match `google.com`, the same
/// near-miss the policy engine's domain matching has to defend against.
pub fn host_under_domain(host_key: &str, domain: &str) -> bool {
    let host = host_key.trim_start_matches('.').to_ascii_lowercase();
    let dom = domain.trim_start_matches('.').to_ascii_lowercase();
    host == dom || host.ends_with(&format!(".{dom}"))
}

/// True if this cookie is a session-identity cookie we must not inject.
///
/// Escape hatch: `NEOBROWSER_INCLUDE_IDENTITY_COOKIES=1` disables the exclusion,
/// injecting Google/LinkedIn/Microsoft session-identity cookies too. Risk: the
/// provider may flag the duplicate session and log the real browser out. Opt-in,
/// use sparingly (e.g. one automated action per day), never the default.
pub fn is_session_auth_excluded(host_key: &str, name: &str) -> bool {
    if identity_cookies_opt_in() {
        return false;
    }
    SESSION_AUTH_EXCLUSIONS.iter().any(|(domains, names)| {
        names.contains(&name) && domains.iter().any(|d| host_under_domain(host_key, d))
    })
}

/// Names of cookies that are not classic "auth" tokens but are still used by
/// providers to bind a session to a specific browser / device fingerprint.
/// Importing them into a headless clone can trigger the same "new device"
/// revocation that importing SID/li_at does, so they are held back by default.
const HIGH_RISK_FINGERPRINT_COOKIES: &[(&[&str], &[&str])] = &[(
    &[
        ".google.com",
        ".google.es",
        ".googleapis.com",
        ".gstatic.com",
        ".youtube.com",
        ".accounts.google.com",
        ".gmail.com",
    ],
    &[
        // Security / anti-fraud / device-binding cookies. They are not
        // identity tokens by themselves, but Google treats their sudden use
        // from a headless browser as evidence of session cloning.
        "AEC",
        "SOCS",
        "CONSENT",
        "1P_JAR",
        "DV",
        "OTZ",
        "SEARCH_SAMESITE",
        "OGPC",
        "GAPS",
    ],
)];

/// True for fingerprint / device-binding cookies that providers use to detect
/// cloned sessions. Held back by default; `NEOBROWSER_INCLUDE_IDENTITY_COOKIES=1`
/// disables this check as well, since the user has explicitly accepted the risk.
pub fn is_high_risk_fingerprint_cookie(host_key: &str, name: &str) -> bool {
    if identity_cookies_opt_in() {
        return false;
    }
    HIGH_RISK_FINGERPRINT_COOKIES
        .iter()
        .any(|(domains, names)| {
            names.contains(&name) && domains.iter().any(|d| host_under_domain(host_key, d))
        })
}

/// True only for an explicit affirmative value.
///
/// Presence alone must NOT be enough: with a bare `is_some()`, spelling out
/// `NEOBROWSER_INCLUDE_IDENTITY_COOKIES=0` to disable the escape hatch would
/// switch it ON — silently injecting the identity cookies the exclusion list
/// exists to hold back, and risking a logout of the user's real browser.
fn identity_cookies_opt_in() -> bool {
    explicit_affirmative(std::env::var("NEOBROWSER_INCLUDE_IDENTITY_COOKIES").ok())
}

/// Session (non-persistent) cookies are excluded by default from real-profile
/// import. They are the live session tokens most likely to trigger "new device"
/// revocation on the user's real browser when cloned into a headless profile.
///
/// Persistent cookies (with a future `expires`) are still imported: they carry
/// "remember me" state and are less likely to be treated as a suspicious
/// duplicate session.
///
/// Escape hatch: `NEOBROWSER_IMPORT_SESSION_COOKIES=1` restores the previous
/// behaviour. Use sparingly, and expect providers to log the real browser out.
pub fn is_session_cookie_excluded(expires_unix: i64) -> bool {
    if explicit_affirmative(std::env::var("NEOBROWSER_IMPORT_SESSION_COOKIES").ok()) {
        return false;
    }
    // Chrome stores session cookies as expires_utc == 0. Anything without a
    // concrete future expiry is treated as a session token here.
    expires_unix <= 0
}

fn explicit_affirmative(value: Option<String>) -> bool {
    match value {
        Some(v) => matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        None => false,
    }
}
