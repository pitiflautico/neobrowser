//! Evaluating JavaScript in the page, and the frame nudge that makes it reliable.
//!
//! Every observation this crate makes eventually comes through `js`. It is the narrowest
//! and most load-bearing function in the codebase, which is why it is alone in a file:
//! a mistake here does not produce an error, it produces a plausible wrong answer.

use std::time::Duration;

use serde_json::{json, Value};

use crate::cdp::{CdpClient, CdpError};

/// Evaluate `expr` in the page and return its value.
///
/// Two shapes are accepted, distinguished by whether the code contains `return `:
///
/// - **A statement body.** Wrapped as `(async function(){ … })()`, so it may declare
///   variables, branch, and `await`. This is what most snippets are.
/// - **An expression or a bare script.** Passed to `Runtime.evaluate` unchanged. Chrome
///   evaluates it as a script, so a sequence of statements works and the value is the
///   completion value of the last one.
///
/// # The hazard in that heuristic
///
/// The choice is made by `expr.contains("return ")`, which cannot tell a `return` at the
/// top level from one inside a nested callback. Code whose only `return` sits in, say, an
/// `Array.map` callback gets wrapped as a function body — and that body never returns, so
/// the result is `undefined`. Not an error: `undefined`, silently, which a caller reads as
/// a page that evaluated to nothing.
///
/// That is the same failure mode as the automatic-semicolon-insertion bug documented in
/// [`crate::js`], and it reached production twice. Every shipped snippet is asserted safe
/// against it in the `js` module's tests. **A new snippet that needs a value must have a
/// `return` at the top level of its body** — an indented one is fine, a nested-only one is
/// not.
///
/// The heuristic is kept rather than replaced because a brace-aware scanner would have to
/// track string and comment context to be correct, and being subtly wrong here is worse
/// than being crudely right with the constraint written down and tested.
pub async fn js(client: &CdpClient, expr: &str) -> Result<Value, CdpError> {
    let wrapped;
    let expression = if expr.contains("return ") {
        wrapped = format!("(async function(){{{expr}}})()");
        wrapped.as_str()
    } else {
        expr
    };
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
