//! CDP mouse interaction tools — click, hover, drag, upload.
//!
//! All methods resolve element positions via DOM.getBoxModel and dispatch
//! Input.dispatchMouseEvent commands through the page session.

use crate::session::ChromeSession;
use crate::{ChromeError, Result};
use serde_json::json;

impl ChromeSession {
    /// Resolve the center coordinates of an element matched by CSS selector.
    ///
    /// Pipeline: DOM.getDocument -> DOM.querySelector -> DOM.getBoxModel -> center of content quad.
    async fn resolve_element_position(&self, selector: &str) -> Result<(f64, f64)> {
        // 1. Get document root node.
        let doc = self
            .cdp
            .send_to(&self.page_session_id, "DOM.getDocument", None)
            .await?;

        let root_node_id = doc
            .get("root")
            .and_then(|r| r.get("nodeId"))
            .and_then(|v| v.as_i64())
            .ok_or_else(|| ChromeError::CommandFailed {
                method: "DOM.getDocument".into(),
                error: "missing root nodeId".into(),
            })?;

        // 2. Query for the element.
        let query_result = self
            .cdp
            .send_to(
                &self.page_session_id,
                "DOM.querySelector",
                Some(json!({
                    "nodeId": root_node_id,
                    "selector": selector,
                })),
            )
            .await?;

        let node_id = query_result
            .get("nodeId")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| ChromeError::CommandFailed {
                method: "DOM.querySelector".into(),
                error: format!("element not found: {selector}"),
            })?;

        if node_id == 0 {
            return Err(ChromeError::CommandFailed {
                method: "DOM.querySelector".into(),
                error: format!("element not found: {selector}"),
            });
        }

        // 3. Get box model and calculate center from content quad.
        let box_result = self
            .cdp
            .send_to(
                &self.page_session_id,
                "DOM.getBoxModel",
                Some(json!({ "nodeId": node_id })),
            )
            .await?;

        let content = box_result
            .get("model")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_array())
            .ok_or_else(|| ChromeError::CommandFailed {
                method: "DOM.getBoxModel".into(),
                error: "missing content quad".into(),
            })?;

        center_from_quad(content)
    }

    /// Resolve the DOM nodeId for an element matched by CSS selector.
    async fn resolve_node_id(&self, selector: &str) -> Result<i64> {
        let doc = self
            .cdp
            .send_to(&self.page_session_id, "DOM.getDocument", None)
            .await?;

        let root_node_id = doc
            .get("root")
            .and_then(|r| r.get("nodeId"))
            .and_then(|v| v.as_i64())
            .ok_or_else(|| ChromeError::CommandFailed {
                method: "DOM.getDocument".into(),
                error: "missing root nodeId".into(),
            })?;

        let query_result = self
            .cdp
            .send_to(
                &self.page_session_id,
                "DOM.querySelector",
                Some(json!({
                    "nodeId": root_node_id,
                    "selector": selector,
                })),
            )
            .await?;

        let node_id = query_result
            .get("nodeId")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| ChromeError::CommandFailed {
                method: "DOM.querySelector".into(),
                error: format!("element not found: {selector}"),
            })?;

        if node_id == 0 {
            return Err(ChromeError::CommandFailed {
                method: "DOM.querySelector".into(),
                error: format!("element not found: {selector}"),
            });
        }

        Ok(node_id)
    }

    /// Dispatch a single mouse event to the page.
    async fn dispatch_mouse(
        &self,
        event_type: &str,
        x: f64,
        y: f64,
        button: &str,
        click_count: i32,
    ) -> Result<()> {
        self.cdp
            .send_to(
                &self.page_session_id,
                "Input.dispatchMouseEvent",
                Some(json!({
                    "type": event_type,
                    "x": x,
                    "y": y,
                    "button": button,
                    "clickCount": click_count,
                })),
            )
            .await?;
        Ok(())
    }

    /// Click an element identified by CSS selector.
    ///
    /// If `dbl_click` is true, dispatches with clickCount=2 (double-click).
    pub async fn click(&self, selector: &str, dbl_click: bool) -> Result<()> {
        let (x, y) = self.resolve_element_position(selector).await?;
        let click_count = if dbl_click { 2 } else { 1 };

        // Move to the element first.
        self.dispatch_mouse("mouseMoved", x, y, "none", 0).await?;

        // Press + release.
        self.dispatch_mouse("mousePressed", x, y, "left", click_count)
            .await?;
        self.dispatch_mouse("mouseReleased", x, y, "left", click_count)
            .await?;

        Ok(())
    }

    /// Hover over an element identified by CSS selector.
    pub async fn hover(&self, selector: &str) -> Result<()> {
        let (x, y) = self.resolve_element_position(selector).await?;
        self.dispatch_mouse("mouseMoved", x, y, "none", 0).await?;
        Ok(())
    }

    /// Drag from one element to another, both identified by CSS selectors.
    pub async fn drag(&self, from_selector: &str, to_selector: &str) -> Result<()> {
        let (fx, fy) = self.resolve_element_position(from_selector).await?;
        let (tx, ty) = self.resolve_element_position(to_selector).await?;

        // Move to source, press, move to destination, release.
        self.dispatch_mouse("mouseMoved", fx, fy, "none", 0)
            .await?;
        self.dispatch_mouse("mousePressed", fx, fy, "left", 1)
            .await?;
        self.dispatch_mouse("mouseMoved", tx, ty, "left", 0)
            .await?;
        self.dispatch_mouse("mouseReleased", tx, ty, "left", 1)
            .await?;

        Ok(())
    }

    /// Upload a file through an `<input type="file">` element.
    ///
    /// Uses DOM.setFileInputFiles to set the file path on the input node.
    pub async fn upload_file(&self, selector: &str, file_path: &str) -> Result<()> {
        let node_id = self.resolve_node_id(selector).await?;

        self.cdp
            .send_to(
                &self.page_session_id,
                "DOM.setFileInputFiles",
                Some(json!({
                    "nodeId": node_id,
                    "files": [file_path],
                })),
            )
            .await?;

        Ok(())
    }
}

/// Calculate the center point of a CDP content quad.
///
/// A quad is 8 floats: [x1,y1, x2,y2, x3,y3, x4,y4] representing four corners.
/// Center = average of all four corner coordinates.
pub fn center_from_quad(quad: &[serde_json::Value]) -> Result<(f64, f64)> {
    if quad.len() != 8 {
        return Err(ChromeError::CommandFailed {
            method: "DOM.getBoxModel".into(),
            error: format!("expected 8 quad values, got {}", quad.len()),
        });
    }

    let vals: std::result::Result<Vec<f64>, _> = quad
        .iter()
        .map(|v| {
            v.as_f64().ok_or_else(|| ChromeError::CommandFailed {
                method: "DOM.getBoxModel".into(),
                error: "non-numeric quad value".into(),
            })
        })
        .collect();
    let vals = vals?;

    let cx = (vals[0] + vals[2] + vals[4] + vals[6]) / 4.0;
    let cy = (vals[1] + vals[3] + vals[5] + vals[7]) / 4.0;

    Ok((cx, cy))
}

/// Build the JSON params for Input.dispatchMouseEvent.
///
/// Exposed for testing serialization without a live session.
pub fn mouse_event_params(
    event_type: &str,
    x: f64,
    y: f64,
    button: &str,
    click_count: i32,
) -> serde_json::Value {
    json!({
        "type": event_type,
        "x": x,
        "y": y,
        "button": button,
        "clickCount": click_count,
    })
}
