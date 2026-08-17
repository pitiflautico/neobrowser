//! `neobrowser doctor` — answer "will this work here, and if not, why".
//!
//! The JSON form exists for CI and for bug reports. It reports the sandbox state, the policy
//! profile and a real launch attempt, because the useful diagnostic is never a version
//! string — it is whether Chrome actually came up on this machine.

//! NeoBrowser — a fast, stealthy MCP browser-automation server that drives real
//! Chrome via CDP. `serve` runs the MCP server (default); `doctor` checks the
//! environment; `tools` prints the tool catalog for humans/AIs.

use neobrowser::{chrome, paths, tool_impls};

use super::report::{report_policy, report_sandbox, smoke_test};

/// Report Chrome discovery, version, and home-dir layout — the Rust equivalent of
/// the Python `neobrowser doctor`.
pub async fn doctor() {
    let bin = chrome::chrome_bin();
    println!("NeoBrowser doctor");
    println!(" home: {}", paths::home().display());
    println!("  chrome bin:  {}", bin.display());
    println!("  chrome found: {}", bin.exists());
    match chrome::detect_chrome_major(bin) {
        Some(major) => println!("  chrome major: {major}"),
        None => println!("  chrome major: <unknown>"),
    }
    match chrome::chrome_user_agent() {
        Some(ua) => println!("  user-agent:  {ua}"),
        None => println!("  user-agent:  <default>"),
    }
    report_sandbox();
    report_policy();
    // Prove the process/CDP path works end-to-end if Chrome is present.
    if bin.exists() {
        print!("  launch+CDP:  ");
        match smoke_test().await {
            Ok(title) => println!("ok (about:blank title={title:?})"),
            Err(e) => println!("FAILED: {e}"),
        }
    }
}

/// `doctor --json` — every environment check as one machine-readable document.
///
/// Exists so CI and installers can gate on the result instead of grepping prose. Each
/// check reports `ok` plus a `detail`, and the process exits non-zero if any check
/// failed — a doctor that always exits 0 is decoration.
pub async fn doctor_json() {
    use serde_json::{json, Value};

    let bin = chrome::chrome_bin();
    let mut checks: Vec<Value> = Vec::new();

    let major = chrome::detect_chrome_major(bin);
    checks.push(
        json!({ "check": "chrome_found", "ok": bin.exists(), "detail": bin.display().to_string() }),
    );
    checks.push(json!({
        "check": "chrome_version",
        "ok": major.is_some(),
        "detail": major.clone().unwrap_or_else(|| "unknown".into()),
    }));

    let support = chrome::sandbox_support();
    let opted_out = chrome::no_sandbox_opt_in_active();
    checks.push(json!({
        "check": "sandbox",
        "ok": !opted_out && support == chrome::SandboxSupport::Available,
        "detail": if opted_out {
            "DISABLED via NEOBROWSER_ALLOW_NO_SANDBOX".to_string()
        } else {
            format!("{support:?}")
        },
    }));

    let policy = neobrowser::policy::Policy::from_env();
    checks.push(json!({
        "check": "policy",
        "ok": true,
        "detail": format!(
            "profile={} allow={:?} deny={:?}",
            policy.profile.label(),
            policy.allow_list(),
            policy.deny_list()
        ),
    }));
    // The one policy configuration that silently refuses everything.
    let autonomous_without_list =
        policy.profile == neobrowser::policy::Profile::Autonomous && policy.allow_list().is_empty();
    checks.push(json!({
        "check": "policy_usable",
        "ok": !autonomous_without_list,
        "detail": if autonomous_without_list {
            "autonomous profile with an empty NEOBROWSER_ALLOW_DOMAINS refuses every call"
        } else {
            "ok"
        },
    }));

    let vault_ok = neobrowser::vault::available();
    checks.push(json!({
        "check": "vault",
        "ok": vault_ok,
        "detail": if vault_ok {
            "OS credential store reachable; session material is encrypted at rest"
        } else {
            "no OS credential store and no NEOBROWSER_VAULT_KEY: save_cookies/save_session will refuse rather than write plaintext"
        },
    }));

    let home = paths::home();
    let writable = neobrowser::sessions::probe_writable(&home).is_ok();
    checks.push(
        json!({ "check": "home_writable", "ok": writable, "detail": home.display().to_string() }),
    );

    let roots = neobrowser::reach::upload_roots_for_report();
    checks.push(
        json!({ "check": "upload_roots", "ok": !roots.is_empty(), "detail": roots.join(", ") }),
    );

    let profile_dir = paths::profile_dir();
    let locked = chrome::profile_lock_holder(&profile_dir);
    checks.push(json!({
        "check": "profile_free",
        "ok": locked.is_none(),
        "detail": match locked {
            Some(pid) => format!("{} is held by pid {pid}", profile_dir.display()),
            None => profile_dir.display().to_string(),
        },
    }));

    let registry = tool_impls::build_registry();
    checks.push(json!({
        "check": "mcp_tools",
        "ok": !registry.is_empty(),
        "detail": format!("{} tools registered", registry.len()),
    }));

    if bin.exists() {
        let (ok, detail) = match smoke_test().await {
            Ok(title) => (true, format!("about:blank title={title:?}")),
            Err(e) => (false, e),
        };
        checks.push(json!({ "check": "launch_cdp", "ok": ok, "detail": detail }));
    } else {
        checks.push(
            json!({ "check": "launch_cdp", "ok": false, "detail": "skipped: no Chrome binary" }),
        );
    }

    let failed: Vec<Value> = checks
        .iter()
        .filter(|c| c["ok"] == Value::Bool(false))
        .map(|c| c["check"].clone())
        .collect();
    let report = json!({
        "ok": failed.is_empty(),
        "version": env!("CARGO_PKG_VERSION"),
        "failed": failed,
        "checks": checks,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{}".into())
    );
    if !failed.is_empty() {
        std::process::exit(1);
    }
}
