//! Navigation, settling, and reading the page once it has settled.
//!
//! The distinction that matters here is between *arriving* and *being ready*. Chrome fires
//! its load events long before a modern page has finished rendering, so `navigate_budgeted`
//! spends whatever remains of its budget on `settle_dom` rather than returning the moment
//! the network goes quiet. A tool that returns too early reports an empty page as an empty
//! page — the most expensive kind of wrong answer, because it looks like a fact.

use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::cdp::{CdpClient, CdpError};

use super::eval::nudge_frame;

use super::eval::eval_body;

/// Navigate to `url` and wait for the page to be usable, bounded by `budget`.
///
/// Returns whether the load actually completed. A `false` means the budget ran out first —
/// the caller reports that rather than presenting a half-loaded page as ready.
///
/// Two things differ from a naive implementation, both learned the hard way. The wait is
/// bounded by a caller-supplied [`crate::action::Budget`] instead of a hardcoded 15s, so a
/// slow site can no longer burn a fixed quarter-minute regardless of what the caller had
/// time for. And the fixed post-load sleep is gone: `readyState === "complete"` is followed
/// by a *condition* wait for the DOM to stop growing, which returns immediately on a static
/// page and only pays for hydration when hydration is actually happening.
pub async fn navigate_budgeted(
    client: &CdpClient,
    url: &str,
    budget: &crate::action::Budget,
) -> Result<bool, CdpError> {
    client.send("Page.navigate", json!({ "url": url })).await?;

    // Bounded backoff, never past the deadline.
    let mut interval = Duration::from_millis(50);
    let mut complete = false;
    while !budget.expired() {
        if let Ok(Value::String(state)) = eval_body(client, "return document.readyState").await {
            if state == "complete" {
                complete = true;
                break;
            }
        }
        tokio::time::sleep(budget.capped_at(interval)).await;
        interval = (interval * 2).min(Duration::from_millis(400));
    }

    if complete {
        settle_dom(client, budget).await;
    }
    // Force frames so any above-the-fold deferred content paints before tools read.
    nudge_frame(client).await;
    Ok(complete)
}

/// Wait for the DOM to stop changing: two consecutive identical element counts, or the
/// budget, whichever comes first.
///
/// This replaces a blind `sleep(wait_s)`. On a static page the second sample matches the
/// first and it costs one round trip; on a hydrating SPA it keeps sampling until the tree
/// settles. Capped at 2s of its own so a page that mutates forever — a carousel, a live
/// ticker — cannot hold the navigation open.
async fn settle_dom(client: &CdpClient, budget: &crate::action::Budget) {
    let own_deadline = Instant::now() + Duration::from_secs(2);
    let mut last: Option<f64> = None;
    while !budget.expired() && Instant::now() < own_deadline {
        let count = eval_body(client, "return document.getElementsByTagName('*').length")
            .await
            .ok()
            .and_then(|v| v.as_f64());
        match (last, count) {
            (Some(prev), Some(now)) if prev == now => return,
            (_, Some(now)) => last = Some(now),
            // Cannot read the page: stop waiting rather than spinning on errors.
            (_, None) => return,
        }
        tokio::time::sleep(budget.capped_at(Duration::from_millis(120))).await;
    }
}

/// Backwards-compatible wrapper: `wait_s` becomes the budget.
///
/// Kept so older call sites keep working while tools move to explicit budgets. The budget is
/// the LARGER of `wait_s` and 15s, so this wrapper cannot make an existing caller *more*
/// likely to time out than before.
pub async fn navigate(client: &CdpClient, url: &str, wait_s: f64) -> Result<(), CdpError> {
    let budget = crate::action::Budget::from_secs(wait_s.max(15.0));
    navigate_budgeted(client, url, &budget).await?;
    Ok(())
}

/// The current page URL.
pub async fn current_url(client: &CdpClient) -> Result<String, CdpError> {
    let v = eval_body(client, "return location.href").await?;
    Ok(v.as_str().unwrap_or("").to_string())
}

/// Visible text of `selector` (defaults to `body`), trimmed.
///
/// Returns an explicit marker for PDF documents, because Chrome renders them in
/// its own viewer and the text is not part of the DOM. Returning an empty string
/// here would make a PDF indistinguishable from a page with no content.
pub async fn read_text(client: &CdpClient, selector: &str) -> Result<String, CdpError> {
    read_text_with_options(client, selector, false).await
}

/// Visible text of `selector`, optionally with links rendered as `[text](href)`.
pub async fn read_text_with_options(
    client: &CdpClient,
    selector: &str,
    include_links: bool,
) -> Result<String, CdpError> {
    // Materialize deferred/virtualized content before reading it.
    nudge_frame(client).await;

    // PDF detection: Chrome's PDF viewer does not expose the text in the DOM,
    // so we download the file and extract the text with pdftotext.
    let content_type = eval_body(client, "return document.contentType || ''")
        .await
        .ok()
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_default();
    if content_type == "application/pdf" {
        return match super::pdf::extract_pdf_text(client).await {
            Some(Ok(text)) => Ok(text),
            Some(Err(e)) => Ok(format!("(pdf extraction failed: {e})")),
            None => Ok("(pdf: could not determine URL for extraction)".to_string()),
        };
    }

    let sel = serde_json::to_string(selector).unwrap();
    if include_links {
        let expr = format!(
            r#"return (() => {{
                const el = document.querySelector({sel});
                if (!el) return '';
                const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT, null, false);
                const parts = [];
                let node;
                while (node = walker.nextNode()) {{
                    const text = node.textContent.trim();
                    if (text) parts.push(text);
                }}
                const links = Array.from(el.querySelectorAll('a[href]'))
                    .map(a => `[${{a.innerText.trim()}}](${{a.href}})`)
                    .filter(s => s.length > 4);
                return parts.join(' ') + (links.length ? '\n\nLinks:\n' + links.join('\n') : '');
            }})()"#
        );
        let v = eval_body(client, &expr).await?;
        return Ok(v.as_str().unwrap_or("").to_string());
    }

    let expr = format!("return document.querySelector({sel})?.innerText?.trim() || ''");
    let v = eval_body(client, &expr).await?;
    Ok(v.as_str().unwrap_or("").to_string())
}

const VALID_SCREENSHOT_FORMATS: &[&str] = &["png", "jpeg"];

/// Capture the viewport as base64. `format` is "png" or "jpeg".
pub async fn screenshot_base64(
    client: &CdpClient,
    format: &str,
    quality: i64,
) -> Result<String, CdpError> {
    if !VALID_SCREENSHOT_FORMATS.contains(&format) {
        return Err(CdpError::Protocol {
            method: "Page.captureScreenshot".into(),
            code: -1,
            message: format!("Unsupported screenshot format '{format}'. Use png or jpeg."),
        });
    }
    let mut params = json!({ "format": format });
    if format == "jpeg" {
        params["quality"] = json!(quality.clamp(0, 100));
    }
    let result = client.send("Page.captureScreenshot", params).await?;
    Ok(result
        .get("data")
        .and_then(|d| d.as_str())
        .unwrap_or("")
        .to_string())
}
