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
    let code = format!(
        r#"return (function() {{
            var role = {role}; var textQ = {textq}; var nth = {nth};
            var sel = role ? '[role=' + role + '],button,a,[role=button],[role=link]' : 'button,a,[role=button],[role=link],input[type=submit]';
            var els = Array.from(document.querySelectorAll(sel));
            var matches = els.filter(function(e) {{
                return e.textContent.toLowerCase().indexOf(textQ) !== -1 ||
                       (e.getAttribute('aria-label')||'').toLowerCase().indexOf(textQ) !== -1;
            }});
            var total = matches.length;
            var visible = matches.filter(function(e) {{
                var r = e.getBoundingClientRect();
                if (r.width === 0 || r.height === 0) return false;
                var s = getComputedStyle(e);
                if (s.visibility === 'hidden' || s.display === 'none' || s.opacity === '0') return false;
                // Reject anything inside a collapsed ancestor — the accordion case.
                // Stop before <body>/<html>: those routinely measure zero height with
                // overflow:hidden on sites using fixed or virtualised scrolling, and
                // treating that as "collapsed" would hide every element on the page.
                for (var p = e.parentElement;
                     p && p !== document.body && p !== document.documentElement;
                     p = p.parentElement) {{
                    var pr = p.getBoundingClientRect();
                    if (pr.height === 0 || pr.width === 0) {{
                        if (getComputedStyle(p).overflow !== 'visible') return false;
                    }}
                }}
                return true;
            }});
            if (total === 0)
                return JSON.stringify({{ok: false, error: "no match for: " + {textraw}}});
            if (visible.length === 0)
                return JSON.stringify({{ok: false, matched_total: total, matched_visible: 0,
                    error: "matched " + total + " node(s) for " + {textraw} +
                           ", all hidden or inside a collapsed container"}});
            var target = visible[Math.min(nth, visible.length-1)];
            window.__nbClickTarget = target;
            return JSON.stringify({{ok: true, matched_total: total,
                matched_visible: visible.length,
                text: target.textContent.trim().slice(0,60), nth: nth}});
        }})()"#,
        role = js_lit(role),
        textq = js_lit(&text.to_lowercase()),
        textraw = js_lit(text),
        nth = nth,
    );
    let picked = str_or(page::js(client, &code).await?, r#"{"ok": false}"#);
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
        page::js(client, &code).await?,
        r#"{"dismissed": false}"#,
    ))
}

/// `scroll` — scroll the viewport, then force a frame so load-on-scroll content paints.
pub async fn scroll(client: &CdpClient, direction: &str, amount: i64) -> Result<String, CdpError> {
    match direction {
        "top" => {
            page::js(client, "window.scrollTo(0, 0)").await?;
        }
        "bottom" => {
            page::js(client, "window.scrollTo(0, document.body.scrollHeight)").await?;
        }
        "up" => {
            page::js(client, &format!("window.scrollBy(0, -{amount})")).await?;
        }
        _ => {
            page::js(client, &format!("window.scrollBy(0, {amount})")).await?;
        }
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    page::nudge_frame(client).await; // virtualized lists load-on-scroll via a frame
    let pos = page::js(client, "return window.scrollY").await?;
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
            let count = page::js(client, &expr).await?.as_i64().unwrap_or(0);
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
