//! MCP protocol (JSON-RPC 2.0 over stdin/stdout).
//!
//! Port of the protocol half of the Python `server.py`: `initialize`, `tools/list`,
//! `tools/call`, and `notifications/initialized`, with the same argument-validation
//! contract and the same 500k-char text cap. Screenshots return native MCP image
//! content instead of the Python string-JSON round-trip.
//!
//! Split into [`mod@serve`] (the stdio loop and cooperative shutdown), [`protocol`] (version
//! negotiation) and [`dispatch`] (routing a request to a tool and shaping the response).
//! The server identity and the instructions text stay here, since they are what the server
//! *is* rather than what it does.

pub mod dispatch;
pub mod protocol;
pub mod serve;

pub use dispatch::handle_request;
pub use serve::serve;

use serde_json::{json, Value};
use std::time::Duration;
use tokio::io::AsyncWriteExt;

const SERVER_NAME: &str = "neobrowser";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const PROTOCOL_VERSION: &str = "2024-11-05";
const MAX_TEXT: usize = 500_000;

/// Whether the connected client advertised the `elicitation` capability at
/// initialize (Claude Code CLI does; several other clients silently don't).
static CLIENT_SUPPORTS_ELICITATION: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Tools that can be gated behind interactive user approval.
/// `NEOBROWSER_REQUIRE_APPROVAL`: unset = no gating; `1`/`true`/`all` = the
/// default sensitive set; otherwise a comma-separated tool list.
fn approval_required(tool: &str) -> bool {
    const DEFAULT_SENSITIVE: &[&str] = &["submit", "form_fill", "download", "upload", "login"];
    match std::env::var("NEOBROWSER_REQUIRE_APPROVAL") {
        Err(_) => false,
        Ok(v) => {
            let v = v.trim().to_ascii_lowercase();
            if v.is_empty() || v == "0" || v == "off" {
                false
            } else if v == "1" || v == "true" || v == "all" {
                DEFAULT_SENSITIVE.contains(&tool)
            } else {
                v.split(',').any(|t| t.trim() == tool)
            }
        }
    }
}

/// Whether a `tools/call` must be confirmed interactively first.
pub(crate) enum ApprovalGate {
    NotNeeded,
    /// Gated, but the client never advertised elicitation support.
    Unsupported {
        id: Value,
        tool: String,
    },
    /// Gated and the client supports elicitation: ask the user.
    Ask {
        id: Value,
        tool: String,
    },
}

/// Decide whether a request needs interactive approval before it runs.
pub(crate) fn approval_gate(req: &Value) -> ApprovalGate {
    if req.get("method").and_then(|m| m.as_str()) != Some("tools/call") {
        return ApprovalGate::NotNeeded;
    }
    let params = req.get("params").cloned().unwrap_or(Value::Null);
    let tool = params
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or("")
        .to_string();
    if !approval_required(&tool) {
        return ApprovalGate::NotNeeded;
    }
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    if CLIENT_SUPPORTS_ELICITATION.load(std::sync::atomic::Ordering::Relaxed) {
        ApprovalGate::Ask { id, tool }
    } else {
        ApprovalGate::Unsupported { id, tool }
    }
}

/// Ask the user to confirm an action via `elicitation/create`; true on accept.
/// Waits up to 120s for the matching response, ignoring unrelated traffic.
pub(crate) async fn ask_user(
    lines_rx: &mut tokio::sync::mpsc::UnboundedReceiver<String>,
    stdout: &mut tokio::io::Stdout,
    tool: &str,
) -> bool {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = format!(
        "nb-elicit-{}",
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let req = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "elicitation/create",
        "params": {
            "message": format!("NeoBrowser wants to run '{tool}'. Allow?"),
            "requestedSchema": {
                "type": "object",
                "properties": {
                    "confirm": { "type": "boolean", "title": format!("Allow {tool}?") }
                },
                "required": ["confirm"]
            }
        }
    });
    let mut buf = serde_json::to_string(&req).unwrap_or_default();
    buf.push('\n');
    if stdout.write_all(buf.as_bytes()).await.is_err() || stdout.flush().await.is_err() {
        return false;
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    loop {
        let next = tokio::time::timeout_at(deadline, lines_rx.recv()).await;
        let Ok(Some(line)) = next else { return false }; // timeout or EOF
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if msg.get("id").and_then(|v| v.as_str()) != Some(id.as_str()) {
            continue; // not our answer; sequential clients make this rare
        }
        let result = msg.get("result").cloned().unwrap_or(Value::Null);
        let action = result.get("action").and_then(|a| a.as_str()).unwrap_or("");
        let confirmed = result
            .get("content")
            .and_then(|c| c.get("confirm"))
            .and_then(|c| c.as_bool())
            .unwrap_or(false);
        return action == "accept" && confirmed;
    }
}

/// Record whether the connected client advertised elicitation support.
pub(crate) fn set_elicitation_supported(supported: bool) {
    CLIENT_SUPPORTS_ELICITATION.store(supported, std::sync::atomic::Ordering::Relaxed);
}

/// Guidance injected into the model's context at `initialize` (MCP `instructions`),
/// so an AI understands how to drive these tools well without trial and error.
const INSTRUCTIONS: &str = "\
NeoBrowser drives a real Chrome via CDP for autonomous web use. It is stealthy \
(passes bot detectors with a genuine fingerprint) and can reuse your real logged-in \
sessions.

Core loop:
- `navigate {url}` first. Its result flags any bot wall / captcha / consent / login \
gate — react to that hint (dismiss_overlay, login, or a real profile) instead of \
retrying blindly.
- `read` returns visible text; `page_info`/`analyze` describe structure (forms, \
buttons, overlays).
- To act on an element: `find {intent}` (natural language, e.g. \"send button\") \
returns a backendNodeId, then `click {backend_node_id}`. Or `find_and_click {text}`. \
Clicks are real (isTrusted) mouse events, scroll the target into view, and only \
target VISIBLE elements — a match inside a collapsed accordion step or a hidden \
header panel is skipped, not clicked.
- Mutating actions (navigate, click, type, fill, form_fill, submit, find_and_click) \
return a verified result envelope, NOT a confirmation message. Read `status`:
  * `succeeded` — the page actually changed; `evidence.changes` says how \
(navigation / title / dom_nodes / text / control_state).
  * `uncertain` — the event was delivered but NOTHING on the page changed. This is \
not a success. Do not build on it. Re-observe, or try a different target.
  * `failed` — the action could not be performed; `detail` says why and `retryable` \
says whether trying again could help.
  * `blocked` — a wall or a policy stopped it. `needs_human` — only a person can \
proceed (captcha, login, MFA).
  A covered target reports `failed` with the covering element named: \
`dismiss_overlay`, then retry.
- Give slow pages room with `budget_s` instead of retrying blindly; a budget that \
runs out is reported as `uncertain` with a `budget_exhausted` warning, never as \
success.
- Multi-step forms: each step keeps its own buttons in the DOM, so target the step \
you mean (a CSS selector scoped to its form) rather than the first button with the \
right label, and check the page changed before moving on.
- Forms: `fill {selector,value}` or `form_fill {fields}` (by label), then `submit`.
- Files: `upload {selector,files}`; `download {url}` (reuses session cookies).

Rendering note: content is force-rendered on read/find/scroll (headless compositor \
is otherwise idle), so prefer those over blind waits.

Search is multi-source and routes around walls: `search` (web), `search_images`, \
`search_videos`.

Tabs: `new_tab`/`list_tabs`/`switch_tab`/`close_tab` — tools act on the active tab.

Real sessions: set NEOBROWSER_REAL_PROFILE to start authenticated; or \
NEOBROWSER_ATTACH_PORT to drive a Chrome you already have open. Act only as the user \
would themselves.

Chrome locks a profile exclusively, so two sessions sharing one cannot both run. \
If a launch reports the profile is in use, either attach to that browser on the port \
it names, or set NEOBROWSER_PROFILE=<name> to get an isolated one.

Policy: calls are checked before they run. A refusal comes back as JSON with \
`status`, `reason` and `remedy` — read it instead of retrying the same call. \
`status: \"blocked\"` means a rule forbids this destination or action; change course, \
and surface `remedy` to the user if only they can lift it. \
`status: \"requires_confirmation\"` means the action is permitted but needs the \
user's explicit approval first — ask them, then re-issue. Never try to route around \
a refusal, and never treat one as if the action had succeeded.";

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use std::sync::Arc;

    use crate::browser::Browser;
    use crate::tool_impls;
    use crate::tools::ToolCtx;

    use super::protocol::{negotiate_protocol_version, SUPPORTED_PROTOCOL_VERSIONS};
    use super::*;

    fn ctx() -> ToolCtx {
        ctx_with_policy(crate::policy::Policy::default())
    }

    fn ctx_with_policy(policy: crate::policy::Policy) -> ToolCtx {
        ToolCtx {
            browser: Arc::new(Browser::new()),
            registry: Arc::new(tool_impls::build_registry()),
            policy: Arc::new(policy),
            trace: Arc::new(crate::trace::Trace::new("trace_test")),
            bridge: None,
        }
    }

    #[tokio::test]
    async fn initialize_returns_server_info() {
        let reg = tool_impls::build_registry();
        let req = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" });
        let resp = handle_request(&reg, &ctx(), &req).await.unwrap();
        assert_eq!(resp["result"]["serverInfo"]["name"], "neobrowser");
        assert_eq!(resp["result"]["protocolVersion"], PROTOCOL_VERSION);
    }

    #[tokio::test]
    async fn tools_list_advertises_registered_tools() {
        let reg = tool_impls::build_registry();
        let req = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" });
        let resp = handle_request(&reg, &ctx(), &req).await.unwrap();
        let tools = resp["result"]["tools"].as_array().unwrap();
        assert!(tools.iter().any(|t| t["name"] == "status"));
    }

    #[tokio::test]
    async fn unknown_tool_is_rpc_error() {
        let reg = tool_impls::build_registry();
        let req = json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": { "name": "nope", "arguments": {} }
        });
        let resp = handle_request(&reg, &ctx(), &req).await.unwrap();
        assert_eq!(resp["error"]["code"], -32601);
        assert!(resp["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Unknown tool: nope"));
    }

    #[tokio::test]
    async fn bad_argument_is_iserror_not_crash() {
        let reg = tool_impls::build_registry();
        // status takes no args; passing one must be a validation isError.
        let req = json!({
            "jsonrpc": "2.0", "id": 4, "method": "tools/call",
            "params": { "name": "status", "arguments": { "x": 1 } }
        });
        let resp = handle_request(&reg, &ctx(), &req).await.unwrap();
        assert_eq!(resp["result"]["isError"], true);
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("unknown argument(s): x"), "got: {text}");
    }

    #[tokio::test]
    async fn notification_returns_no_response() {
        let reg = tool_impls::build_registry();
        let req = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        assert!(handle_request(&reg, &ctx(), &req).await.is_none());
    }

    #[tokio::test]
    async fn invalid_numeric_args_are_iserror_not_panic() {
        // Regression: negative/NaN waits used to reach Duration::from_secs* and
        // panic the whole server. Validation runs before any Chrome launch.
        let reg = tool_impls::build_registry();
        for (tool, args) in [
            ("submit", json!({ "wait_s": -1.0 })),
            ("wait", json!({ "ms": -5 })),
        ] {
            let req = json!({
                "jsonrpc": "2.0", "id": 10, "method": "tools/call",
                "params": { "name": tool, "arguments": args }
            });
            let resp = handle_request(&reg, &ctx(), &req).await.unwrap();
            assert_eq!(resp["result"]["isError"], true, "{tool} should be isError");
            let text = resp["result"]["content"][0]["text"].as_str().unwrap();
            assert!(text.contains("must be"), "{tool} got: {text}");
        }
    }

    #[tokio::test]
    async fn status_tool_runs_end_to_end_without_chrome() {
        // status reports discovery without launching Chrome, so it works in CI.
        let reg = tool_impls::build_registry();
        let req = json!({
            "jsonrpc": "2.0", "id": 5, "method": "tools/call",
            "params": { "name": "status", "arguments": {} }
        });
        let resp = handle_request(&reg, &ctx(), &req).await.unwrap();
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["session_up"], false);
        assert!(parsed.get("chrome_bin").is_some());
    }

    #[tokio::test]
    async fn the_protocol_version_is_negotiated_not_asserted() {
        let reg = tool_impls::build_registry();
        // A version we support must be echoed back.
        for want in SUPPORTED_PROTOCOL_VERSIONS {
            let req = json!({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": { "protocolVersion": want }
            });
            let resp = handle_request(&reg, &ctx(), &req).await.unwrap();
            assert_eq!(
                resp["result"]["protocolVersion"], *want,
                "must echo a supported version"
            );
        }
        // Something we do not support falls back to ours rather than failing.
        let req = json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "protocolVersion": "1999-01-01" }
        });
        let resp = handle_request(&reg, &ctx(), &req).await.unwrap();
        assert_eq!(resp["result"]["protocolVersion"], PROTOCOL_VERSION);

        // No version at all (older clients) still gets a usable answer.
        let req = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" });
        let resp = handle_request(&reg, &ctx(), &req).await.unwrap();
        assert_eq!(resp["result"]["protocolVersion"], PROTOCOL_VERSION);
    }

    /// Only `file://` roots describe a directory. Treating an `https://` root as one
    /// would hand `upload` a path that does not exist, or worse, a relative one.
    #[test]
    fn only_file_roots_are_taken_as_directories() {
        let params = json!({
            "capabilities": { "roots": { "roots": [
                { "uri": "https://example.com/", "name": "web" },
                { "uri": "file:///tmp", "name": "tmp" },
            ] } }
        });
        // The call also negotiates; the assertion here is that it does not panic and
        // that a non-file root is filtered out rather than parsed as a path.
        let _ = negotiate_protocol_version(&params);
        let roots = crate::reach::mcp_roots();
        assert!(
            roots.is_empty() || roots.iter().all(|p| p.is_absolute()),
            "a non-file root must never become a path: {roots:?}"
        );
    }

    /// Build a policy from a scoped env mutation, so these tests neither race other
    /// env-mutating tests nor depend on the host's config.
    ///
    /// The guard lives and dies inside this function. Holding it across the caller's
    /// `.await` would both trip clippy's `await_holding_lock` and risk deadlocking
    /// the suite, and it is unnecessary: once the `Policy` is built it owns its
    /// rules and never reads the environment again.
    fn deny_all_policy() -> crate::policy::Policy {
        let _g = crate::env_test_guard();
        std::env::set_var("NEOBROWSER_DENY_DOMAINS", "blocked.test");
        let p = crate::policy::Policy::from_env();
        std::env::remove_var("NEOBROWSER_DENY_DOMAINS");
        p
    }

    /// The point of the engine: a refused call must not reach the tool. If this
    /// regressed, `navigate` would launch Chrome and fetch the blocked host anyway.
    #[tokio::test]
    async fn a_denied_call_never_reaches_the_tool() {
        let reg = tool_impls::build_registry();
        let req = json!({
            "jsonrpc": "2.0", "id": 6, "method": "tools/call",
            "params": { "name": "navigate", "arguments": { "url": "https://blocked.test/x" } }
        });
        let resp = handle_request(&reg, &ctx_with_policy(deny_all_policy()), &req)
            .await
            .unwrap();
        assert_eq!(resp["result"]["isError"], true);
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        // Parseable JSON, not a prose error string — a client has to be able to
        // branch on `status` without pattern-matching English.
        let parsed: Value = serde_json::from_str(text).expect("refusal must be valid JSON");
        assert_eq!(parsed["status"], "blocked");
        assert_eq!(parsed["tool"], "navigate");
        assert_eq!(parsed["action_class"], "navigate");
        assert!(parsed["reason"].as_str().unwrap().contains("blocked.test"));
        assert!(!parsed["remedy"].as_str().unwrap().is_empty());
        // The body being a policy payload is itself the proof the tool never ran:
        // `navigate` returns "Navigated to …" or a Chrome error, never this shape.
        assert!(parsed.get("action_class").is_some());
        assert!(!text.contains("Navigated to"));
    }

    #[tokio::test]
    async fn an_allowed_destination_is_not_refused() {
        let reg = tool_impls::build_registry();
        // `status` takes no url and is a Read, so it passes the same engine that
        // refused the call above — proving the denial was the rule, not the wiring.
        let req = json!({
            "jsonrpc": "2.0", "id": 7, "method": "tools/call",
            "params": { "name": "status", "arguments": {} }
        });
        let resp = handle_request(&reg, &ctx_with_policy(deny_all_policy()), &req)
            .await
            .unwrap();
        assert!(resp["result"]["isError"].as_bool() != Some(true));
    }

    /// A refusal must be distinguishable from a malformed request: the call was
    /// well-formed, so it is a tool-level error, not a JSON-RPC protocol error.
    #[tokio::test]
    async fn a_refusal_is_a_tool_error_not_an_rpc_error() {
        let reg = tool_impls::build_registry();
        let req = json!({
            "jsonrpc": "2.0", "id": 8, "method": "tools/call",
            "params": { "name": "navigate", "arguments": { "url": "https://blocked.test/x" } }
        });
        let resp = handle_request(&reg, &ctx_with_policy(deny_all_policy()), &req)
            .await
            .unwrap();
        assert!(resp.get("error").is_none(), "must not be an RPC error");
        assert!(resp["result"].is_object());
    }
}
