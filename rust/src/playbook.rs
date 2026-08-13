//! Playbook persistence: recorded tool-call sequences saved under
//! `~/.neobrowser/playbooks/{domain}/{task}.json`, replayed by re-invoking each tool.
//!
//! A step is `{ "tool": "<name>", "args": { ... } }`. Recording is driven from the
//! MCP dispatch layer (see `mcp.rs`); replay is the `replay` tool re-dispatching
//! each step through the registry.

use std::path::PathBuf;

use serde_json::Value;

use crate::paths;

fn playbook_path(domain: &str, task: &str) -> PathBuf {
    // Sanitize path components so a crafted name can't escape the base dir.
    paths::playbooks_base()
        .join(sanitize(domain))
        .join(format!("{}.json", sanitize(task)))
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .take(120)
        .collect()
}

/// Persist a recorded step list. Written 0600: recorded `fill`/`form_fill`/`type`
/// steps can contain credentials, same as the cookie/session snapshots.
pub fn save(domain: &str, task: &str, steps: &[Value]) -> std::io::Result<()> {
    let path = playbook_path(domain, task);
    crate::sessions::write_private(
        &path,
        &serde_json::to_string_pretty(steps).unwrap_or_else(|_| "[]".into()),
    )
}

/// Load a recorded step list (empty if the playbook does not exist).
pub fn load(domain: &str, task: &str) -> Vec<Value> {
    let path = playbook_path(domain, task);
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Tools whose calls are worth recording (mutating page actions). Read-only and
/// meta tools (status/read/screenshot/record_task/replay/…) are never recorded.
pub fn is_recordable(tool: &str) -> bool {
    matches!(
        tool,
        "navigate"
            | "click"
            | "type"
            | "fill"
            | "form_fill"
            | "submit"
            | "find_and_click"
            | "scroll"
            | "paginate"
            | "dismiss_overlay"
            | "upload"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn recordable_set() {
        assert!(is_recordable("navigate"));
        assert!(is_recordable("click"));
        assert!(!is_recordable("read"));
        assert!(!is_recordable("replay"));
        assert!(!is_recordable("record_task"));
    }

    #[test]
    fn sanitize_blocks_path_traversal() {
        // Slashes (the only traversal vector) are stripped; dots alone are harmless
        // since without a separator they can't escape the base dir.
        let s = sanitize("../../etc");
        assert!(!s.contains('/'), "slash survived sanitize: {s}");
        assert_eq!(s, ".._.._etc");
        assert_eq!(sanitize("linkedin.com"), "linkedin.com");
    }

    #[test]
    fn save_then_load_round_trip() {
        let _g = crate::env_test_guard();
        std::env::set_var("NEOBROWSER_HOME", "/tmp/nb-playbook-test");
        let steps = vec![json!({ "tool": "navigate", "args": { "url": "https://x.com" } })];
        save("x.com", "open", &steps).unwrap();
        let loaded = load("x.com", "open");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0]["tool"], "navigate");
        assert_eq!(load("x.com", "missing").len(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn save_writes_owner_only_perms() {
        // Playbooks can capture credentials via fill/form_fill/type steps.
        let _g = crate::env_test_guard();
        std::env::set_var("NEOBROWSER_HOME", "/tmp/nb-playbook-perms-test");
        save("x.com", "creds", &[json!({ "tool": "fill", "args": {} })]).unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(playbook_path("x.com", "creds"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
