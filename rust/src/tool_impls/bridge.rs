//! The Chrome Bridge: driving tabs the user explicitly shared from their own
//! browser. See `extension/README.md` for the security model.
//!
//! Split out of a single 2700-line `tool_impls.rs`: at 67 tools that file had
//! stopped being navigable, and a reviewer could not tell which tools a change
//! touched.

use async_trait::async_trait;
use serde_json::{json, Map, Value};

use crate::tools::{ParamSpec, ParamType, Tool, ToolCtx, ToolError, ToolOutput, ToolSpec};

use super::arg_str;

pub struct BridgeStatusTool;

#[async_trait]
impl Tool for BridgeStatusTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "bridge_status",
            description: "Whether the NeoBrowser Bridge extension is connected, and which tabs the user has shared with this agent. Use it before assuming a bridge tab is drivable: sharing is per-tab and the user can revoke at any time.",
            params: vec![],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        _args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        match &ctx.bridge {
            None => Ok(ToolOutput::text(
                json!({
                    "enabled": false,
                    "hint": "set NEOBROWSER_BRIDGE_PORT=9333 and restart to enable the bridge, then load extension/ in chrome://extensions",
                })
                .to_string(),
            )),
            Some(bridge) => Ok(ToolOutput::text(
                json!({
                    "enabled": true,
                    "port": bridge.port(),
                    "extension_connected": bridge.is_connected().await,
                    "shared_tabs": bridge.shared_tabs().await,
                    "token_file": crate::bridge::token_path().display().to_string(),
                    "hint": "the user shares tabs from the extension popup; nothing is drivable until they do",
                })
                .to_string(),
            )),
        }
    }
}

pub struct BridgeCdpTool;

#[async_trait]
impl Tool for BridgeCdpTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "bridge_cdp",
            description: "Send one CDP command to a tab the user shared through the bridge extension — their real browser, their real session, nothing cloned. The extension refuses any tab that is not currently shared, so this cannot reach the rest of their browser.",
            params: vec![
                ParamSpec::new("tab_id", ParamType::Integer, "Chrome tab id, from bridge_status.shared_tabs").required(),
                ParamSpec::new("method", ParamType::String, "CDP method, e.g. Runtime.evaluate").required(),
                ParamSpec::new("params", ParamType::Object, "CDP params object"),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let Some(bridge) = &ctx.bridge else {
            return Err(ToolError::Failed(
                "the bridge is not enabled. Set NEOBROWSER_BRIDGE_PORT and restart".into(),
            ));
        };
        let tab_id = args
            .get("tab_id")
            .and_then(Value::as_i64)
            .ok_or_else(|| ToolError::Argument("bridge_cdp: tab_id must be an integer".into()))?;
        let method = arg_str(args, "method")
            .ok_or_else(|| ToolError::Argument("bridge_cdp: method must be a string".into()))?;
        let params = args.get("params").cloned().unwrap_or(json!({}));

        match bridge.send(tab_id, method, params).await {
            Ok(result) => Ok(ToolOutput::text(
                json!({ "ok": true, "result": result }).to_string(),
            )),
            // A refusal from the extension (tab not shared) is a normal outcome the
            // model should read and act on, not a server fault.
            Err(e) => Ok(ToolOutput::text(
                json!({ "ok": false, "error": e }).to_string(),
            )),
        }
    }
}

// --- C4 composite actions + D1 profile modes -----------------------------------
