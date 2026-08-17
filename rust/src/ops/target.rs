//! Getting to the element you meant, and clearing whatever is in the way.
//!
//! The gap between "click the Accept button" and a click that lands is mostly obstruction:
//! the element is below the fold, or hasn't rendered yet, or a consent banner is sitting on
//! top of it. So these belong together — scroll to it, wait for it, dismiss what covers it,
//! then click it. `dismiss_overlay` is deliberately conservative about what counts as an
//! overlay, because dismissing something that was not an overlay destroys page state that
//! the caller cannot get back.

use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::cdp::{CdpClient, CdpError};
use crate::page;

use super::{bounded_ms_i64, js_lit, str_or};

/// `find_and_click` — click the nth **visible** clickable whose text/aria-label
/// contains `text`.
///
/// Visibility is not cosmetic here. Multi-step forms (accordions, wizards,
/// header panels duplicating a body form) keep collapsed steps in the DOM at
/// `height: 0`, so a plain text match happily returns a button nobody can see
/// and the click goes to the wrong step — silently, with `ok: true`. Hidden
/// candidates are filtered out and still counted, so the caller can tell
/// "nothing matched" from "everything that matched was hidden".
///
/// The click itself is delegated to `page::click_backend_node` rather than a JS
/// `.click()`, so it is a real isTrusted mouse event and inherits the
/// scroll-into-view and overlay hit-test from there.
pub async fn find_and_click(
    client: &CdpClient,
    text: &str,
    role: &str,
    nth: i64,
) -> Result<String, CdpError> {
    let code = crate::js::find_and_click()
        .with("ROLE", &js_lit(role))
        .with("TEXTQ", &js_lit(&text.to_lowercase()))
        .with("TEXTRAW", &js_lit(text))
        .with("NTH", &nth.to_string())
        .returning();
    let picked = str_or(page::eval_body(client, &code).await?, r#"{"ok": false}"#);
    let mut report: Value = serde_json::from_str(&picked)
        .unwrap_or_else(|_| json!({ "ok": false, "error": "find_and_click: bad selection" }));
    if report.get("ok").and_then(Value::as_bool) != Some(true) {
        return Ok(report.to_string());
    }

    // Hand the chosen node to the real mouse-click path.
    let outcome = match page::click_stashed_node(client, "__nbClickTarget").await? {
        Some(o) => o,
        None => {
            report["ok"] = json!(false);
            report["error"] = json!("find_and_click: element vanished before the click");
            return Ok(report.to_string());
        }
    };
    match outcome {
        page::ClickOutcome::Clicked => {}
        page::ClickOutcome::NoLayoutUsedJs => {
            report["note"] = json!("clicked via JS fallback (no box model)");
        }
        page::ClickOutcome::NotFound => {
            report["ok"] = json!(false);
            report["error"] = json!("find_and_click: element vanished before the click");
        }
        page::ClickOutcome::Disabled { reason } => {
            report["ok"] = json!(false);
            report["error"] = json!(format!("find_and_click: {reason}"));
            // The actionable instruction, because retrying is the one thing that cannot work.
            report["hint"] = json!(
                "change whatever keeps the control disabled — a required field, a pending \
                 validation — rather than retrying the click"
            );
        }
        page::ClickOutcome::Obscured { by } => {
            report["ok"] = json!(false);
            report["error"] = json!(format!(
                "matched a visible element but it is covered by {by}. \
                 Dismiss the overlay (dismiss_overlay), then retry."
            ));
        }
    }
    Ok(report.to_string())
}

/// Loaded from `js/dismiss_overlay.js`. See [`crate::js`] for why the snippets live
/// in real JavaScript files rather than Rust string literals.
pub(super) fn dismiss_overlay_js() -> &'static str {
    include_str!("../../js/dismiss_overlay.js")
}

pub async fn dismiss_overlay(client: &CdpClient, force: bool) -> Result<String, CdpError> {
    let code = dismiss_overlay_js().replace("FORCE", if force { "true" } else { "false" });
    Ok(str_or(
        page::eval_body(client, &code).await?,
        r#"{"dismissed": false}"#,
    ))
}

/// `scroll` — scroll the viewport, then force a frame so load-on-scroll content paints.
pub async fn scroll(client: &CdpClient, direction: &str, amount: i64) -> Result<String, CdpError> {
    match direction {
        "top" => {
            page::eval_expr(client, "window.scrollTo(0, 0)").await?;
        }
        "bottom" => {
            page::eval_expr(client, "window.scrollTo(0, document.body.scrollHeight)").await?;
        }
        "up" => {
            page::eval_expr(client, &format!("window.scrollBy(0, -{amount})")).await?;
        }
        _ => {
            page::eval_expr(client, &format!("window.scrollBy(0, {amount})")).await?;
        }
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    page::nudge_frame(client).await; // virtualized lists load-on-scroll via a frame
    let pos = page::eval_body(client, "return window.scrollY").await?;
    let pos = pos.as_f64().unwrap_or(0.0) as i64;
    Ok(json!({ "scrolled": direction, "amount": amount, "scrollY": pos }).to_string())
}

/// `wait` — sleep `ms`, or poll until `selector` appears (whichever `selector` implies).
pub async fn wait(client: &CdpClient, ms: i64, selector: Option<&str>) -> Result<String, CdpError> {
    let ms = bounded_ms_i64(ms);
    if let Some(sel) = selector {
        let deadline = Instant::now() + Duration::from_millis(ms);
        let mut found = false;
        while Instant::now() < deadline {
            let expr = format!("return document.querySelectorAll({}).length", js_lit(sel));
            let count = page::eval_body(client, &expr).await?.as_i64().unwrap_or(0);
            if count > 0 {
                found = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        Ok(json!({ "found": found, "selector": sel, "waited_ms": ms }).to_string())
    } else {
        tokio::time::sleep(Duration::from_millis(ms)).await;
        Ok(format!("Waited {ms}ms"))
    }
}
