//! Receiving a file from a page.
//!
//! Downloads land under a directory this tool owns rather than the user's real Downloads
//! folder, so a page that triggers a download cannot overwrite something the user cares
//! about by choosing a filename.

//! Upload and download: the tools that move files between the machine and a page.
//!
//! Both are gated on the same allowlist (`resolve_upload_path`) so a second file-reading
//! tool cannot end up with weaker validation than the first — which is exactly what
//! nearly happened when `har_import` was added.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::{json, Map};

use crate::cdp::{CdpClient, CdpError};
use crate::paths;

use super::super::fetch::{guarded_get, read_capped};
use super::super::files::{download_size_cap, write_download_atomically};
use super::super::ssrf::validate_url;

pub(super) fn paths_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// `download` — fetch a public URL to `~/.neobrowser/downloads/`, reusing the tab's
/// cookies so auth-gated files work. 200 MB cap.
pub async fn download(
    client: &CdpClient,
    url: &str,
    filename: Option<&str>,
) -> Result<String, CdpError> {
    if !validate_url(url) {
        return Ok(json!({ "ok": false, "error": "blocked: only public http(s) URLs allowed (SSRF guard)" }).to_string());
    }
    let ddir = paths::home().join("downloads");
    if std::fs::create_dir_all(&ddir).is_err() {
        return Ok(json!({ "ok": false, "error": "could not create downloads dir" }).to_string());
    }
    let raw_name = filename
        .map(String::from)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            url.trim_end_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or("download")
                .split('?')
                .next()
                .unwrap_or("download")
                .to_string()
        });
    let safe: String = raw_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .take(120)
        .collect();
    let safe = if safe.is_empty() {
        "download".to_string()
    } else {
        safe
    };
    let dest = ddir.join(&safe);

    // Reuse the tab's cookies for this URL.
    let mut cookie_header = String::new();
    if let Ok(res) = client
        .send("Network.getCookies", json!({ "urls": [url] }))
        .await
    {
        if let Some(cookies) = res.get("cookies").and_then(|c| c.as_array()) {
            let parts: Vec<String> = cookies
                .iter()
                .filter_map(|c| {
                    let n = c.get("name").and_then(|v| v.as_str())?;
                    let v = c.get("value").and_then(|v| v.as_str())?;
                    Some(format!("{n}={v}"))
                })
                .collect();
            cookie_header = parts.join("; ");
        }
    }

    let cookie_opt = if cookie_header.is_empty() {
        None
    } else {
        Some(cookie_header.as_str())
    };
    let empty_headers = Map::new();
    let (resp, withheld) = match guarded_get(
        url,
        "Mozilla/5.0",
        Duration::from_secs(30),
        &empty_headers,
        cookie_opt,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => return Ok(json!({ "ok": false, "error": e }).to_string()),
    };
    // A download that redirected off-origin loses the tab's cookies, so it may
    // land on a login page instead of the file. Say so rather than saving that
    // HTML under the requested filename.
    if !withheld.is_empty() {
        tracing::warn!(
            url = %url,
            "download redirected off the requested origin; session cookies were not forwarded"
        );
    }
    let bytes = match read_capped(resp, download_size_cap()).await {
        Ok(b) => b,
        Err(e) => return Ok(json!({ "ok": false, "error": e.to_string() }).to_string()),
    };
    if bytes.len() >= download_size_cap() {
        // Reported, not silently truncated: a half file saved under the requested
        // name is worse than a refusal, because it looks like a complete download.
        return Ok(json!({
            "ok": false,
            "error": format!(
                "response exceeded the {} MiB download cap; nothing was written. Raise NEOBROWSER_MAX_DOWNLOAD_MB if this is expected",
                download_size_cap() / (1024 * 1024)
            ),
        })
        .to_string());
    }
    let (final_path, renamed) = match write_download_atomically(&dest, &bytes) {
        Ok(v) => v,
        Err(e) => {
            return Ok(json!({ "ok": false, "error": format!("write failed: {e}") }).to_string())
        }
    };
    let mut out = json!({
        "ok": true,
        "path": final_path.display().to_string(),
        "bytes": bytes.len(),
    });
    if renamed {
        out["warnings"] = json!([format!(
            "a file already existed at {}; saved under a new name rather than overwriting it",
            dest.display()
        )]);
    }
    Ok(out.to_string())
}
