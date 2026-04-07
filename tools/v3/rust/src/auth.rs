//! Auth & Session Management for NeoBrowser.
//!
//! Architecture:
//!   - Profiles: named auth identities ("linkedin-work", "gemini-personal")
//!   - Secrets: OS keychain via `keyring`, never exposed to LLM
//!   - Sessions: cookies + metadata persisted per domain
//!   - TOTP: seeds in keychain, codes generated programmatically
//!   - Challenges: structured 2FA handoff (agent ↔ user ↔ browser)

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ─── Profile ───

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthProfile {
    pub profile_id: String,
    pub display_name: String,
    pub domains: Vec<String>,
    pub login_url: Option<String>,
    pub username_field: Option<String>,  // CSS selector or field name hint
    pub password_field: Option<String>,
    pub totp_enabled: bool,
    pub preferred_backend: SessionBackend,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionBackend {
    ManagedCookies,     // Our cookie store (~/.neobrowser/sessions/)
    ChromeProfile,      // Real Chrome user_data_dir
}

impl Default for SessionBackend {
    fn default() -> Self {
        Self::ManagedCookies
    }
}

// ─── Stored Session ───

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredSession {
    pub session_id: String,
    pub profile_id: String,
    pub domain: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_verified_at: Option<DateTime<Utc>>,
    pub health: SessionHealth,
    pub cookies: Vec<serde_json::Value>,
    #[serde(default)]
    pub local_storage: std::collections::HashMap<String, String>,
    pub auth_markers: Vec<String>,      // URL patterns that confirm auth
    pub logout_markers: Vec<String>,    // URL patterns that mean logged out
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionHealth {
    pub status: HealthStatus,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum HealthStatus {
    Unknown,
    Valid,
    Expired,
    Invalid,
}

// ─── Auth Challenge (2FA handoff) ───

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthChallenge {
    pub challenge_id: String,
    pub profile_id: String,
    pub domain: String,
    pub challenge_type: ChallengeType,
    pub status: ChallengeStatus,
    pub user_message: String,
    pub target_node_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChallengeType {
    OtpSms,
    OtpEmail,
    Totp,
    MagicLink,
    Captcha,
    SecurityQuestion,
    ApprovalPrompt,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ChallengeStatus {
    Pending,
    Resolved,
    Expired,
    Failed,
}

// ─── Auth State Machine ───

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuthState {
    Idle,
    DetectingLogin,
    FillingCredentials,
    SubmittingLogin,
    AwaitingChallenge(AuthChallenge),
    FillingTotp,
    VerifyingSession,
    Authenticated,
    Failed(String),
}

// ─── Secret Store (OS Keychain) ───

const KEYCHAIN_SERVICE: &str = "neobrowser";

pub struct SecretStore;

impl SecretStore {
    /// Store a credential in OS keychain.
    /// Key format: "profile/{profile_id}/{kind}" where kind = username|password|totp_seed
    pub fn set(profile_id: &str, kind: &str, value: &str) -> Result<(), String> {
        let key = format!("profile/{profile_id}/{kind}");
        let entry = keyring::Entry::new(KEYCHAIN_SERVICE, &key)
            .map_err(|e| format!("keyring init: {e}"))?;
        entry
            .set_password(value)
            .map_err(|e| format!("keyring set: {e}"))?;
        Ok(())
    }

    /// Retrieve a credential from OS keychain.
    pub fn get(profile_id: &str, kind: &str) -> Result<Option<String>, String> {
        let key = format!("profile/{profile_id}/{kind}");
        let entry = keyring::Entry::new(KEYCHAIN_SERVICE, &key)
            .map_err(|e| format!("keyring init: {e}"))?;
        match entry.get_password() {
            Ok(val) => Ok(Some(val)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(format!("keyring get: {e}")),
        }
    }

    /// Delete a credential.
    pub fn delete(profile_id: &str, kind: &str) -> Result<(), String> {
        let key = format!("profile/{profile_id}/{kind}");
        let entry = keyring::Entry::new(KEYCHAIN_SERVICE, &key)
            .map_err(|e| format!("keyring init: {e}"))?;
        let _ = entry.delete_credential();
        Ok(())
    }
}

// ─── TOTP Generator ───

pub fn generate_totp(profile_id: &str) -> Result<String, String> {
    let seed = SecretStore::get(profile_id, "totp_seed")?
        .ok_or_else(|| format!("No TOTP seed for profile {profile_id}"))?;

    let totp = totp_rs::TOTP::new(
        totp_rs::Algorithm::SHA1,
        6,
        1,
        30,
        totp_rs::Secret::Encoded(seed)
            .to_bytes()
            .map_err(|e| format!("TOTP decode: {e}"))?,
    )
    .map_err(|e| format!("TOTP init: {e}"))?;

    let code = totp.generate_current().map_err(|e| format!("TOTP gen: {e}"))?;
    Ok(code)
}

// ─── Profile Store (filesystem) ───

fn base_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".neobrowser")
}

fn profiles_dir() -> PathBuf {
    base_dir().join("profiles")
}

fn sessions_dir() -> PathBuf {
    base_dir().join("sessions")
}

pub fn ensure_dirs() -> Result<(), String> {
    for d in [profiles_dir(), sessions_dir()] {
        std::fs::create_dir_all(&d).map_err(|e| format!("mkdir {}: {e}", d.display()))?;
        // Set permissions to 0700 on unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o700);
            std::fs::set_permissions(&d, perms).ok();
        }
    }
    Ok(())
}

// ─── Profile CRUD ───

pub fn save_profile(profile: &AuthProfile) -> Result<(), String> {
    ensure_dirs()?;
    let path = profiles_dir().join(format!("{}.json", profile.profile_id));
    let json = serde_json::to_string_pretty(profile).map_err(|e| format!("{e}"))?;
    std::fs::write(&path, json).map_err(|e| format!("write profile: {e}"))?;
    set_file_permissions(&path);
    Ok(())
}

pub fn load_profile(profile_id: &str) -> Result<Option<AuthProfile>, String> {
    let path = profiles_dir().join(format!("{profile_id}.json"));
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path).map_err(|e| format!("read profile: {e}"))?;
    let profile: AuthProfile = serde_json::from_str(&content).map_err(|e| format!("parse profile: {e}"))?;
    Ok(Some(profile))
}

pub fn list_profiles() -> Result<Vec<AuthProfile>, String> {
    ensure_dirs()?;
    let mut profiles = Vec::new();
    let dir = profiles_dir();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(profile) = serde_json::from_str::<AuthProfile>(&content) {
                        profiles.push(profile);
                    }
                }
            }
        }
    }
    Ok(profiles)
}

pub fn delete_profile(profile_id: &str) -> Result<(), String> {
    let path = profiles_dir().join(format!("{profile_id}.json"));
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| format!("delete profile: {e}"))?;
    }
    // Also delete secrets
    SecretStore::delete(profile_id, "username").ok();
    SecretStore::delete(profile_id, "password").ok();
    SecretStore::delete(profile_id, "totp_seed").ok();
    // Delete sessions
    let sess_dir = sessions_dir().join(profile_id);
    if sess_dir.exists() {
        std::fs::remove_dir_all(&sess_dir).ok();
    }
    Ok(())
}

/// Find which profile matches a given domain.
pub fn find_profile_for_domain(domain: &str) -> Result<Option<AuthProfile>, String> {
    let profiles = list_profiles()?;
    for p in profiles {
        for d in &p.domains {
            if domain == d || domain.ends_with(&format!(".{d}")) {
                return Ok(Some(p));
            }
        }
    }
    Ok(None)
}

// ─── Session CRUD ───

pub fn save_session(session: &StoredSession) -> Result<(), String> {
    ensure_dirs()?;
    let dir = sessions_dir().join(&session.profile_id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir session: {e}"))?;
    let path = dir.join(format!("{}.json", session.domain));
    let json = serde_json::to_string_pretty(session).map_err(|e| format!("{e}"))?;
    std::fs::write(&path, json).map_err(|e| format!("write session: {e}"))?;
    set_file_permissions(&path);
    Ok(())
}

pub fn load_session(profile_id: &str, domain: &str) -> Result<Option<StoredSession>, String> {
    let path = sessions_dir().join(profile_id).join(format!("{domain}.json"));
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path).map_err(|e| format!("read session: {e}"))?;
    let session: StoredSession =
        serde_json::from_str(&content).map_err(|e| format!("parse session: {e}"))?;
    Ok(Some(session))
}

pub fn list_sessions(profile_id: &str) -> Result<Vec<StoredSession>, String> {
    let dir = sessions_dir().join(profile_id);
    let mut sessions = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if let Ok(content) = std::fs::read_to_string(entry.path()) {
                if let Ok(s) = serde_json::from_str::<StoredSession>(&content) {
                    sessions.push(s);
                }
            }
        }
    }
    Ok(sessions)
}

// ─── Cookie Export from Chrome Session ───

/// Export current browser cookies for a domain into a StoredSession.
pub fn create_session_from_cookies(
    profile_id: &str,
    domain: &str,
    cookies: Vec<serde_json::Value>,
    local_storage: std::collections::HashMap<String, String>,
    auth_markers: Vec<String>,
) -> StoredSession {
    let now = Utc::now();
    StoredSession {
        session_id: uuid::Uuid::new_v4().to_string(),
        profile_id: profile_id.to_string(),
        domain: domain.to_string(),
        created_at: now,
        updated_at: now,
        last_verified_at: Some(now),
        health: SessionHealth {
            status: HealthStatus::Valid,
            reason: Some("Fresh login".into()),
        },
        cookies,
        local_storage,
        auth_markers,
        logout_markers: vec![],
    }
}

// ─── Challenge Management ───

pub fn create_challenge(
    profile_id: &str,
    domain: &str,
    challenge_type: ChallengeType,
    user_message: &str,
    target_node_id: Option<String>,
) -> AuthChallenge {
    AuthChallenge {
        challenge_id: uuid::Uuid::new_v4().to_string(),
        profile_id: profile_id.to_string(),
        domain: domain.to_string(),
        challenge_type,
        status: ChallengeStatus::Pending,
        user_message: user_message.to_string(),
        target_node_id,
        created_at: Utc::now(),
        expires_at: Some(Utc::now() + chrono::Duration::minutes(5)),
    }
}

// ─── Helpers ───

fn set_file_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(path, perms).ok();
    }
}
