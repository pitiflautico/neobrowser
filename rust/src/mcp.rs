//! MCP protocol (JSON-RPC 2.0 over stdin/stdout).
//!
//! Port of the protocol half of the Python `server.py`: `initialize`, `tools/list`,
//! `tools/call`, and `notifications/initialized`, with the same argument-validation
//! contract and the same 500k-char text cap. Screenshots return native MCP image
//! content instead of the Python string-JSON round-trip.

use std::sync::Arc;

use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;

use crate::browser::Browser;
use crate::tool_impls;
use crate::tools::{Registry, ToolCtx, ToolError, ToolOutput};

const SERVER_NAME: &str = "neobrowser";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const PROTOCOL_VERSION: &str = "2024-11-05";
const MAX_TEXT: usize = 500_000;

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

/// Run the MCP server over stdin/stdout until EOF or a termination signal.
pub async fn serve() {
    let browser = Arc::new(Browser::new());
    let registry = Arc::new(tool_impls::build_registry());
    let policy = Arc::new(crate::policy::Policy::from_env());
    // One trace per server process, so every action, refusal and wall in this session
    // correlates under a single id.
    let trace = Arc::new(crate::trace::Trace::new(new_trace_id()));
    // Announced at startup: an operator reading the log must be able to tell which
    // rules are in force without inferring it from a later denial.
    tracing::info!(
        profile = policy.profile.label(),
        allow = ?policy.allow_list(),
        deny = ?policy.deny_list(),
        "policy engine active"
    );
    tracing::info!(trace_id = trace.trace_id(), "session trace started");

    // The HTTP transport, when configured, runs alongside stdio. Each HTTP session gets
    // its own browser, so it never shares this stdio session's profile or cookies.
    if let Some((bind, port)) = crate::http_transport::configured() {
        let transport = crate::http_transport::HttpTransport::new(bind, port, registry.clone());
        match crate::sessions::write_private(
            &crate::http_transport::token_path(),
            transport.token(),
        ) {
            Ok(()) => tracing::info!(
                token_file = %crate::http_transport::token_path().display(),
                "MCP HTTP transport enabled; run `neobrowser http token` for the bearer token"
            ),
            Err(e) => tracing::warn!(error = %e, "could not write the HTTP token file"),
        }
        tokio::spawn(async move {
            if let Err(e) = crate::http_transport::serve(transport).await {
                tracing::warn!(error = %e, port, "the HTTP transport could not listen");
            }
        });
    }

    // The bridge is opt-in and runs alongside the stdio transport. Spawned rather than
    // awaited: it serves the extension for the whole session while MCP requests keep
    // flowing on stdin.
    let bridge = crate::bridge::configured_port().map(|port| {
        let bridge = crate::bridge::Bridge::new(port);
        match bridge.write_token_file() {
            Ok(path) => tracing::info!(
                port,
                token_file = %path.display(),
                "bridge enabled; run `neobrowser bridge token` and paste the value into \
                 the extension popup"
            ),
            Err(e) => tracing::warn!(error = %e, "could not write the bridge token file"),
        }
        tokio::spawn({
            let bridge = bridge.clone();
            async move {
                if let Err(e) = crate::bridge::serve(bridge).await {
                    tracing::warn!(error = %e, port, "bridge could not listen");
                }
            }
        });
        bridge
    });
    let ctx = ToolCtx {
        browser,
        registry: registry.clone(),
        policy,
        trace: trace.clone(),
        bridge,
    };

    // Read stdin on a plain std thread instead of `tokio::io::stdin()`: tokio's
    // stdin leaves a permanently-blocked blocking task that prevents the runtime
    // (and thus the whole process) from exiting after a signal-triggered
    // shutdown — the server would hang until stdin EOF. A detached std thread
    // does not block process exit.
    let (lines_tx, mut lines_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    std::thread::spawn(move || {
        use std::io::BufRead;
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            match line {
                Ok(l) => {
                    if lines_tx.send(l).is_err() {
                        return; // server shutting down
                    }
                }
                Err(_) => return, // stdin error: close the channel (EOF path)
            }
        }
    });
    let mut stdout = tokio::io::stdout();

    // Registered ONCE, before the loop. The previous version called
    // `shutdown_signal()` inside `select!`, which builds a fresh signal
    // registration on every request and drops it again — churn on a
    // process-global resource, and a handler whose lifetime is a single
    // iteration rather than the process.
    let mut shutdown = std::pin::pin!(shutdown_signal());

    loop {
        // Race the next request line against SIGTERM/SIGINT: MCP clients kill
        // their servers with SIGTERM on exit, and without handling it the
        // headless Chrome outlived the server (orphaned processes).
        let line = tokio::select! {
            line = lines_rx.recv() => match line {
                Some(l) => l,
                None => break, // stdin EOF
            },
            _ = &mut shutdown => {
                // Flagged before breaking: any action still waiting bounds itself with a
                // `Budget`, and `Budget::expired` consults this flag — so setting it here
                // cancels every in-flight wait cooperatively instead of leaving the
                // process to finish a 30-second navigation before it can exit.
                crate::action::begin_shutdown();
                tracing::info!("termination signal received; cancelling in-flight waits");
                break;
            }
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(line) {
            Ok(req) => handle_request(&registry, &ctx, &req).await,
            Err(e) => Some(error_response(
                &Value::Null,
                -32700,
                &format!("Parse error: {e}"),
            )),
        };
        if let Some(resp) = response {
            let mut buf = serde_json::to_string(&resp).unwrap_or_default();
            buf.push('\n');
            if stdout.write_all(buf.as_bytes()).await.is_err() {
                break;
            }
            let _ = stdout.flush().await;
        }
    }

    // Clean shutdown: never leak a headless Chrome.
    ctx.browser.shutdown().await;

    // The bundle is written on the way out rather than per event: an agent run is
    // only diagnosable as a whole, and writing incrementally would put a disk write
    // on every action's critical path.
    if !trace.is_empty() {
        match trace.write_bundle() {
            Ok(path) => tracing::info!(
                trace_id = trace.trace_id(),
                path = %path.display(),
                "wrote evidence bundle; inspect with `neobrowser trace open <id>`"
            ),
            Err(e) => tracing::warn!(error = %e, "could not write the evidence bundle"),
        }
    }
}

/// MCP protocol versions this server can speak, newest first.
const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// Pick the protocol version to answer with, and record any declared roots.
///
/// The rule from the MCP spec: echo the client's version when we support it, otherwise
/// answer with our preferred one and let the client decide whether it can proceed.
fn negotiate_protocol_version(params: &Value) -> String {
    // Roots arrive in the same handshake, so this is the one place they can be captured
    // before any tool runs.
    if let Some(roots) = params
        .get("capabilities")
        .and_then(|c| c.get("roots"))
        .and_then(|r| r.get("roots"))
        .and_then(Value::as_array)
    {
        let paths: Vec<std::path::PathBuf> = roots
            .iter()
            .filter_map(|r| r.get("uri").and_then(Value::as_str))
            // Only file:// roots mean anything for filesystem access; an http root is
            // not a directory and must not be treated as one.
            .filter_map(|uri| uri.strip_prefix("file://"))
            .map(std::path::PathBuf::from)
            .collect();
        if !paths.is_empty() {
            tracing::info!(roots = ?paths, "client declared MCP roots; upload is scoped to them");
            crate::reach::set_mcp_roots(paths);
        }
    }

    let requested = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or("");
    if SUPPORTED_PROTOCOL_VERSIONS.contains(&requested) {
        return requested.to_string();
    }
    if !requested.is_empty() {
        tracing::info!(
            requested,
            offering = PROTOCOL_VERSION,
            "client asked for an unsupported MCP protocol version; offering ours"
        );
    }
    PROTOCOL_VERSION.to_string()
}

/// A session trace id: `trace_<millis>_<counter>`.
///
/// Time-prefixed so bundles sort chronologically in a directory listing, and
/// counter-suffixed so two servers started in the same millisecond do not collide.
fn new_trace_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("trace_{millis}_{}", N.fetch_add(1, Ordering::Relaxed))
}

/// Resolve on SIGINT (all platforms) or SIGTERM (unix) — the normal ways an MCP
/// client (Claude Desktop, Cursor) terminates its server.
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = term.recv() => {}
                }
            }
            // No SIGTERM handler: fall back to ctrl_c only.
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// Handle one JSON-RPC request. Returns `Some(response)` or `None` for notifications.
pub async fn handle_request(registry: &Registry, ctx: &ToolCtx, req: &Value) -> Option<Value> {
    let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let req_id = req.get("id").cloned().unwrap_or(Value::Null);
    let params = req.get("params").cloned().unwrap_or(Value::Null);

    match method {
        "initialize" => Some(result_response(
            &req_id,
            json!({
                // Negotiated, not asserted: if the client names a version we support,
                // answer with theirs. Replying with a fixed version a client did not ask
                // for is how a compatible pair fails to connect.
                "protocolVersion": negotiate_protocol_version(&params),
                "capabilities": { "tools": {} },
                "serverInfo": { "name": SERVER_NAME, "version": VERSION },
                "instructions": INSTRUCTIONS,
            }),
        )),
        "tools/list" => Some(result_response(
            &req_id,
            json!({ "tools": registry.descriptors_for(crate::tools::Toolset::from_env()) }),
        )),
        "tools/call" => Some(handle_tool_call(registry, ctx, &req_id, &params).await),
        "notifications/initialized" => None,
        _ => {
            if req.get("id").is_some() {
                Some(error_response(
                    &req_id,
                    -32601,
                    &format!("Unknown method: {method}"),
                ))
            } else {
                None
            }
        }
    }
}

async fn handle_tool_call(
    registry: &Registry,
    ctx: &ToolCtx,
    req_id: &Value,
    params: &Value,
) -> Value {
    let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let empty = serde_json::Map::new();
    let args = params
        .get("arguments")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or(empty);

    let tool = match registry.get(tool_name) {
        Some(t) => t.clone(),
        None => {
            return error_response(req_id, -32601, &format!("Unknown tool: {tool_name}"));
        }
    };

    // Validate before dispatch — a bad param is a caller error, not a server fault.
    if let Err(e) = tool.spec().validate_args(&args) {
        return tool_error_response(req_id, &e);
    }

    // Policy is evaluated after validation (so the target host is parsed from
    // already-well-formed arguments) and before execution — nothing runs that the
    // policy would refuse. A refusal is a tool-level error, not a protocol error:
    // the call was legal, the action was not, and the model needs to read why.
    let class = crate::policy::classify(tool_name);
    let target = crate::policy::target_host(&args);
    let decision = ctx.policy.evaluate(class, target.as_deref());
    if let Some(payload) = decision.to_payload(tool_name, class) {
        tracing::warn!(
            trace_id = ctx.trace.trace_id(),
            tool = tool_name,
            class = class.label(),
            target = target.as_deref().unwrap_or("-"),
            "policy refused a call"
        );
        ctx.trace.record(
            "policy_refusal",
            None,
            None,
            json!({
                "tool": tool_name,
                "class": class.label(),
                "target": target.clone(),
                "payload": serde_json::from_str::<Value>(&payload).unwrap_or(Value::Null),
            }),
        );
        return policy_refusal_response(req_id, &payload);
    }
    if class.is_elevated() {
        // Allowed, but recorded: the audit trail of who touched credentials, files
        // or arbitrary script is what makes an incident reconstructable later.
        tracing::info!(
            tool = tool_name,
            class = class.label(),
            target = target.as_deref().unwrap_or("-"),
            "elevated action permitted"
        );
    }

    let outcome = tool.call(ctx, &args).await;

    // Recorded for every call, not only failures: a timeline with the successes
    // missing cannot show where a run diverged from what was intended.
    ctx.trace.record(
        "tool_call",
        None,
        None,
        json!({
            "tool": tool_name,
            "class": class.label(),
            "target": target,
            // The full URL, not just the host: a redirect chain and a query string are
            // the evidence that makes a run reconstructable. It passes through
            // `trace::redact` on the way in, which is why recording it is safe.
            "url": args.get("url").and_then(Value::as_str),
            "ok": outcome.is_ok(),
        }),
    );

    // Record mutating actions into the active playbook (if any) on success.
    if outcome.is_ok()
        && crate::playbook::is_recordable(tool_name)
        && ctx.browser.is_recording().await
    {
        ctx.browser
            .record_step(tool_name, &Value::Object(args.clone()))
            .await;
    }

    match outcome {
        Ok(ToolOutput::Text(mut text)) => {
            if text.len() > MAX_TEXT {
                let original = text.len();
                text.truncate(MAX_TEXT);
                text.push_str(&format!("\n... (truncated from {original} chars)"));
            }
            // MCP `structuredContent` in addition to the text, for tools whose output
            // is already JSON — which is now every action, every policy refusal and
            // every observation. A client that understands it can branch on
            // `status`/`ok` without re-parsing a string; one that does not still
            // reads the same text as before, so this is additive.
            let mut result = json!({ "content": [{ "type": "text", "text": text.clone() }] });
            if let Ok(Value::Object(structured)) = serde_json::from_str::<Value>(&text) {
                result["structuredContent"] = Value::Object(structured);
            }
            result_response(req_id, result)
        }
        Ok(ToolOutput::Image { data, mime }) => result_response(
            req_id,
            json!({ "content": [{ "type": "image", "data": data, "mimeType": mime }] }),
        ),
        Err(e) => tool_error_response(req_id, &e),
    }
}

/// A policy refusal: `isError` so the client knows the action did not happen, but the
/// body is the payload verbatim.
///
/// It does not go through `tool_error_response` on purpose — that prefixes `Error: `,
/// which would leave the JSON unparseable and defeat the reason for making the
/// refusal structured in the first place.
fn policy_refusal_response(req_id: &Value, payload: &str) -> Value {
    result_response(
        req_id,
        json!({
            "content": [{ "type": "text", "text": payload }],
            "isError": true,
        }),
    )
}

fn tool_error_response(req_id: &Value, err: &ToolError) -> Value {
    result_response(
        req_id,
        json!({
            "content": [{ "type": "text", "text": format!("Error: {err}") }],
            "isError": true,
        }),
    )
}

fn result_response(req_id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": req_id, "result": result })
}

fn error_response(req_id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": req_id, "error": { "code": code, "message": message } })
}

#[cfg(test)]
mod tests {
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
