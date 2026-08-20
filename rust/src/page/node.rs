//! Resolving a target to something a pointer can aim at.
//!
//! Between "the model asked for the Submit button" and dispatching a mouse event at (x, y)
//! sits a scroll, a selector lookup, a node resolution and a box measurement. Why the
//! resulting point might still be unclickable is [`super::diagnose`]'s job.

use crate::cdp::{CdpClient, CdpError};
use serde_json::json;

/// Scroll a node into the viewport before measuring or clicking it.
///
/// Failures are ignored on purpose: a node inside an unscrollable container is still
/// clickable where it sits, and refusing would break those cases for no gain.
pub(super) async fn scroll_into_view(client: &CdpClient, backend_node_id: i64) {
    let _ = client
        .send(
            "DOM.scrollIntoViewIfNeeded",
            json!({ "backendNodeId": backend_node_id }),
        )
        .await;
}

/// Resolve a backendNodeId to a Runtime objectId.
pub(super) async fn resolve_object_id(
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

/// Center of an element's content box, or None if it has no layout.
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

/// Resolve a CSS selector to a backendNodeId, or None if it matches nothing.
///
/// Public alias for the CSS -> `backendNodeId` resolver, for tools that address elements by
/// selector but need a node id (hover, drag, click variants).
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
