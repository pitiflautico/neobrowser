//! Cookie/session snapshotting + a scripted login, over CDP.
//!
//! These are the *manual* snapshot paths (`save_cookies`/`restore_cookies`) plus a
//! full session save (cookies + localStorage) and a scripted `login`. They operate
//! on the live tab's cookies via `Network.getCookies`/`Network.setCookies` — no
//! SQLite needed (that is the separate real-profile auto-auth path, which reuses the
//! Phase-5 crypto in `cookies.rs`).
//!
//! Split by what each part is responsible for: [`jar`] persists cookies, [`snapshot`] adds
//! the local-storage half of a session, [`profile`] reports which profile is in use and what
//! was held back, and [`mod@login`] drives an interactive sign-in. The paths and the private
//! write helper stay here, because every part needs them.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::paths;

pub mod jar;
pub mod login;
pub mod profile;
pub mod snapshot;

pub use jar::{restore_cookies, revoke_session, save_cookies};
pub use login::login;
pub use profile::{profile_mode, profile_mode_report, session_info, ProfileMode};
pub use snapshot::save_session;

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

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::jar::cookies_vault_path;
    use super::login::same_page;
    use super::profile::coverage_report;
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
