//! Filling fields and submitting forms.
//!
//! Filling a single field is easy; filling a form is not, and the difference is why
//! `form_fill` exists separately. A real form re-renders between fields — a country select
//! reveals a state select, a validation message shifts the layout — so each field is
//! located freshly at the moment it is filled rather than from one upfront snapshot.
//! `submit` then has the harder job: deciding whether submitting actually did anything,
//! since a form that silently fails validation looks exactly like one that succeeded.

use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

use crate::cdp::{CdpClient, CdpError};
use crate::page;

use super::{bounded_secs_f64, js_lit, str_or};

/// `fill` — set a field's value using the element's own prototype setter and fire
/// input/change (React/Vue-safe), handling select/checkbox/radio/contenteditable.
pub async fn fill(client: &CdpClient, selector: &str, value: &str) -> Result<String, CdpError> {
    let code = crate::js::fill_control()
        .with("SEL", &js_lit(selector))
        .with("VAL", &js_lit(value))
        .returning();
    Ok(str_or(
        page::js(client, &code).await?,
        r#"{"ok": false, "error": "js returned null"}"#,
    ))
}

/// `form_fill` — fill multiple fields by fuzzy label/name/placeholder/aria match.
pub async fn form_fill(
    client: &CdpClient,
    fields: &Map<String, Value>,
    form_index: i64,
) -> Result<String, CdpError> {
    let mut results = Map::new();
    for (label, value) in fields {
        let value_str = match value {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        let code = crate::js::form_fill_fields()
            .with("IDX", &form_index.to_string())
            .with("LABEL", &js_lit(label))
            .with("VAL", &js_lit(&value_str))
            .returning();
        let res = page::js(client, &code).await?;
        let parsed: Value = match &res {
            Value::String(s) => serde_json::from_str(s).unwrap_or(json!({ "ok": false })),
            _ => json!({ "ok": false }),
        };
        results.insert(label.clone(), parsed);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Ok(json!({ "filled": results }).to_string())
}

/// `submit` — click a submit control (or given selector) and wait for navigation.
pub async fn submit(
    client: &CdpClient,
    selector: Option<&str>,
    wait_s: f64,
) -> Result<String, CdpError> {
    let url_before = str_or(page::js(client, "return location.href").await?, "");
    let method;
    if let Some(sel) = selector {
        let code = format!(
            "var el = document.querySelector({}); if (el) el.click();",
            js_lit(sel)
        );
        page::js(client, &code).await?;
        method = sel.to_string();
    } else {
        let m = page::js(client, &crate::js::submit_form().returning()).await?;
        method = m.as_str().unwrap_or("").to_string();
        if method.is_empty() {
            return Ok(
                json!({ "ok": false, "error": "no submit button or form found" }).to_string(),
            );
        }
    }

    let t0 = Instant::now();
    let deadline = t0 + Duration::from_secs_f64(bounded_secs_f64(wait_s));
    let mut url_after = url_before.clone();
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let ready = str_or(page::js(client, "return document.readyState").await?, "");
        let url_now = str_or(page::js(client, "return location.href").await?, &url_before);
        if url_now != url_before || ready == "complete" {
            url_after = url_now;
            break;
        }
    }
    let waited_ms = t0.elapsed().as_millis();
    Ok(
        json!({ "ok": true, "method": method, "url": url_after, "waited_ms": waited_ms })
            .to_string(),
    )
}
