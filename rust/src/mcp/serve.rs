//! The stdio server loop, and shutting it down without lying about it.
//!
//! stdout is the protocol stream, which is why nothing here ever prints to it — a single
//! stray log line corrupts the session for the client. Shutdown is cooperative rather than
//! abrupt: the signal sets a flag that in-flight actions consult through their budget, so a
//! cancelled action reports *cancelled* rather than timing out or, worse, reporting the
//! success it never verified.

//! MCP protocol (JSON-RPC 2.0 over stdin/stdout).
//!
//! Port of the protocol half of the Python `server.py`: `initialize`, `tools/list`,
//! `tools/call`, and `notifications/initialized`, with the same argument-validation
//! contract and the same 500k-char text cap. Screenshots return native MCP image
//! content instead of the Python string-JSON round-trip.

use std::sync::Arc;

use serde_json::Value;
use tokio::io::AsyncWriteExt;

use super::dispatch::{error_response, handle_request, tool_error_response};
use super::{approval_gate, ask_user, ApprovalGate};
use crate::browser::Browser;
use crate::tool_impls;
use crate::tools::{ToolCtx, ToolError};

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
            Ok(req) => {
                // Human approval gate (#12): sensitive tools can require an
                // interactive confirm via MCP elicitation before dispatch.
                match approval_gate(&req) {
                    ApprovalGate::NotNeeded => handle_request(&registry, &ctx, &req).await,
                    ApprovalGate::Unsupported { id, tool } => Some(tool_error_response(
                        &id,
                        &ToolError::Failed(format!(
                            "{tool}: approval required (NEOBROWSER_REQUIRE_APPROVAL) but this client did not advertise elicitation support"
                        )),
                    )),
                    ApprovalGate::Ask { id, tool } => {
                        match ask_user(&mut lines_rx, &mut stdout, &tool).await {
                            true => handle_request(&registry, &ctx, &req).await,
                            false => Some(tool_error_response(
                                &id,
                                &ToolError::Failed(format!("{tool}: declined by the user")),
                            )),
                        }
                    }
                }
            }
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
