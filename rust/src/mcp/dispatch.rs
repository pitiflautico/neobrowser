//! Routing a JSON-RPC request to a tool, and shaping what comes back.
//!
//! Every call passes the policy engine before it runs, and a refusal is a *result* rather
//! than a transport error, so a model can read why it was refused and choose differently.
//! Results carry `structuredContent` alongside the text form, because the text is for a human
//! reading a transcript and the structure is for the client.

//! MCP protocol (JSON-RPC 2.0 over stdin/stdout).
//!
//! Port of the protocol half of the Python `server.py`: `initialize`, `tools/list`,
//! `tools/call`, and `notifications/initialized`, with the same argument-validation
//! contract and the same 500k-char text cap. Screenshots return native MCP image
//! content instead of the Python string-JSON round-trip.

use serde_json::{json, Value};

use super::protocol::negotiate_protocol_version;
use super::{set_elicitation_supported, INSTRUCTIONS, MAX_TEXT, SERVER_NAME, VERSION};
use crate::tools::{Registry, ToolCtx, ToolError, ToolOutput};

/// Handle one JSON-RPC request. Returns `Some(response)` or `None` for notifications.
pub async fn handle_request(registry: &Registry, ctx: &ToolCtx, req: &Value) -> Option<Value> {
    let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let req_id = req.get("id").cloned().unwrap_or(Value::Null);
    let params = req.get("params").cloned().unwrap_or(Value::Null);

    match method {
        "initialize" => {
            let supports = req
                .get("params")
                .and_then(|p| p.get("capabilities"))
                .and_then(|c| c.get("elicitation"))
                .is_some();
            set_elicitation_supported(supports);
            Some(result_response(
                &req_id,
                json!({
                    // Negotiated, not asserted: if the client names a version we support,
                    // answer with theirs. Replying with a fixed version a client did not ask
                    // for is how a compatible pair fails to connect.
                    "protocolVersion": negotiate_protocol_version(&params),
                    "capabilities": { "tools": {}, "elicitation": {} },
                    "serverInfo": { "name": SERVER_NAME, "version": VERSION },
                    "instructions": INSTRUCTIONS,
                }),
            ))
        }
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

    let call_start = std::time::Instant::now();
    let outcome = tool.call(ctx, &args).await;

    // Durable audit trail (append-only, secrets masked) — never breaks a call.
    crate::audit::log_tool_call(
        tool_name,
        &args,
        outcome.is_ok(),
        outcome.as_ref().err().map(|e| e.to_string()).as_deref(),
        call_start.elapsed(),
    );

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

pub(super) fn tool_error_response(req_id: &Value, err: &ToolError) -> Value {
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

pub(super) fn error_response(req_id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": req_id, "error": { "code": code, "message": message } })
}
