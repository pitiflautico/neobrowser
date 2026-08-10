//! MCP protocol (JSON-RPC 2.0 over stdin/stdout).
//!
//! Port of the protocol half of the Python `server.py`: `initialize`, `tools/list`,
//! `tools/call`, and `notifications/initialized`, with the same argument-validation
//! contract and the same 500k-char text cap. Screenshots return native MCP image
//! content instead of the Python string-JSON round-trip.

use std::sync::Arc;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

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
Clicks are real (isTrusted) mouse events.
- Forms: `fill {selector,value}` or `form_fill {fields}` (by label), then `submit`.
- Files: `upload {selector,files}`; `download {url}` (reuses session cookies).

Rendering note: content is force-rendered on read/find/scroll (headless compositor \
is otherwise idle), so prefer those over blind waits.

Search is multi-source and routes around walls: `search` (web), `search_images`, \
`search_videos`.

Tabs: `new_tab`/`list_tabs`/`switch_tab`/`close_tab` — tools act on the active tab.

Real sessions: set NEOBROWSER_REAL_PROFILE to start authenticated; or \
NEOBROWSER_ATTACH_PORT to drive a Chrome you already have open. Act only as the user \
would themselves.";

/// Run the MCP server over stdin/stdout until EOF.
pub async fn serve() {
    let browser = Arc::new(Browser::new());
    let registry = Arc::new(tool_impls::build_registry());
    let ctx = ToolCtx {
        browser,
        registry: registry.clone(),
    };

    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin).lines();
    let mut stdout = tokio::io::stdout();

    while let Ok(Some(line)) = reader.next_line().await {
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
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": SERVER_NAME, "version": VERSION },
                "instructions": INSTRUCTIONS,
            }),
        )),
        "tools/list" => Some(result_response(
            &req_id,
            json!({ "tools": registry.descriptors() }),
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

    let outcome = tool.call(ctx, &args).await;

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
            result_response(
                req_id,
                json!({ "content": [{ "type": "text", "text": text }] }),
            )
        }
        Ok(ToolOutput::Image { data, mime }) => result_response(
            req_id,
            json!({ "content": [{ "type": "image", "data": data, "mimeType": mime }] }),
        ),
        Err(e) => tool_error_response(req_id, &e),
    }
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
        ToolCtx {
            browser: Arc::new(Browser::new()),
            registry: Arc::new(tool_impls::build_registry()),
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
}
