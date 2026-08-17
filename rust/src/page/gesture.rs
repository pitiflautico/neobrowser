//! The public pointer gestures, built on the primitives in [`super::pointer`].
//!
//! These are the verbs the tools expose: click this selector, click that stashed node,
//! hover, right-click, drag. Each one resolves its target freshly rather than trusting a
//! node id it was handed earlier, because a `backendNodeId` is invalidated by any re-render
//! between the observation and the action — and a stale id does not error, it silently
//! addresses a different element.

use std::time::Duration;

use serde_json::json;

use crate::cdp::{CdpClient, CdpError};

use super::diagnose::obscured_by;
use super::pointer::human_mouse_move;

use super::node::{backend_node_for_selector, box_center, scroll_into_view};
use super::pointer::{click_backend_node, ClickOutcome};

/// Click an element previously stashed on `window.<global>` by page JS, with a
/// real mouse click. Lets a JS selection step (which can evaluate layout and
/// computed styles) hand its pick to the isTrusted click path. The global is
/// cleared afterwards. Returns None if the global holds no element.
pub async fn click_stashed_node(
    client: &CdpClient,
    global: &str,
) -> Result<Option<ClickOutcome>, CdpError> {
    let res = client
        .send(
            "Runtime.evaluate",
            json!({ "expression": format!("window.{global}") }),
        )
        .await?;
    let object_id = res
        .get("result")
        .and_then(|r| r.get("objectId"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let Some(object_id) = object_id else {
        return Ok(None);
    };
    let described = client
        .send("DOM.describeNode", json!({ "objectId": object_id }))
        .await?;
    let backend_id = described
        .get("node")
        .and_then(|n| n.get("backendNodeId"))
        .and_then(|v| v.as_i64());
    // Don't leave our scratch global on the page.
    let _ = client
        .send(
            "Runtime.evaluate",
            json!({ "expression": format!("delete window.{global}") }),
        )
        .await;
    let Some(backend_id) = backend_id else {
        return Ok(None);
    };
    Ok(Some(click_backend_node(client, backend_id).await?))
}

/// Click the first element matching a CSS `selector` with a real mouse click.
pub async fn click_selector(client: &CdpClient, selector: &str) -> Result<ClickOutcome, CdpError> {
    match backend_node_for_selector(client, selector).await? {
        Some(id) => click_backend_node(client, id).await,
        None => Ok(ClickOutcome::NotFound),
    }
}

/// Hover over an element: move the real cursor there without pressing.
///
/// Needed for menus and tooltips that only appear on pointer-over, which a click
/// cannot reveal because the click dismisses them.
pub async fn hover(client: &CdpClient, backend_node_id: i64) -> Result<String, CdpError> {
    scroll_into_view(client, backend_node_id).await;
    let Some((cx, cy)) = box_center(client, backend_node_id).await? else {
        return Err(CdpError::Closed(
            "hover: element has no box model (it may be hidden)".into(),
        ));
    };
    human_mouse_move(client, cx, cy).await?;
    client
        .send(
            "Input.dispatchMouseEvent",
            json!({ "type": "mouseMoved", "x": cx, "y": cy }),
        )
        .await?;
    Ok(format!("hovered at ({cx:.0}, {cy:.0})"))
}

/// Double or right click, reusing the same scroll + hit-test discipline as `click`.
pub async fn click_variant(
    client: &CdpClient,
    backend_node_id: i64,
    button: &str,
    click_count: i64,
) -> Result<String, CdpError> {
    scroll_into_view(client, backend_node_id).await;
    let Some((cx, cy)) = box_center(client, backend_node_id).await? else {
        return Err(CdpError::Closed(
            "click: element has no box model (it may be hidden)".into(),
        ));
    };
    if let Some(by) = obscured_by(client, backend_node_id, cx, cy).await? {
        return Err(CdpError::Closed(format!(
            "not clicked: target is covered by {by}"
        )));
    }
    human_mouse_move(client, cx, cy).await?;
    for ty in ["mousePressed", "mouseReleased"] {
        client
            .send(
                "Input.dispatchMouseEvent",
                json!({
                    "type": ty,
                    "x": cx,
                    "y": cy,
                    "button": button,
                    "clickCount": click_count,
                }),
            )
            .await?;
    }
    Ok(format!("{button} click x{click_count} dispatched"))
}

/// Drag from one element to another with real mouse events.
pub async fn drag_and_drop(client: &CdpClient, from: i64, to: i64) -> Result<String, CdpError> {
    scroll_into_view(client, from).await;
    let Some((fx, fy)) = box_center(client, from).await? else {
        return Err(CdpError::Closed("drag: source has no box model".into()));
    };
    let Some((tx, ty)) = box_center(client, to).await? else {
        return Err(CdpError::Closed("drag: target has no box model".into()));
    };
    human_mouse_move(client, fx, fy).await?;
    client
        .send(
            "Input.dispatchMouseEvent",
            json!({ "type": "mousePressed", "x": fx, "y": fy, "button": "left", "clickCount": 1 }),
        )
        .await?;
    // Intermediate moves while held: HTML5 drag-and-drop and every JS drag library start
    // tracking on movement, so a press-then-release at the destination does nothing at all.
    for step in 1..=10 {
        let t = step as f64 / 10.0;
        client
            .send(
                "Input.dispatchMouseEvent",
                json!({
                    "type": "mouseMoved",
                    "x": fx + (tx - fx) * t,
                    "y": fy + (ty - fy) * t,
                    "button": "left",
                }),
            )
            .await?;
        tokio::time::sleep(Duration::from_millis(16)).await;
    }
    client
        .send(
            "Input.dispatchMouseEvent",
            json!({ "type": "mouseReleased", "x": tx, "y": ty, "button": "left", "clickCount": 1 }),
        )
        .await?;
    Ok(format!("dragged ({fx:.0},{fy:.0}) -> ({tx:.0},{ty:.0})"))
}
