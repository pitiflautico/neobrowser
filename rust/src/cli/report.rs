//! The pieces `doctor --json` reports, and the smoke test behind them.
//!
//! Separated from the command itself because these are the parts worth reading on their own:
//! what the policy profile actually permits, whether the renderer sandbox is on and why, and
//! whether a real Chrome launch-and-navigate succeeds.

//! NeoBrowser — a fast, stealthy MCP browser-automation server that drives real
//! Chrome via CDP. `serve` runs the MCP server (default); `doctor` checks the
//! environment; `tools` prints the tool catalog for humans/AIs.

use neobrowser::{cdp, chrome, paths};

use std::time::Duration;

/// Report the policy profile and domain rules that will govern this session.
///
/// Shown even when everything is permissive: "no rules are configured" is itself a
/// posture the operator should see stated, not have to infer from silence.
pub fn report_policy() {
    let policy = neobrowser::policy::Policy::from_env();
    println!(" policy: {}", policy.profile.label());
    if policy.has_domain_rules() {
        let allow = policy.allow_list();
        let deny = policy.deny_list();
        println!(
            "               allow: {}",
            if allow.is_empty() {
                "(any)".to_string()
            } else {
                allow.join(", ")
            }
        );
        if !deny.is_empty() {
            println!("               deny:  {}", deny.join(", "));
        }
    } else if policy.profile == neobrowser::policy::Profile::Autonomous {
        // The one combination that denies everything: worth flagging here rather
        // than letting the operator discover it through a wall of refusals.
        println!(
            "               *** no allowlist set — the autonomous profile will refuse \
             every call ***"
        );
    } else {
        println!("               no domain rules (any destination allowed)");
    }
}

/// Report whether Chrome's renderer sandbox will actually be active.
///
/// Printed unconditionally, and loudly when it is off: an operator who cannot see
/// that the sandbox is disabled has no way to know the browser is one renderer bug
/// away from the whole machine.
pub fn report_sandbox() {
    let support = chrome::sandbox_support();
    let opted_out = chrome::no_sandbox_opt_in_active();
    let real_profile = std::env::var("NEOBROWSER_REAL_PROFILE")
        .ok()
        .filter(|v| !v.trim().is_empty());

    print!("  sandbox:     ");
    match (opted_out, support) {
        (false, chrome::SandboxSupport::Available) => println!("ON (host supports it)"),
        (false, blocked) => {
            println!("host CANNOT sandbox — launches will be refused ({blocked:?})");
            println!(
                "               fix the host rather than disabling the sandbox; \
                 `neobrowser doctor` after the fix should read ON"
            );
        }
        (true, _) => {
            println!("*** OFF — NEOBROWSER_ALLOW_NO_SANDBOX is set ***");
            println!(
                "               a compromised page can escape the renderer and reach \
                 this machine"
            );
            if let Some(p) = real_profile {
                println!(
                    "               and it is holding real cookies from profile {p:?} — \
                     unset one of the two"
                );
            }
        }
    }
}

/// Launch headless Chrome, open a tab, connect CDP, evaluate a trivial expression.
pub async fn smoke_test() -> Result<String, String> {
    let profile = paths::profiles_base().join("doctor");
    let mut proc = chrome::ChromeProcess::launch(&profile)
        .await
        .map_err(|e| e.to_string())?;
    let result = async {
        chrome::wait_for_chrome(proc.port, Duration::from_secs(10))
            .await
            .map_err(|e| e.to_string())?;
        let tab = chrome::open_new_tab(proc.port)
            .await
            .map_err(|e| e.to_string())?;
        let client = cdp::CdpClient::connect(&tab.web_socket_debugger_url)
            .await
            .map_err(|e| e.to_string())?;
        client
            .send("Page.enable", serde_json::json!({}))
            .await
            .map_err(|e| e.to_string())?;
        let title = client
            .eval("document.title")
            .await
            .map_err(|e| e.to_string())?;
        Ok::<String, String>(title.as_str().unwrap_or("").to_string())
    }
    .await;
    proc.kill(true).await;
    result
}
