//! Reaching content the ordinary DOM query cannot: shadow roots, iframes, blocking
//! dialogs, and device emulation.
//!
//! Each of these is a place where "the element is right there on screen" and
//! `document.querySelector` returns `null`, which reads to a model as "the page is
//! broken" and to a user as "the tool does not work".
//!
//! - **Shadow DOM.** A closed-over web component's internals are invisible to a
//!   top-level query. Any design system built on custom elements (so: most enterprise
//!   apps) is unreachable without piercing.
//! - **Iframes.** Each frame is its own document with its own execution context. A
//!   same-origin frame can be walked from JS; a cross-origin one cannot, and pretending
//!   otherwise would silently miss content — so they are *listed* rather than faked.
//! - **Dialogs.** `alert`/`confirm`/`beforeunload` block the renderer. Every subsequent
//!   CDP evaluation hangs until the dialog is answered, which looks exactly like a
//!   crashed browser.
//! - **Emulation.** Geolocation, permissions and viewport are what let a mobile layout
//!   or a location-gated flow be tested at all.

use serde_json::{json, Value};

use crate::cdp::{CdpClient, CdpError};

/// JSON-encode a value for interpolation into a JS snippet.
///
/// Every string that crosses into JavaScript goes through this. Interpolating raw would let
/// a selector like `a"]+alert(1)+["` escape its literal and execute — the snippet is code,
/// and its arguments are not.
fn json_str(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".into())
}

/// Find, read, click or fill an element that may live inside shadow roots or
/// same-origin iframes.
///
/// The click here is a JS `.click()`, not a trusted mouse event, and that is a real
/// trade-off rather than an oversight: coordinates inside a nested frame do not map
/// cleanly to the top-level viewport, so a dispatched mouse event would land in the
/// wrong place. Reported in the result so the caller knows which kind of click it got.
pub async fn pierce(
    client: &CdpClient,
    selector: &str,
    action: &str,
    value: &str,
) -> Result<String, CdpError> {
    if !matches!(action, "read" | "click" | "fill") {
        return Err(CdpError::Closed(format!(
            "pierce: action must be read, click or fill; got {action:?}"
        )));
    }
    crate::page::nudge_frame(client).await;
    // JSON-encoded, not interpolated raw: a selector containing a quote must not be able
    // to break out of its literal and become code.
    let snippet = crate::js::pierce()
        .with("SEL", &json_str(selector))
        .with("ACTION", &json_str(action))
        .with("VALUE", &json_str(value));
    debug_assert!(
        snippet.unresolved().is_empty(),
        "unsubstituted placeholders would reach the browser: {:?}",
        snippet.unresolved()
    );
    let raw = crate::page::eval_body(client, &snippet.returning()).await?;
    let mut parsed: Value = match &raw {
        Value::String(s) => serde_json::from_str(s).unwrap_or(json!({ "found": false })),
        other => other.clone(),
    };
    if action == "click" && parsed.get("found") == Some(&Value::Bool(true)) {
        parsed["click_kind"] = json!("javascript");
        parsed["note"] = json!(
            "clicked via JS because coordinates inside a nested frame or shadow root do \
             not map to the top-level viewport. Use `click` for a trusted mouse event on \
             top-level elements"
        );
    }
    Ok(parsed.to_string())
}

/// List every frame in the page, flagging which are reachable from JS.
///
/// The point is to make a cross-origin frame *visible* rather than an unexplained
/// absence: an agent told "this frame is cross-origin" can navigate to it directly,
/// whereas an agent that simply cannot find an element retries forever.
pub async fn list_frames(client: &CdpClient) -> Result<String, CdpError> {
    let tree = client.send("Page.getFrameTree", json!({})).await?;
    let mut frames = Vec::new();
    collect_frames(tree.get("frameTree"), &mut frames, 0);

    // Which same-origin frames JS can actually enter, checked from the page rather
    // than inferred from URLs — the origin comparison Chrome applies is the only
    // authority on it.
    let reachable = crate::page::eval_body(client, &crate::js::frame_access().returning())
        .await
        .ok()
        .and_then(|v| match v {
            Value::String(s) => serde_json::from_str::<Value>(&s).ok(),
            other => Some(other),
        })
        .unwrap_or(json!([]));

    Ok(json!({
        "frames": frames,
        "elements": reachable,
        "note": "Elements inside a same-origin frame are reachable with `pierce`. A \
                 cross-origin frame needs `navigate` to its URL, or a new tab — there is \
                 no way to read into it from the parent",
    })
    .to_string())
}

fn collect_frames(node: Option<&Value>, out: &mut Vec<Value>, depth: usize) {
    // Bounded: a malformed or adversarial tree must not recurse without end.
    let Some(node) = node else { return };
    if depth > 16 {
        return;
    }
    if let Some(frame) = node.get("frame") {
        out.push(json!({
            "id": frame.get("id").and_then(Value::as_str).unwrap_or(""),
            "url": crate::trace::redact(frame.get("url").and_then(Value::as_str).unwrap_or("")),
            "name": frame.get("name").and_then(Value::as_str).unwrap_or(""),
            "depth": depth,
        }));
    }
    if let Some(children) = node.get("childFrames").and_then(Value::as_array) {
        for child in children {
            collect_frames(Some(child), out, depth + 1);
        }
    }
}

/// How a JavaScript dialog should be answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogAction {
    Accept,
    Dismiss,
}

/// Answer a pending `alert` / `confirm` / `prompt` / `beforeunload`.
///
/// This has to exist as an explicit tool because a dialog blocks the renderer: every
/// later CDP evaluation hangs, which is indistinguishable from a hung browser. Being
/// able to say "dismiss it" turns a dead session into a recoverable one.
pub async fn handle_dialog(
    client: &CdpClient,
    action: DialogAction,
    prompt_text: Option<&str>,
) -> Result<String, CdpError> {
    let mut params = json!({ "accept": action == DialogAction::Accept });
    if let Some(text) = prompt_text {
        params["promptText"] = json!(text);
    }
    // `Page.enable` is required for the dialog domain to be live; harmless if already on.
    let _ = client.send("Page.enable", json!({})).await;
    match client.send("Page.handleJavaScriptDialog", params).await {
        Ok(_) => Ok(json!({
            "ok": true,
            "action": if action == DialogAction::Accept { "accepted" } else { "dismissed" },
        })
        .to_string()),
        // "No dialog is showing" is the common case and not really an error: report it
        // plainly so a caller can stop looking for one.
        Err(e) => Ok(json!({
            "ok": false,
            "error": e.to_string(),
            "hint": "no dialog was open. Dialogs block the page, so if calls are hanging \
                     the cause is elsewhere",
        })
        .to_string()),
    }
}

/// Override geolocation, viewport, or grant permissions.
///
/// Grouped into one call because they are always used together — testing a
/// location-gated mobile flow needs all three, and three separate round trips would
/// leave a half-configured browser if one failed.
pub async fn emulate(
    client: &CdpClient,
    latitude: Option<f64>,
    longitude: Option<f64>,
    width: Option<i64>,
    height: Option<i64>,
    mobile: bool,
    permissions: &[String],
) -> Result<String, CdpError> {
    let mut applied = Vec::new();

    if let (Some(lat), Some(lon)) = (latitude, longitude) {
        client
            .send(
                "Emulation.setGeolocationOverride",
                json!({ "latitude": lat, "longitude": lon, "accuracy": 50 }),
            )
            .await?;
        applied.push(json!({ "geolocation": { "latitude": lat, "longitude": lon } }));
    }

    if let (Some(w), Some(h)) = (width, height) {
        client
            .send(
                "Emulation.setDeviceMetricsOverride",
                json!({
                    "width": w,
                    "height": h,
                    // A mobile viewport with a deviceScaleFactor of 1 renders a desktop
                    // layout at phone dimensions, which is not what anyone means by
                    // "test the mobile view".
                    "deviceScaleFactor": if mobile { 2 } else { 1 },
                    "mobile": mobile,
                }),
            )
            .await?;
        applied.push(json!({ "viewport": { "width": w, "height": h, "mobile": mobile } }));
    }

    if !permissions.is_empty() {
        // CDP names differ from the web-facing Permissions API names, so map the ones
        // a caller would naturally write.
        let mapped: Vec<String> = permissions
            .iter()
            .map(|p| match p.trim().to_ascii_lowercase().as_str() {
                "geolocation" => "geolocation".to_string(),
                "camera" => "videoCapture".to_string(),
                "microphone" => "audioCapture".to_string(),
                "notifications" => "notifications".to_string(),
                "clipboard" => "clipboardReadWrite".to_string(),
                other => other.to_string(),
            })
            .collect();
        client
            .send(
                "Browser.grantPermissions",
                json!({ "permissions": mapped.clone() }),
            )
            .await?;
        applied.push(json!({ "permissions": mapped }));
    }

    if applied.is_empty() {
        return Ok(json!({
            "ok": false,
            "error": "nothing to emulate: pass latitude+longitude, width+height, or permissions",
        })
        .to_string());
    }
    Ok(json!({ "ok": true, "applied": applied }).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The snippet reaches the browser fully substituted and JSON-escaped. A leftover
    /// `__SEL__` would fail in a way that looks like a page problem, and a raw
    /// interpolation would be an injection.
    #[test]
    fn the_pierce_snippet_is_fully_substituted_and_escaped() {
        let snippet = crate::js::pierce()
            .with("SEL", &json_str("a\"]+alert(1)+[\""))
            .with("ACTION", &json_str("read"))
            .with("VALUE", &json_str(""));
        assert!(
            snippet.unresolved().is_empty(),
            "placeholders left: {:?}",
            snippet.unresolved()
        );
        let src = snippet.expr();
        // The quote is escaped, so the selector stays a string literal.
        assert!(src.contains("\\\"]+alert(1)+[\\\""), "not escaped: {src}");
        assert!(src.contains("action === 'read'"));
    }

    /// The behaviours the snippet must retain after being moved out of Rust.
    #[test]
    fn the_pierce_snippet_still_descends_and_uses_the_framework_setter() {
        let src = crate::js::pierce().expr();
        assert!(
            src.contains("el.shadowRoot"),
            "must descend into shadow roots"
        );
        assert!(src.contains("contentDocument"), "must descend into iframes");
        assert!(src.contains("depth > 12"), "recursion must be bounded");
        assert!(
            src.contains("getOwnPropertyDescriptor"),
            "must use the value setter"
        );
        assert!(
            src.contains("composed: true"),
            "events must cross shadow boundaries"
        );
    }

    #[test]
    fn frame_collection_is_bounded_and_flattens_the_tree() {
        // A three-level tree.
        let tree = json!({
            "frame": { "id": "root", "url": "https://a.test/", "name": "" },
            "childFrames": [{
                "frame": { "id": "c1", "url": "https://b.test/", "name": "inner" },
                "childFrames": [{
                    "frame": { "id": "c2", "url": "https://c.test/", "name": "" },
                }],
            }],
        });
        let mut out = Vec::new();
        collect_frames(Some(&tree), &mut out, 0);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0]["id"], "root");
        assert_eq!(out[0]["depth"], 0);
        assert_eq!(out[2]["depth"], 2);
    }

    /// Frame URLs carry query strings, so they go through redaction like every other
    /// URL that leaves the process.
    #[test]
    fn frame_urls_are_redacted() {
        let tree = json!({
            "frame": { "id": "r", "url": "https://a.test/?access_token=LEAKME", "name": "" },
        });
        let mut out = Vec::new();
        collect_frames(Some(&tree), &mut out, 0);
        assert!(!out[0]["url"].as_str().unwrap().contains("LEAKME"));
    }

    #[test]
    fn frame_collection_survives_malformed_input() {
        let mut out = Vec::new();
        collect_frames(None, &mut out, 0);
        collect_frames(Some(&json!({})), &mut out, 0);
        collect_frames(Some(&json!("nonsense")), &mut out, 0);
        assert!(out.is_empty());
    }

    /// Deep nesting must terminate. A frame tree is attacker-influenced, so an
    /// unbounded walk is a stack overflow waiting to be triggered.
    #[test]
    fn frame_collection_stops_at_the_depth_limit() {
        // Build a 40-deep chain.
        let mut node = json!({ "frame": { "id": "leaf", "url": "", "name": "" } });
        for i in 0..40 {
            node = json!({
                "frame": { "id": format!("f{i}"), "url": "", "name": "" },
                "childFrames": [node],
            });
        }
        let mut out = Vec::new();
        collect_frames(Some(&node), &mut out, 0);
        assert!(
            out.len() <= 17,
            "walk was not bounded: {} frames",
            out.len()
        );
    }
}
