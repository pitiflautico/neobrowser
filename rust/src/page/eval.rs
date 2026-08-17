//! Evaluating JavaScript in the page, and the frame nudge that makes it reliable.
//!
//! Every observation this crate makes eventually comes through `js`. It is the narrowest
//! and most load-bearing function in the codebase, which is why it is alone in a file:
//! a mistake here does not produce an error, it produces a plausible wrong answer.

use std::time::Duration;

use serde_json::{json, Value};

use crate::cdp::{CdpClient, CdpError};

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
