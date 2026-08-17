//! What styles actually apply to an element, as opposed to what the stylesheet says.

//! Deep debugging: performance traces with Web Vitals, network waterfalls with
//! bounded response bodies, computed styles, and HAR export.
//!
//! This is the gap against Chrome DevTools MCP. The existing `console_logs` /
//! `network_log` tools answer "what happened"; this module answers "why is it slow"
//! and "what exactly did that request return", which is what a developer debugging
//! their own app actually needs.
//!
//! Everything here is read-only and passes through [`crate::trace::redact`] before it
//! leaves, because a network waterfall is made of headers and query strings — i.e. of
//! session tokens.

use serde_json::Value;

use crate::cdp::{CdpClient, CdpError};

/// `computed_style` — the resolved CSS for one element.
///
/// Answers "why does it look like that", which a DOM dump cannot: the cascade result
/// is what matters and it is not visible in the markup.
pub async fn computed_style(
    client: &CdpClient,
    selector: &str,
    properties: &[String],
) -> Result<String, CdpError> {
    let want = if properties.is_empty() {
        // A useful default rather than all ~340 properties, which would blow the
        // context budget for no benefit.
        vec![
            "display",
            "position",
            "visibility",
            "opacity",
            "z-index",
            "width",
            "height",
            "color",
            "background-color",
            "font-size",
            "font-family",
            "overflow",
            "pointer-events",
        ]
        .into_iter()
        .map(String::from)
        .collect()
    } else {
        properties.to_vec()
    };
    let snippet = crate::js::computed_style()
        .with(
            "SEL",
            &serde_json::to_string(selector).unwrap_or_else(|_| "\"\"".into()),
        )
        .with(
            "PROPS",
            &serde_json::to_string(&want).unwrap_or_else(|_| "[]".into()),
        );
    let raw = crate::page::js(client, &snippet.returning()).await?;
    Ok(match raw {
        Value::String(s) => s,
        other => other.to_string(),
    })
}
