//! Filesystem boundaries: which directories may be read from and written to.
//!
//! Two concerns that were tangled with the HTTP code: where uploads may come from (MCP
//! roots, or an explicit directory, or a conservative default set), and how a download is
//! written without clobbering an existing file or leaving a partial one behind.

use std::path::PathBuf;

/// MCP roots the client declared at `initialize`, if any.
///
/// A client that speaks MCP roots is telling us which directories the user has actually
/// opened. Honouring that is strictly better than a guessed default set: it means
/// `upload` can read the project the user is working in, and nothing else — instead of
/// all of Downloads, Desktop and Documents.
static MCP_ROOTS: std::sync::OnceLock<Vec<PathBuf>> = std::sync::OnceLock::new();

/// Record the roots from the client's `initialize`. First call wins, because the roots
/// are part of the session handshake and a later change would silently widen or narrow
/// what an in-flight action may read.
pub fn set_mcp_roots(roots: Vec<PathBuf>) {
    let canonical: Vec<PathBuf> = roots
        .into_iter()
        .map(|p| std::fs::canonicalize(&p).unwrap_or(p))
        .collect();
    if !canonical.is_empty() {
        let _ = MCP_ROOTS.set(canonical);
    }
}

pub fn mcp_roots() -> &'static [PathBuf] {
    MCP_ROOTS.get().map(Vec::as_slice).unwrap_or(&[])
}

/// Maximum bytes `download` will accept, from `NEOBROWSER_MAX_DOWNLOAD_MB`.
///
/// A cap has to exist: without one, a hostile or misconfigured URL can fill the disk,
/// and the body is buffered before being written. 200 MiB is generous for the
/// documents and archives this tool is for, and it is raisable.
pub(super) fn download_size_cap() -> usize {
    let mb = std::env::var("NEOBROWSER_MAX_DOWNLOAD_MB")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|m| *m > 0)
        .unwrap_or(200);
    mb * 1024 * 1024
}

/// Write a download without overwriting an existing file, and without leaving a
/// partial file behind on failure.
///
/// Two problems with the previous `fs::write(&dest, ..)`. It silently replaced any
/// existing file — an agent downloading `invoice.pdf` twice destroyed the first one.
/// And a failure mid-write left a truncated file at the destination that looked
/// complete. So: write to a temp name, then link into place under a free filename.
///
/// Returns the path actually written and whether a suffix had to be added.
pub(super) fn write_download_atomically(
    dest: &std::path::Path,
    bytes: &[u8],
) -> std::io::Result<(PathBuf, bool)> {
    use std::io::Write;

    let parent = dest.parent().unwrap_or(std::path::Path::new("."));
    std::fs::create_dir_all(parent)?;

    let mut tmp_name = dest.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(format!(".part-{}", std::process::id()));
    let tmp = dest.with_file_name(tmp_name);
    {
        let mut f = std::fs::File::create(&tmp)?;
        if let Err(e) = f.write_all(bytes).and_then(|_| f.sync_all()) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
    }

    // Find a free name. `create_new` on the target is what makes this race-free:
    // testing `exists()` and then renaming would let a concurrent download win the
    // gap and be clobbered anyway.
    let mut candidate = dest.to_path_buf();
    let mut renamed = false;
    for n in 1..1000 {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(_) => {
                // We now own this name; replacing the placeholder with the temp file
                // is atomic within the directory.
                std::fs::rename(&tmp, &candidate)?;
                return Ok((candidate, renamed));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                renamed = true;
                let stem = dest
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "download".into());
                let ext = dest
                    .extension()
                    .map(|e| format!(".{}", e.to_string_lossy()))
                    .unwrap_or_default();
                candidate = parent.join(format!("{stem} ({n}){ext}"));
            }
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(e);
            }
        }
    }
    let _ = std::fs::remove_file(&tmp);
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not find a free filename after 1000 attempts",
    ))
}
