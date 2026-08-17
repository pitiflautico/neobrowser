//! Resolving a target to something a pointer can aim at, and diagnosing why it cannot.
//!
//! Between "the model asked for the Submit button" and "dispatch a mouse event at (x, y)"
//! sits a surprising amount of work: find the node, scroll it into view, get its box, and
//! check that nothing is sitting on top of it. That last part is the reason this module is
//! worth its own file. When a click fails, the useful answer is never "it failed" — it is
//! "a cookie banner with z-index 9999 is covering it", and producing that answer requires
//! asking the page what is actually at those coordinates.

use serde_json::json;

use crate::cdp::{CdpClient, CdpError};

pub(super) async fn scroll_into_view(client: &CdpClient, backend_node_id: i64) {
    let _ = client
        .send(
            "DOM.scrollIntoViewIfNeeded",
            json!({ "backendNodeId": backend_node_id }),
        )
        .await;
}

/// Hit-test (cx, cy): return a description of the element on top when it is
/// NOT the target nor one of its descendants, else None.
///
/// Descendants must pass: the centre of a `<button>` usually lands on an inner
/// `<span>`, which is a perfectly good click. What has to be rejected is a node
/// from a different branch — that is the overlay case.
pub(super) async fn obscured_by(
    client: &CdpClient,
    backend_node_id: i64,
    cx: f64,
    cy: f64,
) -> Result<Option<String>, CdpError> {
    let hit = client
        .send(
            "DOM.getNodeForLocation",
            json!({ "x": cx as i64, "y": cy as i64, "includeUserAgentShadowDOM": false }),
        )
        .await;
    // Hit-testing is best-effort: if the command is unavailable or errors, do
    // not block a click that would otherwise have worked.
    let Ok(hit) = hit else { return Ok(None) };
    let Some(hit_id) = hit.get("backendNodeId").and_then(|v| v.as_i64()) else {
        return Ok(None);
    };
    if hit_id == backend_node_id {
        return Ok(None);
    }
    let related = js_nodes_related(client, backend_node_id, hit_id).await?;
    if related {
        return Ok(None);
    }
    Ok(Some(describe_node(client, hit_id).await))
}

/// True when either node contains the other (so the hit is inside the target,
/// or the target is inside the hit — a label wrapping its own control).
async fn js_nodes_related(
    client: &CdpClient,
    a_backend: i64,
    b_backend: i64,
) -> Result<bool, CdpError> {
    let Some(a_obj) = resolve_object_id(client, a_backend).await? else {
        return Ok(false);
    };
    let Some(b_obj) = resolve_object_id(client, b_backend).await? else {
        return Ok(false);
    };
    let res = client
        .send(
            "Runtime.callFunctionOn",
            json!({
                "objectId": a_obj,
                "functionDeclaration":
                    "function(other){ return this.contains(other) || other.contains(this); }",
                "arguments": [{ "objectId": b_obj }],
                "returnByValue": true,
            }),
        )
        .await?;
    Ok(res
        .get("result")
        .and_then(|r| r.get("value"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false))
}

/// A short `tag.class` label for a node, for the obscured-by message.
async fn describe_node(client: &CdpClient, backend_node_id: i64) -> String {
    let Ok(Some(obj)) = resolve_object_id(client, backend_node_id).await else {
        return "unknown element".into();
    };
    let res = client
        .send(
            "Runtime.callFunctionOn",
            json!({
                "objectId": obj,
                "functionDeclaration": "function(){ \
                    var c = (this.className && this.className.baseVal !== undefined) \
                        ? this.className.baseVal : (this.className || ''); \
                    return (this.tagName || '?').toLowerCase() \
                        + (c ? '.' + String(c).trim().split(/\\s+/).slice(0,2).join('.') : ''); }",
                "returnByValue": true,
            }),
        )
        .await;
    res.ok()
        .and_then(|r| {
            r.get("result")
                .and_then(|x| x.get("value"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "unknown element".into())
}

/// Resolve a backendNodeId to a Runtime objectId.
async fn resolve_object_id(
    client: &CdpClient,
    backend_node_id: i64,
) -> Result<Option<String>, CdpError> {
    let Ok(node) = client
        .send(
            "DOM.resolveNode",
            json!({ "backendNodeId": backend_node_id }),
        )
        .await
    else {
        return Ok(None);
    };
    Ok(node
        .get("object")
        .and_then(|o| o.get("objectId"))
        .and_then(|v| v.as_str())
        .map(str::to_string))
}

/// Move the cursor to (tx, ty) over several eased, jittered steps with human-cadence
/// pauses — approximating a real hand rather than an instantaneous jump.
pub(super) async fn box_center(
    client: &CdpClient,
    backend_node_id: i64,
) -> Result<Option<(f64, f64)>, CdpError> {
    let res = client
        .send(
            "DOM.getBoxModel",
            json!({ "backendNodeId": backend_node_id }),
        )
        .await;
    let Ok(res) = res else { return Ok(None) };
    let quad = res
        .get("model")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_array());
    let Some(quad) = quad else { return Ok(None) };
    if quad.len() < 8 {
        return Ok(None);
    }
    let q: Vec<f64> = quad.iter().map(|v| v.as_f64().unwrap_or(0.0)).collect();
    let cx = (q[0] + q[2] + q[4] + q[6]) / 4.0;
    let cy = (q[1] + q[3] + q[5] + q[7]) / 4.0;
    Ok(Some((cx, cy)))
}

/// JS `.click()` fallback via DOM.resolveNode + Runtime.callFunctionOn.
pub async fn backend_node_for_css(
    client: &CdpClient,
    selector: &str,
) -> Result<Option<i64>, CdpError> {
    backend_node_for_selector(client, selector).await
}

pub(super) async fn backend_node_for_selector(
    client: &CdpClient,
    selector: &str,
) -> Result<Option<i64>, CdpError> {
    let doc = client
        .send("DOM.getDocument", json!({ "depth": 0 }))
        .await?;
    let Some(root) = doc
        .get("root")
        .and_then(|r| r.get("nodeId"))
        .and_then(|v| v.as_i64())
    else {
        return Ok(None);
    };
    let found = client
        .send(
            "DOM.querySelector",
            json!({ "nodeId": root, "selector": selector }),
        )
        .await?;
    let node_id = found.get("nodeId").and_then(|v| v.as_i64()).unwrap_or(0);
    if node_id == 0 {
        return Ok(None);
    }
    let desc = client
        .send("DOM.describeNode", json!({ "nodeId": node_id }))
        .await?;
    Ok(desc
        .get("node")
        .and_then(|n| n.get("backendNodeId"))
        .and_then(|v| v.as_i64()))
}
