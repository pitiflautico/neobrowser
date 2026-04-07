//! CDP Input domain — mouse, keyboard, touch, drag, gestures.

use super::{CdpResult, CdpTransport};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// ── Modifier constants ─────────────────────────────────────────────

pub const MODIFIER_ALT: i32 = 1;
pub const MODIFIER_CTRL: i32 = 2;
pub const MODIFIER_META: i32 = 4; // Cmd on Mac
pub const MODIFIER_SHIFT: i32 = 8;

// ── Params ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DispatchMouseEventParams {
    #[serde(rename = "type")]
    pub type_: String, // "mousePressed", "mouseReleased", "mouseMoved", "mouseWheel"
    pub x: f64,
    pub y: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub button: Option<String>, // "none", "left", "middle", "right", "back", "forward"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub click_count: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifiers: Option<i32>, // 1=Alt, 2=Ctrl, 4=Meta, 8=Shift
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta_x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta_y: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pointer_type: Option<String>, // "mouse", "pen", "touch"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DispatchKeyEventParams {
    #[serde(rename = "type")]
    pub type_: String, // "keyDown", "keyUp", "rawKeyDown", "char"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifiers: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unmodified_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_identifier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub windows_virtual_key_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_virtual_key_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_repeat: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_keypad: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_system_key: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TouchPoint {
    pub x: f64,
    pub y: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub radius_x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub radius_y: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation_angle: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub force: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tangential_pressure: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tilt_x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tilt_y: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub twist: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<f64>,
}

// ── Low-level methods (direct CDP calls) ────────────────────────────

pub async fn dispatch_mouse_event(
    transport: &dyn CdpTransport,
    params: DispatchMouseEventParams,
) -> CdpResult<()> {
    transport
        .send("Input.dispatchMouseEvent", serde_json::to_value(&params)?)
        .await?;
    Ok(())
}

pub async fn dispatch_key_event(
    transport: &dyn CdpTransport,
    params: DispatchKeyEventParams,
) -> CdpResult<()> {
    transport
        .send("Input.dispatchKeyEvent", serde_json::to_value(&params)?)
        .await?;
    Ok(())
}

pub async fn dispatch_touch_event(
    transport: &dyn CdpTransport,
    type_: &str,
    touch_points: Vec<TouchPoint>,
    modifiers: Option<i32>,
    timestamp: Option<f64>,
) -> CdpResult<()> {
    let mut params = json!({
        "type": type_,
        "touchPoints": serde_json::to_value(&touch_points)?,
    });
    if let Some(m) = modifiers {
        params["modifiers"] = json!(m);
    }
    if let Some(t) = timestamp {
        params["timestamp"] = json!(t);
    }
    transport.send("Input.dispatchTouchEvent", params).await?;
    Ok(())
}

pub async fn insert_text(transport: &dyn CdpTransport, text: &str) -> CdpResult<()> {
    transport
        .send("Input.insertText", json!({ "text": text }))
        .await?;
    Ok(())
}

pub async fn ime_set_composition(
    transport: &dyn CdpTransport,
    text: &str,
    selection_start: i32,
    selection_end: i32,
    replacement_start: Option<i32>,
    replacement_end: Option<i32>,
) -> CdpResult<()> {
    let mut params = json!({
        "text": text,
        "selectionStart": selection_start,
        "selectionEnd": selection_end,
    });
    if let Some(rs) = replacement_start {
        params["replacementStart"] = json!(rs);
    }
    if let Some(re) = replacement_end {
        params["replacementEnd"] = json!(re);
    }
    transport.send("Input.imeSetComposition", params).await?;
    Ok(())
}

pub async fn set_ignore_input_events(
    transport: &dyn CdpTransport,
    ignore: bool,
) -> CdpResult<()> {
    transport
        .send("Input.setIgnoreInputEvents", json!({ "ignore": ignore }))
        .await?;
    Ok(())
}

pub async fn dispatch_drag_event(
    transport: &dyn CdpTransport,
    type_: &str,
    x: f64,
    y: f64,
    data: Value,
    modifiers: Option<i32>,
) -> CdpResult<()> {
    let mut params = json!({
        "type": type_,
        "x": x,
        "y": y,
        "data": data,
    });
    if let Some(m) = modifiers {
        params["modifiers"] = json!(m);
    }
    transport.send("Input.dispatchDragEvent", params).await?;
    Ok(())
}

pub async fn set_intercept_drags(
    transport: &dyn CdpTransport,
    enabled: bool,
) -> CdpResult<()> {
    transport
        .send("Input.setInterceptDrags", json!({ "enabled": enabled }))
        .await?;
    Ok(())
}

pub async fn synthesize_scroll_gesture(
    transport: &dyn CdpTransport,
    x: f64,
    y: f64,
    x_distance: Option<f64>,
    y_distance: Option<f64>,
    x_overscroll: Option<f64>,
    y_overscroll: Option<f64>,
    prevent_fling: Option<bool>,
    speed: Option<i32>,
    gesture_source_type: Option<&str>,
    repeat_count: Option<i32>,
    repeat_delay_ms: Option<i32>,
) -> CdpResult<()> {
    let mut params = json!({ "x": x, "y": y });
    if let Some(v) = x_distance {
        params["xDistance"] = json!(v);
    }
    if let Some(v) = y_distance {
        params["yDistance"] = json!(v);
    }
    if let Some(v) = x_overscroll {
        params["xOverscroll"] = json!(v);
    }
    if let Some(v) = y_overscroll {
        params["yOverscroll"] = json!(v);
    }
    if let Some(v) = prevent_fling {
        params["preventFling"] = json!(v);
    }
    if let Some(v) = speed {
        params["speed"] = json!(v);
    }
    if let Some(v) = gesture_source_type {
        params["gestureSourceType"] = json!(v);
    }
    if let Some(v) = repeat_count {
        params["repeatCount"] = json!(v);
    }
    if let Some(v) = repeat_delay_ms {
        params["repeatDelayMs"] = json!(v);
    }
    transport
        .send("Input.synthesizeScrollGesture", params)
        .await?;
    Ok(())
}

pub async fn synthesize_tap_gesture(
    transport: &dyn CdpTransport,
    x: f64,
    y: f64,
    duration: Option<i32>,
    tap_count: Option<i32>,
    gesture_source_type: Option<&str>,
) -> CdpResult<()> {
    let mut params = json!({ "x": x, "y": y });
    if let Some(v) = duration {
        params["duration"] = json!(v);
    }
    if let Some(v) = tap_count {
        params["tapCount"] = json!(v);
    }
    if let Some(v) = gesture_source_type {
        params["gestureSourceType"] = json!(v);
    }
    transport
        .send("Input.synthesizeTapGesture", params)
        .await?;
    Ok(())
}

pub async fn synthesize_pinch_gesture(
    transport: &dyn CdpTransport,
    x: f64,
    y: f64,
    scale_factor: f64,
    relative_speed: Option<i32>,
    gesture_source_type: Option<&str>,
) -> CdpResult<()> {
    let mut params = json!({
        "x": x,
        "y": y,
        "scaleFactor": scale_factor,
    });
    if let Some(v) = relative_speed {
        params["relativeSpeed"] = json!(v);
    }
    if let Some(v) = gesture_source_type {
        params["gestureSourceType"] = json!(v);
    }
    transport
        .send("Input.synthesizePinchGesture", params)
        .await?;
    Ok(())
}

// ── High-level helpers ──────────────────────────────────────────────

/// Click at coordinates — move + press + release.
pub async fn click(transport: &dyn CdpTransport, x: f64, y: f64) -> CdpResult<()> {
    dispatch_mouse_event(
        transport,
        DispatchMouseEventParams {
            type_: "mouseMoved".into(),
            x,
            y,
            ..Default::default()
        },
    )
    .await?;
    dispatch_mouse_event(
        transport,
        DispatchMouseEventParams {
            type_: "mousePressed".into(),
            x,
            y,
            button: Some("left".into()),
            click_count: Some(1),
            ..Default::default()
        },
    )
    .await?;
    dispatch_mouse_event(
        transport,
        DispatchMouseEventParams {
            type_: "mouseReleased".into(),
            x,
            y,
            button: Some("left".into()),
            click_count: Some(1),
            ..Default::default()
        },
    )
    .await?;
    Ok(())
}

/// Double-click at coordinates.
pub async fn double_click(transport: &dyn CdpTransport, x: f64, y: f64) -> CdpResult<()> {
    dispatch_mouse_event(
        transport,
        DispatchMouseEventParams {
            type_: "mouseMoved".into(),
            x,
            y,
            ..Default::default()
        },
    )
    .await?;
    dispatch_mouse_event(
        transport,
        DispatchMouseEventParams {
            type_: "mousePressed".into(),
            x,
            y,
            button: Some("left".into()),
            click_count: Some(1),
            ..Default::default()
        },
    )
    .await?;
    dispatch_mouse_event(
        transport,
        DispatchMouseEventParams {
            type_: "mouseReleased".into(),
            x,
            y,
            button: Some("left".into()),
            click_count: Some(1),
            ..Default::default()
        },
    )
    .await?;
    dispatch_mouse_event(
        transport,
        DispatchMouseEventParams {
            type_: "mousePressed".into(),
            x,
            y,
            button: Some("left".into()),
            click_count: Some(2),
            ..Default::default()
        },
    )
    .await?;
    dispatch_mouse_event(
        transport,
        DispatchMouseEventParams {
            type_: "mouseReleased".into(),
            x,
            y,
            button: Some("left".into()),
            click_count: Some(2),
            ..Default::default()
        },
    )
    .await?;
    Ok(())
}

/// Right-click at coordinates.
pub async fn right_click(transport: &dyn CdpTransport, x: f64, y: f64) -> CdpResult<()> {
    dispatch_mouse_event(
        transport,
        DispatchMouseEventParams {
            type_: "mouseMoved".into(),
            x,
            y,
            ..Default::default()
        },
    )
    .await?;
    dispatch_mouse_event(
        transport,
        DispatchMouseEventParams {
            type_: "mousePressed".into(),
            x,
            y,
            button: Some("right".into()),
            click_count: Some(1),
            ..Default::default()
        },
    )
    .await?;
    dispatch_mouse_event(
        transport,
        DispatchMouseEventParams {
            type_: "mouseReleased".into(),
            x,
            y,
            button: Some("right".into()),
            click_count: Some(1),
            ..Default::default()
        },
    )
    .await?;
    Ok(())
}

/// Type text using Input.insertText (doesn't fire individual key events).
pub async fn type_text(transport: &dyn CdpTransport, text: &str) -> CdpResult<()> {
    insert_text(transport, text).await
}

/// Press a key (keyDown + keyUp).
pub async fn press_key(
    transport: &dyn CdpTransport,
    key: &str,
    code: Option<&str>,
    modifiers: Option<i32>,
) -> CdpResult<()> {
    dispatch_key_event(
        transport,
        DispatchKeyEventParams {
            type_: "keyDown".into(),
            key: Some(key.into()),
            code: code.map(|c| c.into()),
            modifiers,
            ..Default::default()
        },
    )
    .await?;
    dispatch_key_event(
        transport,
        DispatchKeyEventParams {
            type_: "keyUp".into(),
            key: Some(key.into()),
            code: code.map(|c| c.into()),
            modifiers,
            ..Default::default()
        },
    )
    .await?;
    Ok(())
}

/// Scroll at coordinates (delta in pixels, negative deltaY = scroll down).
pub async fn scroll(
    transport: &dyn CdpTransport,
    x: f64,
    y: f64,
    delta_x: f64,
    delta_y: f64,
) -> CdpResult<()> {
    dispatch_mouse_event(
        transport,
        DispatchMouseEventParams {
            type_: "mouseWheel".into(),
            x,
            y,
            delta_x: Some(delta_x),
            delta_y: Some(delta_y),
            ..Default::default()
        },
    )
    .await
}

/// Drag from one point to another.
pub async fn drag(
    transport: &dyn CdpTransport,
    from_x: f64,
    from_y: f64,
    to_x: f64,
    to_y: f64,
    steps: Option<i32>,
) -> CdpResult<()> {
    let steps = steps.unwrap_or(10);

    // Move to start
    dispatch_mouse_event(
        transport,
        DispatchMouseEventParams {
            type_: "mouseMoved".into(),
            x: from_x,
            y: from_y,
            ..Default::default()
        },
    )
    .await?;

    // Press
    dispatch_mouse_event(
        transport,
        DispatchMouseEventParams {
            type_: "mousePressed".into(),
            x: from_x,
            y: from_y,
            button: Some("left".into()),
            click_count: Some(1),
            ..Default::default()
        },
    )
    .await?;

    // Intermediate moves
    for i in 1..=steps {
        let ratio = i as f64 / steps as f64;
        let cx = from_x + (to_x - from_x) * ratio;
        let cy = from_y + (to_y - from_y) * ratio;
        dispatch_mouse_event(
            transport,
            DispatchMouseEventParams {
                type_: "mouseMoved".into(),
                x: cx,
                y: cy,
                button: Some("left".into()),
                ..Default::default()
            },
        )
        .await?;
    }

    // Release
    dispatch_mouse_event(
        transport,
        DispatchMouseEventParams {
            type_: "mouseReleased".into(),
            x: to_x,
            y: to_y,
            button: Some("left".into()),
            click_count: Some(1),
            ..Default::default()
        },
    )
    .await?;

    Ok(())
}

/// Move mouse to coordinates.
pub async fn mouse_move(transport: &dyn CdpTransport, x: f64, y: f64) -> CdpResult<()> {
    dispatch_mouse_event(
        transport,
        DispatchMouseEventParams {
            type_: "mouseMoved".into(),
            x,
            y,
            ..Default::default()
        },
    )
    .await
}

/// Hold modifier keys while pressing the last key in the sequence.
/// e.g., key_combo(transport, &["Control", "a"]) sends Ctrl+A.
pub async fn key_combo(transport: &dyn CdpTransport, keys: &[&str]) -> CdpResult<()> {
    if keys.is_empty() {
        return Ok(());
    }

    // Build modifier mask from all keys except the last
    let mut modifiers = 0i32;
    for &k in &keys[..keys.len() - 1] {
        match k {
            "Alt" => modifiers |= MODIFIER_ALT,
            "Control" => modifiers |= MODIFIER_CTRL,
            "Meta" | "Command" => modifiers |= MODIFIER_META,
            "Shift" => modifiers |= MODIFIER_SHIFT,
            _ => {} // non-modifier keys in prefix treated as regular
        }
    }

    // Press modifier keys down
    for &k in &keys[..keys.len() - 1] {
        dispatch_key_event(
            transport,
            DispatchKeyEventParams {
                type_: "keyDown".into(),
                key: Some(k.into()),
                modifiers: Some(modifiers),
                ..Default::default()
            },
        )
        .await?;
    }

    // Press + release the final key
    let final_key = keys[keys.len() - 1];
    dispatch_key_event(
        transport,
        DispatchKeyEventParams {
            type_: "keyDown".into(),
            key: Some(final_key.into()),
            modifiers: Some(modifiers),
            ..Default::default()
        },
    )
    .await?;
    dispatch_key_event(
        transport,
        DispatchKeyEventParams {
            type_: "keyUp".into(),
            key: Some(final_key.into()),
            modifiers: Some(modifiers),
            ..Default::default()
        },
    )
    .await?;

    // Release modifier keys (reverse order)
    for &k in keys[..keys.len() - 1].iter().rev() {
        dispatch_key_event(
            transport,
            DispatchKeyEventParams {
                type_: "keyUp".into(),
                key: Some(k.into()),
                modifiers: Some(0),
                ..Default::default()
            },
        )
        .await?;
    }

    Ok(())
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdp::MockTransport;

    #[tokio::test]
    async fn test_dispatch_mouse_event() {
        let mock = MockTransport::new();
        mock.set_default_response(json!({})).await;

        let params = DispatchMouseEventParams {
            type_: "mousePressed".into(),
            x: 100.0,
            y: 200.0,
            button: Some("left".into()),
            click_count: Some(1),
            ..Default::default()
        };
        dispatch_mouse_event(&mock, params).await.unwrap();

        mock.assert_called("Input.dispatchMouseEvent", 1).await;
        let p = mock.call_params("Input.dispatchMouseEvent", 0).await.unwrap();
        assert_eq!(p["type"], "mousePressed");
        assert_eq!(p["x"], 100.0);
        assert_eq!(p["y"], 200.0);
        assert_eq!(p["button"], "left");
        assert_eq!(p["clickCount"], 1);
    }

    #[tokio::test]
    async fn test_click() {
        let mock = MockTransport::new();
        mock.set_default_response(json!({})).await;

        click(&mock, 50.0, 75.0).await.unwrap();

        mock.assert_called("Input.dispatchMouseEvent", 3).await;

        let p0 = mock.call_params("Input.dispatchMouseEvent", 0).await.unwrap();
        assert_eq!(p0["type"], "mouseMoved");

        let p1 = mock.call_params("Input.dispatchMouseEvent", 1).await.unwrap();
        assert_eq!(p1["type"], "mousePressed");
        assert_eq!(p1["button"], "left");

        let p2 = mock.call_params("Input.dispatchMouseEvent", 2).await.unwrap();
        assert_eq!(p2["type"], "mouseReleased");
        assert_eq!(p2["button"], "left");
    }

    #[tokio::test]
    async fn test_double_click() {
        let mock = MockTransport::new();
        mock.set_default_response(json!({})).await;

        double_click(&mock, 10.0, 20.0).await.unwrap();

        // mouseMoved + press(1) + release(1) + press(2) + release(2) = 5
        mock.assert_called("Input.dispatchMouseEvent", 5).await;

        let p3 = mock.call_params("Input.dispatchMouseEvent", 3).await.unwrap();
        assert_eq!(p3["type"], "mousePressed");
        assert_eq!(p3["clickCount"], 2);

        let p4 = mock.call_params("Input.dispatchMouseEvent", 4).await.unwrap();
        assert_eq!(p4["type"], "mouseReleased");
        assert_eq!(p4["clickCount"], 2);
    }

    #[tokio::test]
    async fn test_right_click() {
        let mock = MockTransport::new();
        mock.set_default_response(json!({})).await;

        right_click(&mock, 30.0, 40.0).await.unwrap();

        mock.assert_called("Input.dispatchMouseEvent", 3).await;

        let p1 = mock.call_params("Input.dispatchMouseEvent", 1).await.unwrap();
        assert_eq!(p1["button"], "right");

        let p2 = mock.call_params("Input.dispatchMouseEvent", 2).await.unwrap();
        assert_eq!(p2["button"], "right");
    }

    #[tokio::test]
    async fn test_dispatch_key_event() {
        let mock = MockTransport::new();
        mock.set_default_response(json!({})).await;

        let params = DispatchKeyEventParams {
            type_: "keyDown".into(),
            key: Some("Enter".into()),
            code: Some("Enter".into()),
            modifiers: Some(MODIFIER_CTRL),
            ..Default::default()
        };
        dispatch_key_event(&mock, params).await.unwrap();

        mock.assert_called("Input.dispatchKeyEvent", 1).await;
        let p = mock.call_params("Input.dispatchKeyEvent", 0).await.unwrap();
        assert_eq!(p["type"], "keyDown");
        assert_eq!(p["key"], "Enter");
        assert_eq!(p["code"], "Enter");
        assert_eq!(p["modifiers"], MODIFIER_CTRL);
    }

    #[tokio::test]
    async fn test_press_key() {
        let mock = MockTransport::new();
        mock.set_default_response(json!({})).await;

        press_key(&mock, "Enter", Some("Enter"), None).await.unwrap();

        mock.assert_called("Input.dispatchKeyEvent", 2).await;

        let p0 = mock.call_params("Input.dispatchKeyEvent", 0).await.unwrap();
        assert_eq!(p0["type"], "keyDown");
        assert_eq!(p0["key"], "Enter");

        let p1 = mock.call_params("Input.dispatchKeyEvent", 1).await.unwrap();
        assert_eq!(p1["type"], "keyUp");
        assert_eq!(p1["key"], "Enter");
    }

    #[tokio::test]
    async fn test_type_text() {
        let mock = MockTransport::new();
        mock.set_default_response(json!({})).await;

        type_text(&mock, "Hello, world!").await.unwrap();

        mock.assert_called("Input.insertText", 1).await;
        let p = mock.call_params("Input.insertText", 0).await.unwrap();
        assert_eq!(p["text"], "Hello, world!");
    }

    #[tokio::test]
    async fn test_insert_text() {
        let mock = MockTransport::new();
        mock.set_default_response(json!({})).await;

        insert_text(&mock, "abc").await.unwrap();

        mock.assert_called("Input.insertText", 1).await;
        let p = mock.call_params("Input.insertText", 0).await.unwrap();
        assert_eq!(p["text"], "abc");
    }

    #[tokio::test]
    async fn test_scroll() {
        let mock = MockTransport::new();
        mock.set_default_response(json!({})).await;

        scroll(&mock, 400.0, 300.0, 0.0, -100.0).await.unwrap();

        mock.assert_called("Input.dispatchMouseEvent", 1).await;
        let p = mock.call_params("Input.dispatchMouseEvent", 0).await.unwrap();
        assert_eq!(p["type"], "mouseWheel");
        assert_eq!(p["x"], 400.0);
        assert_eq!(p["y"], 300.0);
        assert_eq!(p["deltaX"], 0.0);
        assert_eq!(p["deltaY"], -100.0);
    }

    #[tokio::test]
    async fn test_drag() {
        let mock = MockTransport::new();
        mock.set_default_response(json!({})).await;

        drag(&mock, 0.0, 0.0, 100.0, 100.0, Some(5)).await.unwrap();

        // move_to_start(1) + press(1) + intermediate_moves(5) + release(1) = 8
        mock.assert_called("Input.dispatchMouseEvent", 8).await;

        let p0 = mock.call_params("Input.dispatchMouseEvent", 0).await.unwrap();
        assert_eq!(p0["type"], "mouseMoved");
        assert_eq!(p0["x"], 0.0);

        let p1 = mock.call_params("Input.dispatchMouseEvent", 1).await.unwrap();
        assert_eq!(p1["type"], "mousePressed");

        let p7 = mock.call_params("Input.dispatchMouseEvent", 7).await.unwrap();
        assert_eq!(p7["type"], "mouseReleased");
        assert_eq!(p7["x"], 100.0);
        assert_eq!(p7["y"], 100.0);
    }

    #[tokio::test]
    async fn test_mouse_move() {
        let mock = MockTransport::new();
        mock.set_default_response(json!({})).await;

        mouse_move(&mock, 250.0, 350.0).await.unwrap();

        mock.assert_called("Input.dispatchMouseEvent", 1).await;
        let p = mock.call_params("Input.dispatchMouseEvent", 0).await.unwrap();
        assert_eq!(p["type"], "mouseMoved");
        assert_eq!(p["x"], 250.0);
        assert_eq!(p["y"], 350.0);
    }

    #[tokio::test]
    async fn test_touch_event() {
        let mock = MockTransport::new();
        mock.set_default_response(json!({})).await;

        let points = vec![TouchPoint {
            x: 100.0,
            y: 200.0,
            radius_x: Some(5.0),
            radius_y: Some(5.0),
            rotation_angle: None,
            force: Some(1.0),
            tangential_pressure: None,
            tilt_x: None,
            tilt_y: None,
            twist: None,
            id: Some(0.0),
        }];

        dispatch_touch_event(&mock, "touchStart", points, None, None)
            .await
            .unwrap();

        mock.assert_called("Input.dispatchTouchEvent", 1).await;
        let p = mock.call_params("Input.dispatchTouchEvent", 0).await.unwrap();
        assert_eq!(p["type"], "touchStart");
        assert_eq!(p["touchPoints"][0]["x"], 100.0);
        assert_eq!(p["touchPoints"][0]["y"], 200.0);
    }

    #[tokio::test]
    async fn test_key_combo() {
        let mock = MockTransport::new();
        mock.set_default_response(json!({})).await;

        // Ctrl+A
        key_combo(&mock, &["Control", "a"]).await.unwrap();

        // Control keyDown + "a" keyDown + "a" keyUp + Control keyUp = 4
        mock.assert_called("Input.dispatchKeyEvent", 4).await;

        let p0 = mock.call_params("Input.dispatchKeyEvent", 0).await.unwrap();
        assert_eq!(p0["type"], "keyDown");
        assert_eq!(p0["key"], "Control");
        assert_eq!(p0["modifiers"], MODIFIER_CTRL);

        let p1 = mock.call_params("Input.dispatchKeyEvent", 1).await.unwrap();
        assert_eq!(p1["type"], "keyDown");
        assert_eq!(p1["key"], "a");
        assert_eq!(p1["modifiers"], MODIFIER_CTRL);

        let p2 = mock.call_params("Input.dispatchKeyEvent", 2).await.unwrap();
        assert_eq!(p2["type"], "keyUp");
        assert_eq!(p2["key"], "a");

        let p3 = mock.call_params("Input.dispatchKeyEvent", 3).await.unwrap();
        assert_eq!(p3["type"], "keyUp");
        assert_eq!(p3["key"], "Control");
    }

    #[tokio::test]
    async fn test_set_ignore_input_events() {
        let mock = MockTransport::new();
        mock.set_default_response(json!({})).await;

        set_ignore_input_events(&mock, true).await.unwrap();

        mock.assert_called("Input.setIgnoreInputEvents", 1).await;
        let p = mock.call_params("Input.setIgnoreInputEvents", 0).await.unwrap();
        assert_eq!(p["ignore"], true);
    }

    #[tokio::test]
    async fn test_synthesize_scroll_gesture() {
        let mock = MockTransport::new();
        mock.set_default_response(json!({})).await;

        synthesize_scroll_gesture(
            &mock,
            100.0,
            200.0,
            None,
            Some(-300.0),
            None,
            None,
            Some(true),
            Some(800),
            None,
            None,
            None,
        )
        .await
        .unwrap();

        mock.assert_called("Input.synthesizeScrollGesture", 1).await;
        let p = mock
            .call_params("Input.synthesizeScrollGesture", 0)
            .await
            .unwrap();
        assert_eq!(p["x"], 100.0);
        assert_eq!(p["y"], 200.0);
        assert_eq!(p["yDistance"], -300.0);
        assert_eq!(p["preventFling"], true);
        assert_eq!(p["speed"], 800);
        // Optional fields not set should be absent
        assert!(p.get("xDistance").is_none());
    }

    #[tokio::test]
    async fn test_modifier_constants() {
        assert_eq!(MODIFIER_ALT, 1);
        assert_eq!(MODIFIER_CTRL, 2);
        assert_eq!(MODIFIER_META, 4);
        assert_eq!(MODIFIER_SHIFT, 8);
        // Composable
        assert_eq!(MODIFIER_CTRL | MODIFIER_SHIFT, 10);
    }
}
