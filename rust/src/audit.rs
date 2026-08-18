//! Persistent audit log: every tool call, append-only, owner-only.
//!
//! From community feedback (HN): operators want a durable record of what the
//! agent actually did — which tools ran, with what (masked) arguments, and how
//! they ended. One JSON line per call in `{NEOBROWSER_HOME}/audit.log` (0600).
//! Disable with `NEOBROWSER_AUDIT=off`.

use std::time::Duration;

use serde_json::{json, Map, Value};

/// Arg keys whose values are masked in the log (credentials, cookies, tokens…).
fn is_sensitive_key(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    ["pass", "secret", "token", "cookie", "credential", "apikey", "api_key"]
        .iter()
        .any(|s| k.contains(s))
}

/// The args as they should appear in the log: sensitive values replaced.
pub fn masked_args(args: &Map<String, Value>) -> Value {
    let mut out = Map::new();
    for (k, v) in args {
        out.insert(
            k.clone(),
            if is_sensitive_key(k) {
                json!("•••")
            } else {
                v.clone()
            },
        );
    }
    Value::Object(out)
}

fn audit_path() -> Option<std::path::PathBuf> {
    if std::env::var("NEOBROWSER_AUDIT")
        .map(|v| v.eq_ignore_ascii_case("off"))
        .unwrap_or(false)
    {
        return None;
    }
    Some(crate::paths::home().join("audit.log"))
}

/// Append one record for a tool call. Best-effort: auditing never breaks a call.
pub fn log_tool_call(name: &str, args: &Map<String, Value>, ok: bool, error: Option<&str>, elapsed: Duration) {
    let Some(path) = audit_path() else { return };
    let line = json!({
        "ts": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        "tool": name,
        "args": masked_args(args),
        "ok": ok,
        "error": error,
        "ms": elapsed.as_millis(),
    })
    .to_string();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    use std::io::Write;
    let appended = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| writeln!(f, "{line}"));
    if appended.is_ok() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_values_are_masked() {
        let mut args = Map::new();
        args.insert("url".into(), json!("https://example.com"));
        args.insert("password".into(), json!("hunter2"));
        args.insert("session_token".into(), json!("abc"));
        let masked = masked_args(&args);
        assert_eq!(masked["url"], json!("https://example.com"));
        assert_eq!(masked["password"], json!("•••"));
        assert_eq!(masked["session_token"], json!("•••"));
    }

    #[test]
    fn audit_line_is_written_with_owner_perms() {
        let _g = crate::env_test_guard();
        let dir = std::env::temp_dir().join("nb-audit-test");
        std::env::set_var("NEOBROWSER_HOME", &dir);
        std::env::remove_var("NEOBROWSER_AUDIT");
        let mut args = Map::new();
        args.insert("password".into(), json!("hunter2"));
        log_tool_call("login", &args, true, None, Duration::from_millis(12));
        let content = std::fs::read_to_string(dir.join("audit.log")).unwrap();
        assert!(content.contains("\"tool\":\"login\""));
        assert!(content.contains("•••"));
        assert!(!content.contains("hunter2"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join("audit.log")).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn audit_off_writes_nothing() {
        let _g = crate::env_test_guard();
        let dir = std::env::temp_dir().join("nb-audit-test-off");
        std::env::set_var("NEOBROWSER_HOME", &dir);
        std::env::set_var("NEOBROWSER_AUDIT", "off");
        log_tool_call("status", &Map::new(), true, None, Duration::ZERO);
        assert!(!dir.join("audit.log").exists());
        std::env::remove_var("NEOBROWSER_AUDIT");
    }
}
