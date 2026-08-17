//! Reading a trace back off disk.
//!
//! Trace ids are rejected if they contain path traversal, because a trace id arrives from
//! whatever asked for it — and `../../etc/passwd` is a valid-looking id.

//! Execution traces: correlated events, secret redaction, and shareable evidence
//! bundles.
//!
//! When an agent run goes wrong, the question is always "what actually happened, in
//! order?" — and the answer was previously spread across the model's transcript, the
//! server's log lines, and nothing else. This module records the timeline itself:
//! every action with its ids, the policy decisions, the walls hit, the URLs and
//! redirects.
//!
//! Two properties do the work.
//!
//! **Correlation.** Every event carries `trace_id`, and where applicable `action_id`
//! and `tab_id`. Without those, interleaved events from two tabs are unreadable.
//!
//! **Redaction by default.** A trace of a browser session naturally contains cookies,
//! `Authorization` headers, tokens in query strings and form values. A bundle exists
//! to be shared — attached to a bug report, pasted into an issue — so redaction is
//! not an option a user has to remember to switch on. [`redact`] runs on every value
//! entering the trace, and the tests are the specification.

use serde_json::Value;

/// Read a previously written bundle, for `neobrowser trace open <id>`.
pub fn read_bundle(trace_id: &str) -> std::io::Result<Value> {
    // The id becomes a filename, so it is validated rather than trusted: an id of
    // `../../.ssh/id_rsa` must not turn a debug command into a file read.
    if !trace_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "trace id must be alphanumeric with _ or -",
        ));
    }
    let path = crate::paths::home()
        .join("traces")
        .join(format!("{trace_id}.json"));
    let text = std::fs::read_to_string(path)?;
    serde_json::from_str(&text)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
}

/// List available traces, newest first.
pub fn list_bundles() -> Vec<String> {
    let dir = crate::paths::home().join("traces");
    let mut out: Vec<(std::time::SystemTime, String)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let id = name.strip_suffix(".json")?.to_string();
            let modified = e.metadata().ok()?.modified().ok()?;
            Some((modified, id))
        })
        .collect();
    // Newest first, so `trace list` shows the run you just finished at the top.
    out.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    out.into_iter().map(|(_, id)| id).collect()
}
