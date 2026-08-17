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

use serde_json::json;

use crate::cdp::{CdpClient, CdpError};
use crate::paths;

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

/// Copy a validated file into a private staging directory and return the staged path.
///
/// This closes a symlink race that validation alone cannot. `resolve_upload_path`
/// canonicalizes and checks the path, but `DOM.setFileInputFiles` makes **Chrome** open it
/// afterwards, by path. Anything able to write in the upload directory can, in that
/// window, replace the file with a symlink to `~/.ssh/id_rsa` — validation saw a real
/// file under an allowed root, and Chrome opens the attacker's target.
///
/// Validating harder does not fix it; the check and the open are in different processes.
/// So the file is copied into a directory only this user can write (`0700` under
/// `NEOBROWSER_HOME`) and Chrome is handed *that* path. The path Chrome opens is one we
/// created and control, so there is nothing to swap.
///
/// The copy is the cost. Bounded by `NEOBROWSER_MAX_UPLOAD_MB`, and the staging directory
/// is cleared per upload so it does not accumulate copies of the user's files.
pub(crate) fn stage_for_upload(validated: &std::path::Path) -> Result<PathBuf, String> {
    let staging = paths::home().join("upload-staging");
    // Recreated each time: a stale copy of a previous upload sitting in NEOBROWSER_HOME is
    // a small data-retention problem with no upside.
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| format!("staging dir: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // 0700: the staged copy is the user's file, and the directory must not be
        // traversable by anyone else while it exists.
        let _ = std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o700));
    }

    // Opened BEFORE the size check and the copy, so both act on the same file handle
    // rather than re-resolving the path and re-opening what may by then be different.
    let mut source = std::fs::File::open(validated)
        .map_err(|e| format!("cannot open {}: {e}", validated.display()))?;
    let size = source
        .metadata()
        .map_err(|e| format!("cannot stat {}: {e}", validated.display()))?
        .len();
    let cap = upload_size_cap();
    if size > cap {
        return Err(format!(
            "{} is {} MiB, over the {} MiB upload cap. Raise NEOBROWSER_MAX_UPLOAD_MB if              this is expected",
            validated.display(),
            size / (1024 * 1024),
            cap / (1024 * 1024)
        ));
    }

    let name = validated
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "upload".into());
    let staged = staging.join(&name);
    let mut dest = std::fs::File::create(&staged).map_err(|e| format!("staging copy: {e}"))?;
    std::io::copy(&mut source, &mut dest).map_err(|e| format!("staging copy: {e}"))?;
    Ok(staged)
}

/// `upload` — attach local files to a file input via `DOM.setFileInputFiles`.
///
/// Security: files must live under an allowed root (see `upload_allowed_roots`) and
/// must not be sensitive (see `is_sensitive_upload`), so a prompt-injected agent
/// cannot exfiltrate arbitrary local files (ssh keys, credentials, cookie stores…).
pub async fn upload(
    client: &CdpClient,
    selector: &str,
    files: Vec<String>,
) -> Result<String, CdpError> {
    // Validate, then STAGE. Handing Chrome the user's original path would leave a window
    // in which that path can be swapped for a symlink before Chrome opens it; the staged
    // copy lives in a directory only we write to. See `stage_for_upload`.
    let mut abs: Vec<String> = Vec::with_capacity(files.len());
    for f in &files {
        let validated = match resolve_upload_path(f) {
            Ok(p) => p,
            Err(reason) => return Ok(json!({ "ok": false, "error": reason }).to_string()),
        };
        match stage_for_upload(&validated) {
            Ok(staged) => abs.push(staged.to_string_lossy().into_owned()),
            Err(reason) => return Ok(json!({ "ok": false, "error": reason }).to_string()),
        }
    }
    let doc = client
        .send("DOM.getDocument", json!({ "depth": 0 }))
        .await?;
    let root = doc
        .get("root")
        .and_then(|r| r.get("nodeId"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let q = client
        .send(
            "DOM.querySelector",
            json!({ "nodeId": root, "selector": selector }),
        )
        .await?;
    let node_id = q.get("nodeId").and_then(|v| v.as_i64()).unwrap_or(0);
    if node_id == 0 {
        return Ok(
            json!({ "ok": false, "error": format!("file input not found: {selector}") })
                .to_string(),
        );
    }
    client
        .send(
            "DOM.setFileInputFiles",
            json!({ "files": abs, "nodeId": node_id }),
        )
        .await?;
    Ok(json!({ "ok": true, "uploaded": abs, "selector": selector }).to_string())
}
