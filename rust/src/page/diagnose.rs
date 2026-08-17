//! Why a click cannot land, in terms a caller can act on.
//!
//! When an action fails, "it failed" is never the useful answer. "A div with z-index 9999
//! covers the target" and "the control is disabled" are — and both are knowable *before*
//! dispatching anything. That is the whole reason this is separate from resolution: finding
//! an element is a mechanical lookup, while explaining why it is unreachable is a diagnosis,
//! and the diagnosis is what turns a `blocked` status into something better than a slow
//! `failed`.
//!
//! The conformance suite found this module short of one check: a disabled control was being
//! clicked anyway and reported as `uncertain`, withholding a diagnosis that cost one round
//! trip to make. `refuses_input` is that check.

use crate::cdp::{CdpClient, CdpError};
use serde_json::json;

use super::node::resolve_object_id;

/// Hit-test (cx, cy): return a description of the element on top when it is
/// NOT the target nor one of its descendants, else None.
///
/// Descendants must pass: the centre of a `<button>` usually lands on an inner
/// `<span>`, which is a perfectly good click. What has to be rejected is a node
/// from a different branch — that is the overlay case.
/// Whether the control refuses input, and why — checked *before* dispatching.
///
/// Three mechanisms, all equivalent from the caller's point of view and all invisible to a
/// hit test: the `disabled` attribute, `aria-disabled` (which a custom component uses because
/// a `<div role="button">` has no `disabled`), and `pointer-events: none`. The last one is
/// worth naming separately because the element is fully visible and looks clickable.
///
/// This exists because reporting `uncertain` here is a wasted diagnosis. The information —
/// "this button is disabled" — is available for the cost of one round trip, and without it a
/// caller retries the same click indefinitely instead of filling in whatever field keeps the
/// control disabled. Conformance scenario C4 is the assertion that we look.
///
/// Best-effort, like `obscured_by`: if the check itself cannot run, it reports nothing rather
/// than blocking a click that would have worked.
pub(super) async fn refuses_input(
    client: &CdpClient,
    backend_node_id: i64,
) -> Result<Option<String>, CdpError> {
    let Some(obj) = resolve_object_id(client, backend_node_id).await? else {
        return Ok(None);
    };
    let res = client
        .send(
            "Runtime.callFunctionOn",
            json!({
                "objectId": obj,
                "functionDeclaration": "function(){ \
                    if (this.disabled === true) return 'the control is disabled'; \
                    var a = this.getAttribute && this.getAttribute('aria-disabled'); \
                    if (a === 'true') return 'the control is aria-disabled'; \
                    var cs = window.getComputedStyle ? window.getComputedStyle(this) : null; \
                    if (cs && cs.pointerEvents === 'none') \
                        return 'the control has pointer-events: none'; \
                    var f = this.closest && this.closest('fieldset[disabled]'); \
                    if (f) return 'the control is inside a disabled fieldset'; \
                    return null; \
                }",
                "returnByValue": true,
            }),
        )
        .await;
    let Ok(res) = res else { return Ok(None) };
    Ok(res
        .get("result")
        .and_then(|r| r.get("value"))
        .and_then(|v| v.as_str())
        .map(str::to_string))
}

/// What is sitting on top of the click point, if anything.
///
/// Hit-tests the coordinates and reports the element found there when it is neither the
/// target nor related to it. Best-effort: if the hit test cannot run, it reports nothing
/// rather than blocking a click that would have worked.
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

/// A digest of everything a click can change *about the target itself*.
///
/// The discriminator that makes "the page changed" mean "the action changed it". A whole-page
/// comparison cannot tell the two apart: on a real shop an `add to cart` click that never
/// landed was reported as `succeeded` because `text` had changed — the page was still settling
/// from the previous navigation, and the residue was credited to the click.
///
/// Requiring a *structural* change instead would break the opposite case, which is just as
/// common: a button whose click only rewrites its own label (`Add to cart` becoming `Remove`,
/// an accordion toggling) changes nothing but text. So the question is not how big the change
/// was but whether it touched the thing that was clicked.
///
/// Covers what a click plausibly alters on an element: its label, its value, its checked and
/// disabled state, the ARIA properties that carry toggle state, its class list (how a
/// framework signals selection), and its position among its siblings. `None` means the node is
/// gone — which is itself strong evidence, since a click that removes its own target did
/// something.
pub async fn element_fingerprint(client: &CdpClient, backend_node_id: i64) -> Option<String> {
    let obj = resolve_object_id(client, backend_node_id).await.ok()??;
    let res = client
        .send(
            "Runtime.callFunctionOn",
            json!({
                "objectId": obj,
                "functionDeclaration": "function(){ \
                    if (!this.isConnected) return null; \
                    var a = ['aria-expanded','aria-pressed','aria-selected','aria-checked', \
                             'aria-disabled','disabled','hidden'] \
                        .map(function(n){ return n + '=' + (this.getAttribute(n) || ''); }, this) \
                        .join(','); \
                    var sib = this.parentElement \
                        ? Array.prototype.indexOf.call(this.parentElement.children, this) \
                          + '/' + this.parentElement.children.length \
                        : '-'; \
                    return [ \
                        (this.textContent || '').trim().slice(0, 120), \
                        this.value === undefined ? '' : String(this.value).slice(0, 60), \
                        this.checked === undefined ? '' : String(this.checked), \
                        (this.className && this.className.toString ? this.className.toString() : ''), \
                        a, sib \
                    ].join('\\u0001'); \
                }",
                "returnByValue": true,
            }),
        )
        .await
        .ok()?;
    res.get("result")
        .and_then(|r| r.get("value"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
}
