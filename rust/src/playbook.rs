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
    // Resolved once and threaded through: `playbooks_base()` reads NEOBROWSER_HOME
    // every call, so re-deriving it for the containment check could compare a path
    // built under one base against a different one.
    let base = paths::playbooks_base();
    // Sanitize path components so a crafted name can't escape the base dir.
    let path = base
        .join(sanitize(domain))
        .join(format!("{}.json", sanitize(task)));
    // Belt and braces: `sanitize` is what guarantees containment, so if it ever
    // regresses, fail into the base dir rather than writing outside it. `domain`
    // can originate in page-derived data, which is not ours to trust.
    debug_assert!(
        is_contained(&base, &path),
        "playbook path escaped: {path:?}"
    );
    if is_contained(&base, &path) {
        path
    } else {
        base.join("_.json")
    }
}

/// Does `path` stay under `base` — i.e. no `..` climbing out of it? Purely
/// lexical, since the file usually does not exist yet and `canonicalize` needs it
/// to.
fn is_contained(base: &std::path::Path, path: &std::path::Path) -> bool {
    use std::path::Component;
    path.starts_with(base) && !path.components().any(|c| c == Component::ParentDir)
}

/// Reduce an untrusted name to one safe path component.
///
/// Replacing separators is not sufficient on its own. A component of exactly `..`
/// contains no separator yet still means "the parent directory", so
/// `playbooks/{domain}` with `domain = ".."` resolves to `~/.neobrowser/` and the
/// playbook lands outside its store. Likewise `.` silently collapses one level.
/// So: keep the readable characters, then reject any result that is not a *name* —
/// empty, or nothing but dots.
fn sanitize(s: &str) -> String {
    let mapped: String = s
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
    if mapped.is_empty() || mapped.chars().all(|c| c == '.') {
        return "_".to_string();
    }
    mapped
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
        let s = sanitize("../../etc");
        assert!(!s.contains('/'), "slash survived sanitize: {s}");
        assert_eq!(s, ".._.._etc");
        assert_eq!(sanitize("linkedin.com"), "linkedin.com");
        // A dot is legal inside a name, so ordinary hidden-style names survive.
        assert_eq!(sanitize(".config"), ".config");
    }

    /// The earlier version of `sanitize` only stripped separators, on the theory
    /// that dots without a slash are harmless. They are not: a component of
    /// exactly `..` climbs a level all by itself.
    #[test]
    fn sanitize_rejects_dot_only_components() {
        for traversal in ["..", ".", "...", "....", ""] {
            let s = sanitize(traversal);
            assert_eq!(s, "_", "{traversal:?} must not survive as a component");
        }
    }

    #[test]
    fn crafted_names_cannot_escape_the_playbook_store() {
        // `playbooks_base()` reads NEOBROWSER_HOME, which sibling tests mutate;
        // without this lock the base can shift mid-test.
        let _g = crate::env_test_guard();
        let base = paths::playbooks_base();
        for (domain, task) in [
            ("..", "cookies"),
            (".", "cookies"),
            ("../..", "cookies"),
            ("linkedin.com", ".."),
            ("..", ".."),
            ("/etc/passwd", "x"),
            ("..\\..\\windows", "x"),
        ] {
            let path = playbook_path(domain, task);
            assert!(
                path.starts_with(&base),
                "({domain:?}, {task:?}) escaped to {path:?}"
            );
            assert!(
                is_contained(&base, &path),
                "({domain:?}, {task:?}) produced a climbing path: {path:?}"
            );
            // Exactly base/<domain>/<task>.json — no level collapsed away.
            assert_eq!(
                path.components().count(),
                base.components().count() + 2,
                "({domain:?}, {task:?}) changed the depth: {path:?}"
            );
        }
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
