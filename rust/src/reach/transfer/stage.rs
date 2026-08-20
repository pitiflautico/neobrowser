//! Staging a validated file, and handing it to Chrome.
//!
//! The copy into a private staging directory is the whole point. Validating a path and then
//! letting another process open that same path later is a time-of-check/time-of-use race: a
//! symlink swapped in between the two makes every check above meaningless. So the bytes are
//! copied, under this process's control, into a directory only this user can read — and Chrome
//! is told about the copy.

use std::path::PathBuf;

use serde_json::json;

use crate::cdp::{CdpClient, CdpError};
use crate::paths;

use super::upload::{resolve_upload_path, upload_size_cap};

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
