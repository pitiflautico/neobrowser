//! Evaluating JavaScript in the page, and the frame nudge that makes it reliable.
//!
//! Every observation this crate makes eventually comes through `js`. It is the narrowest
//! and most load-bearing function in the codebase, which is why it is alone in a file:
//! a mistake here does not produce an error, it produces a plausible wrong answer.

use std::time::Duration;

use serde_json::{json, Value};

use crate::cdp::{CdpClient, CdpError};

/// Evaluate a **function body** in the page and return its value.
///
/// The code is wrapped as `(async function(){ … })()`, so it may declare variables, branch
/// and `await`. To produce a value it must contain a `return` **at the top level of the
/// body** — an indented one is fine, a `return` that only appears inside a nested callback
/// is not, because the outer function would then return nothing and the result would be
/// `undefined` rather than an error.
///
/// That constraint is asserted for every shipped snippet in [`crate::js`]'s tests, which is
/// where it belongs: the caller knows which shape its code is, so nothing here has to guess.
pub async fn eval_body(client: &CdpClient, body: &str) -> Result<Value, CdpError> {
    evaluate(client, &format!("(async function(){{{body}}})()")).await
}

/// Evaluate an **expression or a bare script** in the page and return its value.
///
/// Passed to `Runtime.evaluate` unchanged. Chrome evaluates it as a script, so a sequence of
/// statements is valid and the value is the completion value of the last one — which is why
/// a fire-and-forget snippet like restoring the page's console belongs here rather than in
/// [`eval_body`].
pub async fn eval_expr(client: &CdpClient, expr: &str) -> Result<Value, CdpError> {
    evaluate(client, expr).await
}

/// Evaluate code supplied by the caller, guessing which of the two shapes it is.
///
/// **The only place the guess is acceptable, and only because it is unavoidable here.** The
/// `js` tool hands a model's own JavaScript to the page, so this side genuinely cannot know
/// whether it is an expression or a body — there is nobody to ask.
///
/// The guess is `code.contains("return ")`, and its failure mode is worth stating because it
/// bit this project twice in other forms: code whose only `return` sits inside a nested
/// callback is wrapped as a body, that body returns nothing, and the caller receives
/// `undefined` instead of an error. Confining the heuristic to this one function is the fix —
/// every snippet this crate owns now says which shape it is, so none of them is exposed to it.
pub async fn eval_caller_supplied(client: &CdpClient, code: &str) -> Result<Value, CdpError> {
    if code.contains("return ") {
        eval_body(client, code).await
    } else {
        eval_expr(client, code).await
    }
}

async fn evaluate(client: &CdpClient, expression: &str) -> Result<Value, CdpError> {
    let result = client
        .send(
            "Runtime.evaluate",
            json!({
                "expression": expression,
                "returnByValue": true,
                "awaitPromise": true,
            }),
        )
        .await?;
    Ok(result
        .get("result")
        .and_then(|r| r.get("value"))
        .cloned()
        .unwrap_or(Value::Null))
}

/// Force the compositor to produce frames so deferred content materializes.
///
/// In `--headless=new` the compositor is idle until a frame is requested, so
/// `requestAnimationFrame`, `IntersectionObserver`, and virtualized lists never run
/// their "update the rendering" step. A screenshot is the one thing that reliably
/// forces that step (verified empirically). We capture a 1×1 JPEG (cheap to encode,
/// bytes discarded) a few times with short gaps: the first frame fires the observers
/// that kick off loading, the later frames paint the content they produced.
pub async fn nudge_frame(client: &CdpClient) {
    for i in 0..3 {
        let _ = client
            .send(
                "Page.captureScreenshot",
                json!({
                    "format": "jpeg",
                    "quality": 1,
                    "clip": { "x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0, "scale": 1.0 },
                    "captureBeyondViewport": false,
                    "optimizeForSpeed": true,
                }),
            )
            .await;
        if i < 2 {
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }
}

/// Whether the browser is still delivering synthetic input events to this page.
///
/// Chrome 151 has a state — reachable in ordinary use, and not by anything this crate does
/// wrong — in which `Input.dispatchMouseEvent` and `Input.dispatchKeyEvent` are accepted
/// without error and never delivered to the page. Every other domain keeps working:
/// `Runtime` evaluates, `DOM` measures, `Page.captureScreenshot` paints. A JS `.click()` on
/// the very button that will not respond to a real click works immediately.
///
/// It was found by driving a real site interactively, and it survives everything: reload,
/// `Page.navigate`, `Target.activateTarget`, `Page.bringToFront`,
/// `Input.setIgnoreInputEvents(false)`, clearing device-metrics overrides, and reconnecting
/// the websocket. The only recovery is a different tab, which works immediately.
///
/// So the tool cannot prevent it, and must not paper over it. This probe is how a caller can
/// tell "the click did not change anything" from "this tab stopped accepting clicks at all" —
/// two situations that look identical from the outside and call for completely different
/// responses. It installs a counter, dispatches one `mouseMoved`, and reports whether it
/// arrived. Three round trips, so it belongs on the failure path only.
///
/// A probe that cannot run reports `true`: refusing to act because a diagnostic failed would
/// be worse than the problem it diagnoses.
pub async fn input_is_alive(client: &CdpClient) -> bool {
    const MARKER: &str = "__nb_input_probe";
    let install = format!(
        "window.{MARKER} = 0; \
         document.addEventListener('mousemove', function(){{ window.{MARKER}++; }}, true); 1"
    );
    if eval_expr(client, &install).await.is_err() {
        return true;
    }
    // A corner far from anything hoverable: the probe must not open a menu or fire a tooltip.
    if client
        .send(
            "Input.dispatchMouseEvent",
            json!({ "type": "mouseMoved", "x": 1, "y": 1 }),
        )
        .await
        .is_err()
    {
        return true;
    }
    tokio::time::sleep(Duration::from_millis(120)).await;
    match eval_expr(client, &format!("window.{MARKER} || 0")).await {
        Ok(v) => v.as_i64().unwrap_or(1) > 0,
        Err(_) => true,
    }
}
