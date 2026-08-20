//! Sending a local file into a page, and refusing the ones that should never leave.
//!
//! Two defences that are easy to get wrong. The allow-list is checked against the
//! *canonicalised* path, so `~/project/../.ssh/id_rsa` does not slip through as
//! "under the project root". And the validated file is copied into a private staging
//! directory before Chrome is told about it, because validating a path and then handing that
//! same path to another process is a time-of-check/time-of-use race: a symlink swapped in
//! between the two makes the check meaningless.

//! Upload and download: the tools that move files between the machine and a page.
//!
//! Both are gated on the same allowlist (`resolve_upload_path`) so a second file-reading
//! tool cannot end up with weaker validation than the first — which is exactly what
//! nearly happened when `har_import` was added.

use std::path::PathBuf;

use super::super::files::mcp_roots;
use super::download::paths_home;

/// Directories `upload` may read from. If `NEOBROWSER_UPLOAD_DIR` is set, ONLY that
/// directory is allowed (tightest, recommended for autonomous agents). Otherwise a
/// safe default set of user content folders.
/// The upload roots, as display strings for `doctor --json`.
///
/// Exposed separately so the report shows the same list `upload` enforces, instead of
/// documentation that can drift away from the code.
pub fn upload_roots_for_report() -> Vec<String> {
    upload_allowed_roots()
        .iter()
        .map(|p| p.display().to_string())
        .collect()
}

fn upload_allowed_roots() -> Vec<PathBuf> {
    // Explicit configuration beats everything: an operator who named one directory
    // means that directory.
    if let Some(dir) = std::env::var_os("NEOBROWSER_UPLOAD_DIR") {
        if !dir.is_empty() {
            let p = PathBuf::from(dir);
            return vec![std::fs::canonicalize(&p).unwrap_or(p)];
        }
    }
    // Then the client's declared MCP roots — narrower and more accurate than the
    // guessed defaults below, because the user chose them.
    let roots = mcp_roots();
    if !roots.is_empty() {
        return roots.to_vec();
    }
    let home = paths_home();
    ["Downloads", "Desktop", "Documents"]
        .iter()
        .map(|d| home.join(d))
        .chain(std::iter::once(crate::paths::home().join("downloads")))
        .map(|p| std::fs::canonicalize(&p).unwrap_or(p))
        .collect()
}

/// True for paths that must never be uploaded even from an allowed root — secrets,
/// keys, keychains, credential files, and NeoBrowser's own cookie/session store.
/// This is the defense against a prompt-injected agent exfiltrating local secrets.
pub(crate) fn is_sensitive_upload(canonical: &std::path::Path) -> bool {
    let s = canonical.to_string_lossy().to_lowercase();
    const DENY_SEGMENTS: &[&str] = &[
        "/.ssh/",
        "/.aws/",
        "/.gnupg/",
        "/.gpg/",
        "/.kube/",
        "/.docker/",
        "/.config/gcloud/",
        "/library/keychains/",
        "/.mozilla/",
        "/.password-store/",
    ];
    if DENY_SEGMENTS.iter().any(|seg| s.contains(seg)) {
        return true;
    }
    // NeoBrowser's own secret store (cookies / sessions / profiles).
    let nb = crate::paths::home().to_string_lossy().to_lowercase();
    for sub in ["/cookies", "/sessions", "/profiles"] {
        if s.starts_with(&format!("{nb}{sub}")) {
            return true;
        }
    }
    let name = canonical
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    const DENY_NAMES: &[&str] = &[
        "id_rsa",
        "id_dsa",
        "id_ecdsa",
        "id_ed25519",
        "credentials",
        ".env",
        ".netrc",
        ".pgpass",
        ".git-credentials",
        ".npmrc",
        ".pypirc",
    ];
    if DENY_NAMES.contains(&name.as_str()) {
        return true;
    }
    let ext = canonical
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    matches!(
        ext.as_str(),
        "pem" | "key" | "p12" | "pfx" | "keychain" | "kdbx"
    )
}

/// Resolve a requested upload path, or return a reason it is rejected.
/// Resolve a caller-supplied file path against the allowed roots.
///
/// Public so any tool that reads a local file goes through the SAME check. A second
/// path-reading tool with its own validation is how one of them ends up weaker.
pub fn resolve_upload_path(f: &str) -> Result<PathBuf, String> {
    let expanded = if let Some(rest) = f.strip_prefix("~/") {
        paths_home().join(rest)
    } else {
        PathBuf::from(f)
    };
    let canonical = std::fs::canonicalize(&expanded).map_err(|_| format!("file not found: {f}"))?;
    if is_sensitive_upload(&canonical) {
        return Err(format!("refused (sensitive path): {f}"));
    }
    let roots = upload_allowed_roots();
    if !roots.iter().any(|r| canonical.starts_with(r)) {
        return Err(format!(
            "refused (outside allowed upload dirs): {f}. Allowed: {}. Set NEOBROWSER_UPLOAD_DIR to widen.",
            roots.iter().map(|r| r.display().to_string()).collect::<Vec<_>>().join(", ")
        ));
    }
    Ok(canonical)
}

/// Maximum size of a file `upload` will stage. Bounded because staging copies it.
pub(crate) fn upload_size_cap() -> u64 {
    std::env::var("NEOBROWSER_MAX_UPLOAD_MB")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|m| *m > 0)
        .unwrap_or(100)
        * 1024
        * 1024
}
