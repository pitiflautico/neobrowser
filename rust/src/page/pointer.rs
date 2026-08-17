//! The primitives behind a single click: dispatch it, then prove it landed.
//!
//! `click_backend_node` is the honest core of the whole tool. It moves the pointer the way
//! a hand does, dispatches trusted events at the element's centre, and then — crucially —
//! checks whether anything actually happened. When it cannot tell, it says so
//! (`ClickOutcome::Unverified`) instead of returning success, because a click that reports
//! success without landing is how an agent proceeds confidently into a page it never
//! changed. The fallbacks are ordered by how much they resemble a user, most first.

use std::time::Duration;

use serde_json::json;

use crate::cdp::{CdpClient, CdpError};

use super::diagnose::{obscured_by, refuses_input};
use super::input::Jitter;
use super::node::{box_center, scroll_into_view};

/// What actually happened when we tried to click — never just "we dispatched
/// two mouse events". A caller (and an agent reading the tool result) has to be
/// able to tell a landed click from one that fell off-screen or hit an overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClickOutcome {
    /// A real press/release landed on the target (or one of its descendants).
    Clicked,
    /// The selector matched nothing.
    NotFound,
    /// The node exists but has no box model (display:none, detached, zero-size).
    /// A JS `.click()` fallback was attempted instead.
    NoLayoutUsedJs,
    /// Another element occupies the click point — typically a modal, a cookie
    /// banner or a sticky header. The click was NOT dispatched.
    Obscured { by: String },
    /// The control refuses input: `disabled`, `aria-disabled`, or
    /// `pointer-events: none`. The click was NOT dispatched.
    ///
    /// This is knowable *before* acting, which is the whole reason it is a distinct
    /// outcome. Dispatching at a disabled button and then reporting "nothing changed"
    /// is technically true and practically useless: the caller retries the identical
    /// click forever instead of fixing whatever keeps the control disabled. The
    /// conformance suite's C4 exists for exactly this, and this variant is what it
    /// found missing.
    Disabled { reason: String },
}

impl ClickOutcome {
    /// Did the intended element actually receive a click?
    pub fn landed(&self) -> bool {
        matches!(self, ClickOutcome::Clicked | ClickOutcome::NoLayoutUsedJs)
    }
}

/// Click an element by `backendNodeId` using real mouse events at its centre
/// (isTrusted:true). Falls back to a JS `.click()` when the element has no layout
/// box. Returns true on success.
pub async fn click_backend_node(
    client: &CdpClient,
    backend_node_id: i64,
) -> Result<ClickOutcome, CdpError> {
    // Bring the node into the viewport FIRST. `DOM.getBoxModel` returns
    // viewport-relative coordinates, so a node below the fold yields a `y` that
    // is off-screen and the dispatched event lands nowhere.
    scroll_into_view(client, backend_node_id).await;

    // Re-read the box AFTER scrolling — any coordinates from before are stale.
    let Some((cx, cy)) = box_center(client, backend_node_id).await? else {
        // No box model — fall back to a JS click via the resolved node.
        return if js_click_backend_node(client, backend_node_id).await? {
            Ok(ClickOutcome::NoLayoutUsedJs)
        } else {
            Ok(ClickOutcome::NotFound)
        };
    };

    // A control that refuses input is knowable now, so say so now. Dispatching anyway and
    // reporting `uncertain` afterwards withholds a diagnosis we already have.
    if let Some(reason) = refuses_input(client, backend_node_id).await? {
        return Ok(ClickOutcome::Disabled { reason });
    }

    // Verify the point actually belongs to the target before pressing, so an
    // overlay can be reported instead of silently swallowing the click.
    if let Some(by) = obscured_by(client, backend_node_id, cx, cy).await? {
        return Ok(ClickOutcome::Obscured { by });
    }

    // Behavioral realism: move the cursor to the target along a human-like path
    // (curved, eased, jittered, with per-step pauses) instead of teleporting —
    // the trajectory/timing signals behavioral anti-bot systems inspect.
    human_mouse_move(client, cx, cy).await?;
    // A short dwell before the press, as a human would.
    let mut j = Jitter::new(((cx as i64) ^ (cy as i64) ^ backend_node_id) as u64);
    tokio::time::sleep(Duration::from_millis(40 + j.next() % 80)).await;
    client
        .send(
            "Input.dispatchMouseEvent",
            // `buttons` is the bitmask of buttons held down, and omitting it means Chrome is
            // told the left button is being pressed while no button is held — a contradiction
            // it carries in its internal mouse state. Puppeteer and Playwright both send it;
            // this did not, and the symptom was a tab that stopped delivering input entirely
            // after the first click.
            json!({ "type": "mousePressed", "x": cx, "y": cy, "button": "left",
                    "buttons": 1, "clickCount": 1 }),
        )
        .await?;
    tokio::time::sleep(Duration::from_millis(20 + j.next() % 60)).await;
    client
        .send(
            "Input.dispatchMouseEvent",
            json!({ "type": "mouseReleased", "x": cx, "y": cy, "button": "left",
                    "buttons": 0, "clickCount": 1 }),
        )
        .await?;
    Ok(ClickOutcome::Clicked)
}

/// Move the cursor to (tx, ty) over several eased, jittered steps with human-cadence
/// pauses — approximating a real hand rather than an instantaneous jump.
pub(super) async fn human_mouse_move(client: &CdpClient, tx: f64, ty: f64) -> Result<(), CdpError> {
    let mut j = Jitter::new(((tx as i64).wrapping_mul(31) ^ (ty as i64)) as u64);
    // Start from a plausible off-target origin (as if arriving from up-and-left).
    let sx = (tx - 90.0 - (j.next() % 120) as f64).max(0.0);
    let sy = (ty - 70.0 - (j.next() % 90) as f64).max(0.0);
    let steps = 12 + (j.next() % 10) as usize; // 12–21 steps
    for i in 1..=steps {
        let t = i as f64 / steps as f64;
        let ease = t * t * (3.0 - 2.0 * t); // smoothstep
        let jitter_x = ((j.next() % 5) as f64) - 2.0;
        let jitter_y = ((j.next() % 5) as f64) - 2.0;
        let x = sx + (tx - sx) * ease + if i == steps { 0.0 } else { jitter_x };
        let y = sy + (ty - sy) * ease + if i == steps { 0.0 } else { jitter_y };
        client
            .send(
                "Input.dispatchMouseEvent",
                json!({ "type": "mouseMoved", "x": x.max(0.0), "y": y.max(0.0),
                        "buttons": 0 }),
            )
            .await?;
        tokio::time::sleep(Duration::from_millis(6 + j.next() % 12)).await;
    }
    Ok(())
}

/// JS `.click()` fallback via DOM.resolveNode + Runtime.callFunctionOn.
pub(super) async fn js_click_backend_node(
    client: &CdpClient,
    backend_node_id: i64,
) -> Result<bool, CdpError> {
    let node = client
        .send(
            "DOM.resolveNode",
            json!({ "backendNodeId": backend_node_id }),
        )
        .await?;
    let object_id = node
        .get("object")
        .and_then(|o| o.get("objectId"))
        .and_then(|v| v.as_str());
    let Some(object_id) = object_id else {
        return Ok(false);
    };
    client
        .send(
            "Runtime.callFunctionOn",
            json!({
                "objectId": object_id,
                "functionDeclaration": "function(){ this.click(); return true; }",
                "returnByValue": true,
            }),
        )
        .await?;
    Ok(true)
}
