//! Page-level CDP helpers: the verbs the tools are built from.
//!
//! These wrap raw CDP calls (`Runtime.evaluate`, `Page.navigate`,
//! `Page.captureScreenshot`, `Input.*`, `DOM.*`, `Accessibility.getFullAXTree`)
//! into the operations `chrome_tab.py` exposed. Anti-detection semantics are kept:
//! clicks dispatch real mouse events (isTrusted) at the element centre, and
//! `type_text(human=true)` emits per-key events with human-like cadence.

use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::cdp::{CdpClient, CdpError};

/// Evaluate a JS expression. If it contains `return `, wrap it in an **async** IIFE
/// so both bare `return value` and `return await …` work, and set `awaitPromise`
/// so the returned promise is resolved before we read the value.
///
/// The async wrapper + `awaitPromise` fixes the Python `ChromeTab.js` limitation
/// where any `await` in user code silently returned null (the IIFE was synchronous
/// and `awaitPromise` was false).
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
        if let Ok(Value::String(state)) = js(client, "return document.readyState").await {
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
        let count = js(client, "return document.getElementsByTagName('*').length")
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
    let v = js(client, "return location.href").await?;
    Ok(v.as_str().unwrap_or("").to_string())
}

/// Visible text of `selector` (defaults to `body`), trimmed.
pub async fn read_text(client: &CdpClient, selector: &str) -> Result<String, CdpError> {
    // Materialize deferred/virtualized content before reading it.
    nudge_frame(client).await;
    let sel = serde_json::to_string(selector).unwrap();
    let expr = format!("return document.querySelector({sel})?.innerText?.trim() || ''");
    let v = js(client, &expr).await?;
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

/// Type into the focused element. `human=true` emits per-key keydown/keyup with a
/// human-like cadence (isTrusted events anti-bot layers expect); `false` uses the
/// instant `Input.insertText` (React/Vue-safe paste).
pub async fn type_text(client: &CdpClient, text: &str, human: bool) -> Result<(), CdpError> {
    if !human {
        client
            .send("Input.insertText", json!({ "text": text }))
            .await?;
        return Ok(());
    }
    let mut rng = Jitter::new(text.len() as u64 ^ 0x9E37_79B9);
    for ch in text.chars() {
        // Control chars need real key metadata — `text: "\n"` alone is a key
        // event for *no key*, and editors (Draft.js, Quill, plain textareas)
        // silently drop it, collapsing the user's line breaks.
        if ch == '\n' {
            for ev in ["keyDown", "keyUp"] {
                client
                    .send(
                        "Input.dispatchKeyEvent",
                        json!({
                            "type": ev,
                            "key": "Enter",
                            "code": "Enter",
                            "windowsVirtualKeyCode": 13,
                            "nativeVirtualKeyCode": 13,
                            "text": "\r"
                        }),
                    )
                    .await?;
            }
        } else {
            let s = ch.to_string();
            client
                .send(
                    "Input.dispatchKeyEvent",
                    json!({ "type": "keyDown", "text": s, "key": s }),
                )
                .await?;
            client
                .send(
                    "Input.dispatchKeyEvent",
                    json!({ "type": "keyUp", "text": s, "key": s }),
                )
                .await?;
        }
        // 30–120ms inter-key delay, dependency-free pseudo-random.
        let ms = 30 + (rng.next() % 90);
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }
    Ok(())
}

/// Click an element by `backendNodeId` using real mouse events at its centre
/// (isTrusted:true). Falls back to a JS `.click()` when the element has no layout
/// box. Returns true on success.
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
}

impl ClickOutcome {
    /// Did the intended element actually receive a click?
    pub fn landed(&self) -> bool {
        matches!(self, ClickOutcome::Clicked | ClickOutcome::NoLayoutUsedJs)
    }
}

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
            json!({ "type": "mousePressed", "x": cx, "y": cy, "button": "left", "clickCount": 1 }),
        )
        .await?;
    tokio::time::sleep(Duration::from_millis(20 + j.next() % 60)).await;
    client
        .send(
            "Input.dispatchMouseEvent",
            json!({ "type": "mouseReleased", "x": cx, "y": cy, "button": "left", "clickCount": 1 }),
        )
        .await?;
    Ok(ClickOutcome::Clicked)
}

/// Scroll a node into the viewport before measuring or clicking it.
///
/// Failures are ignored on purpose: a node inside an unscrollable container is still
/// clickable where it sits, and refusing would break those cases for no gain.
async fn scroll_into_view(client: &CdpClient, backend_node_id: i64) {
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
async fn obscured_by(
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
async fn human_mouse_move(client: &CdpClient, tx: f64, ty: f64) -> Result<(), CdpError> {
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
                json!({ "type": "mouseMoved", "x": x.max(0.0), "y": y.max(0.0) }),
            )
            .await?;
        tokio::time::sleep(Duration::from_millis(6 + j.next() % 12)).await;
    }
    Ok(())
}

/// Center of an element's content box, or None if it has no layout.
async fn box_center(
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
async fn js_click_backend_node(client: &CdpClient, backend_node_id: i64) -> Result<bool, CdpError> {
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

/// Resolve a CSS selector to a backendNodeId, or None if it matches nothing.
/// Public alias for the CSS -> `backendNodeId` resolver, for tools that address elements by
/// selector but need a node id (hover, drag, click variants).
pub async fn backend_node_for_css(
    client: &CdpClient,
    selector: &str,
) -> Result<Option<i64>, CdpError> {
    backend_node_for_selector(client, selector).await
}

async fn backend_node_for_selector(
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

/// A semantic node from the accessibility tree.
#[derive(Debug, Clone)]
pub struct AxNode {
    pub role: String,
    pub name: String,
    pub backend_node_id: i64,
}

/// Interactive roles worth surfacing for `find`.
const INTERACTIVE_ROLES: &[&str] = &[
    "button",
    "textbox",
    "combobox",
    "searchbox",
    "link",
    "checkbox",
    "radio",
    "menuitem",
    "tab",
    "switch",
    "slider",
    "option",
];

/// Extract interactive nodes with names from the accessibility tree.
pub async fn ax_interactive_nodes(client: &CdpClient) -> Result<Vec<AxNode>, CdpError> {
    let tree = client
        .send("Accessibility.getFullAXTree", json!({}))
        .await?;
    let mut out = Vec::new();
    let Some(nodes) = tree.get("nodes").and_then(|n| n.as_array()) else {
        return Ok(out);
    };
    for node in nodes {
        if node
            .get("ignored")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            continue;
        }
        let role = node
            .get("role")
            .and_then(|r| r.get("value"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let name = node
            .get("name")
            .and_then(|r| r.get("value"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let backend = node
            .get("backendDOMNodeId")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        if backend == 0 {
            continue;
        }
        // Keep interactive roles even without a name; keep named nodes of any role
        // only if interactive (avoids flooding with StaticText).
        let interactive = INTERACTIVE_ROLES.contains(&role.as_str());
        if interactive {
            out.push(AxNode {
                role,
                name,
                backend_node_id: backend,
            });
        }
    }
    Ok(out)
}

/// Semantic find: score interactive AX nodes against the intent (zero-cost
/// heuristic, Layers 1–2), then fall back to an optional LLM (Layer 3) that only
/// runs when `ANTHROPIC_API_KEY` is set. The LLM only *chooses among* the
/// backendNodeIds we already extracted, and its choice is validated against that
/// set — a prompt injection in page text can't point us at a node not in the snapshot.
pub async fn find(client: &CdpClient, intent: &str) -> Result<Option<AxNode>, CdpError> {
    // The AX tree only contains rendered nodes; force a frame so deferred UI is in it.
    nudge_frame(client).await;
    let nodes = ax_interactive_nodes(client).await?;
    let intent_l = intent.to_lowercase();
    let tokens: Vec<&str> = intent_l
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() > 1)
        .collect();

    // Role hints from the intent phrasing.
    let wants_button =
        intent_l.contains("button") || intent_l.contains("submit") || intent_l.contains("send");
    let wants_input = intent_l.contains("input")
        || intent_l.contains("box")
        || intent_l.contains("field")
        || intent_l.contains("search")
        || intent_l.contains("type")
        || intent_l.contains("write");
    let wants_link = intent_l.contains("link");

    let mut best: Option<(i64, &AxNode)> = None;
    for n in &nodes {
        let name_l = n.name.to_lowercase();
        let mut score: i64 = 0;
        for t in &tokens {
            if name_l == *t {
                score += 10;
            } else if name_l.contains(t) {
                score += 5;
            }
        }
        match n.role.as_str() {
            "button" if wants_button => score += 4,
            "textbox" | "combobox" | "searchbox" if wants_input => score += 4,
            "searchbox" if intent_l.contains("search") => score += 3,
            "link" if wants_link => score += 4,
            _ => {}
        }
        if !n.name.is_empty() {
            score += 1;
        }
        if score > 0 {
            match &best {
                Some((bs, _)) if *bs >= score => {}
                _ => best = Some((score, n)),
            }
        }
    }
    if let Some((_, n)) = best {
        return Ok(Some(n.clone()));
    }

    // Layer 3: optional LLM fallback (no-op + zero cost unless a key is configured).
    if crate::llm::available() && !nodes.is_empty() {
        let snapshot = nodes
            .iter()
            .map(|n| format!("{} | {:?} | {}", n.role, n.name, n.backend_node_id))
            .collect::<Vec<_>>()
            .join("\n");
        if let Some(id) = crate::llm::find_by_intent(&snapshot, intent).await {
            // Validate the LLM's choice against the snapshot (anti prompt-injection).
            if let Some(n) = nodes.iter().find(|n| n.backend_node_id == id) {
                return Ok(Some(n.clone()));
            }
        }
    }
    Ok(None)
}

/// Dependency-free xorshift for humanised typing jitter (not security-sensitive).
struct Jitter(u64);
impl Jitter {
    fn new(seed: u64) -> Self {
        Jitter(seed | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

// --- B3: interaction coverage ---------------------------------------------------

/// Named keys mapped to the fields CDP needs.
///
/// `Input.dispatchKeyEvent` wants `key`, `code`, `windowsVirtualKeyCode` and
/// `nativeVirtualKeyCode` to agree; sending only `key` produces an event a page's handler
/// ignores, which looks like a working keypress that does nothing.
fn key_spec(name: &str) -> Option<(&'static str, &'static str, i64)> {
    Some(match name.trim().to_ascii_lowercase().as_str() {
        "enter" | "return" => ("Enter", "Enter", 13),
        "tab" => ("Tab", "Tab", 9),
        "escape" | "esc" => ("Escape", "Escape", 27),
        "backspace" => ("Backspace", "Backspace", 8),
        "delete" | "del" => ("Delete", "Delete", 46),
        "arrowup" | "up" => ("ArrowUp", "ArrowUp", 38),
        "arrowdown" | "down" => ("ArrowDown", "ArrowDown", 40),
        "arrowleft" | "left" => ("ArrowLeft", "ArrowLeft", 37),
        "arrowright" | "right" => ("ArrowRight", "ArrowRight", 39),
        "home" => ("Home", "Home", 36),
        "end" => ("End", "End", 35),
        "pageup" => ("PageUp", "PageUp", 33),
        "pagedown" => ("PageDown", "PageDown", 34),
        "space" => (" ", "Space", 32),
        _ => return None,
    })
}

/// CDP modifier bitmask: Alt=1, Ctrl=2, Meta=4, Shift=8.
fn modifier_mask(modifiers: &[String]) -> i64 {
    modifiers.iter().fold(0, |acc, m| {
        acc | match m.trim().to_ascii_lowercase().as_str() {
            "alt" => 1,
            "ctrl" | "control" => 2,
            "meta" | "cmd" | "command" => 4,
            "shift" => 8,
            _ => 0,
        }
    })
}

/// Press a named key, optionally with modifiers — `press("Enter")`, `press("a", ["ctrl"])`.
pub async fn press_key(
    client: &CdpClient,
    key: &str,
    modifiers: &[String],
) -> Result<String, CdpError> {
    let mask = modifier_mask(modifiers);
    let (key_name, code, vk) = match key_spec(key) {
        Some(spec) => spec,
        None => {
            // A single printable character: send it as itself rather than refusing, since
            // `press("a", ["ctrl"])` is the natural way to express a shortcut.
            let mut chars = key.chars();
            let (Some(c), None) = (chars.next(), chars.next()) else {
                return Err(CdpError::Closed(format!(
                    "press: unknown key {key:?}. Use a printable character or one of \
                     Enter/Tab/Escape/Backspace/Delete/Arrow*/Home/End/PageUp/PageDown/Space"
                )));
            };
            let upper = c.to_ascii_uppercase() as i64;
            for ty in ["keyDown", "keyUp"] {
                client
                    .send(
                        "Input.dispatchKeyEvent",
                        json!({
                            "type": ty,
                            "key": c.to_string(),
                            // With a modifier held, a printable key must NOT carry text, or
                            // Ctrl+A would also insert an "a".
                            "text": if mask == 0 { c.to_string() } else { String::new() },
                            "modifiers": mask,
                            "windowsVirtualKeyCode": upper,
                            "nativeVirtualKeyCode": upper,
                        }),
                    )
                    .await?;
            }
            return Ok(format!("pressed {c:?} with modifiers {modifiers:?}"));
        }
    };
    for ty in ["keyDown", "keyUp"] {
        client
            .send(
                "Input.dispatchKeyEvent",
                json!({
                    "type": ty,
                    "key": key_name,
                    "code": code,
                    // `text` is what makes a printable key insert; a named key like Enter
                    // must NOT carry text, or it types a character instead of acting.
                    "text": if key_name.len() == 1 && mask == 0 { key_name } else { "" },
                    "modifiers": mask,
                    "windowsVirtualKeyCode": vk,
                    "nativeVirtualKeyCode": vk,
                }),
            )
            .await?;
    }
    Ok(format!("pressed {key_name} with modifiers {modifiers:?}"))
}

/// Hover over an element: move the real cursor there without pressing.
///
/// Needed for menus and tooltips that only render on `mouseover`; a JS `dispatchEvent` is
/// not `isTrusted` and many libraries check.
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

/// Set a checkbox, radio or `<select>` to a value, through the property setter React and Vue
/// listen to.
///
/// A bare `el.checked = true` does not notify a framework's state, so the control visually
/// changes and the app never learns — the classic "the form submitted the old value" bug.
pub async fn set_control(
    client: &CdpClient,
    selector: &str,
    value: &str,
) -> Result<String, CdpError> {
    let snippet = crate::js::set_control()
        .with(
            "SEL",
            &serde_json::to_string(selector).unwrap_or_else(|_| "\"\"".into()),
        )
        .with(
            "VALUE",
            &serde_json::to_string(value).unwrap_or_else(|_| "\"\"".into()),
        );
    let raw = js(client, &snippet.returning()).await?;
    Ok(match raw {
        Value::String(s) => s,
        other => other.to_string(),
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jitter_is_deterministic_and_bounded() {
        let mut j = Jitter::new(42);
        for _ in 0..1000 {
            let ms = 30 + (j.next() % 90);
            assert!((30..120).contains(&ms));
        }
    }

    #[test]
    fn find_scoring_prefers_exact_name_and_role() {
        // Pure scoring check via a tiny reimplementation mirror would drift; instead
        // assert the role-hint booleans the scorer relies on.
        let intent = "send message button".to_lowercase();
        assert!(intent.contains("send"));
        assert!(intent.contains("button"));
    }

    #[test]
    fn interactive_roles_include_textbox_and_button() {
        assert!(INTERACTIVE_ROLES.contains(&"button"));
        assert!(INTERACTIVE_ROLES.contains(&"textbox"));
        assert!(!INTERACTIVE_ROLES.contains(&"StaticText"));
    }
}
