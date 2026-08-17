//! Asking the page what it is: evaluate, summarise, analyse, debug.
//!
//! These are the read-only verbs, and they exist in four sizes because "what is on this
//! page" has four useful answers. `eval_js` is the escape hatch, `page_info` the cheap
//! summary, `analyze` the structural survey a model uses to decide what to do next, and
//! `debug` the answer to "why did that not work" — console errors, failed requests, and
//! the things a screenshot cannot show.

use serde_json::{json, Value};

use crate::cdp::{CdpClient, CdpError};
use crate::page;

use super::str_or;

/// `js` tool — evaluate arbitrary page JS and return the value (string passthrough).
pub async fn eval_js(client: &CdpClient, code: &str) -> Result<String, CdpError> {
    let v = page::js(client, code).await?;
    Ok(match v {
        Value::String(s) => s,
        other => other.to_string(),
    })
}

/// Loaded from `js/page_info.js`. See [`crate::js`] for why the snippets live
/// in real JavaScript files rather than Rust string literals.
fn page_info_js() -> &'static str {
    include_str!("../../js/page_info.js")
}

pub async fn page_info(client: &CdpClient) -> Result<String, CdpError> {
    Ok(str_or(page::js(client, page_info_js()).await?, "{}"))
}

/// Loaded from `js/analyze.js`. See [`crate::js`] for why the snippets live
/// in real JavaScript files rather than Rust string literals.
fn analyze_js() -> &'static str {
    include_str!("../../js/analyze.js")
}

pub async fn analyze(client: &CdpClient) -> Result<String, CdpError> {
    Ok(str_or(page::js(client, analyze_js()).await?, "{}"))
}

/// `debug` — install/flush/remove an in-page console interceptor.
pub async fn debug(client: &CdpClient, action: &str) -> Result<String, CdpError> {
    match action {
        "start" => {
            page::js(
                client,
                r#"if (!window.__neo_debug_logs) window.__neo_debug_logs = [];
                window.__neo_debug_orig = {log: console.log, warn: console.warn, error: console.error};
                ['log','warn','error'].forEach(function(l) {
                    console[l] = function() {
                        var msg = Array.from(arguments).map(function(a){ try{return JSON.stringify(a);}catch(e){return String(a);} }).join(' ');
                        window.__neo_debug_logs.push({level: l, msg: msg, t: Date.now()});
                        window.__neo_debug_orig[l].apply(console, arguments);
                    };
                });"#,
            )
            .await?;
            Ok(json!({ "ok": true, "action": "interceptor_installed" }).to_string())
        }
        "stop" => {
            page::js(
                client,
                r#"if (window.__neo_debug_orig) {
                    console.log = window.__neo_debug_orig.log;
                    console.warn = window.__neo_debug_orig.warn;
                    console.error = window.__neo_debug_orig.error;
                    delete window.__neo_debug_orig;
                }
                window.__neo_debug_logs = [];"#,
            )
            .await?;
            Ok(json!({ "ok": true, "action": "interceptor_removed" }).to_string())
        }
        _ => {
            // flush (default)
            Ok(str_or(
                page::js(
                    client,
                    "var logs = window.__neo_debug_logs || []; window.__neo_debug_logs = []; return JSON.stringify(logs);",
                )
                .await?,
                "[]",
            ))
        }
    }
}
