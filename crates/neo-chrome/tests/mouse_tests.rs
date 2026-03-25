//! Unit tests for mouse.rs — coordinate math and event serialization.
//!
//! These tests exercise pure functions without a live Chrome instance.

use neo_chrome::mouse::{center_from_quad, mouse_event_params};
use serde_json::json;

#[test]
fn center_from_quad_simple_rect() {
    // 100x50 rect at (10,20): corners (10,20), (110,20), (110,70), (10,70)
    let quad = vec![
        json!(10.0),
        json!(20.0),
        json!(110.0),
        json!(20.0),
        json!(110.0),
        json!(70.0),
        json!(10.0),
        json!(70.0),
    ];
    let (cx, cy) = center_from_quad(&quad).unwrap();
    assert!((cx - 60.0).abs() < 0.001, "cx={cx}");
    assert!((cy - 45.0).abs() < 0.001, "cy={cy}");
}

#[test]
fn center_from_quad_unit_square() {
    let quad = vec![
        json!(0.0),
        json!(0.0),
        json!(1.0),
        json!(0.0),
        json!(1.0),
        json!(1.0),
        json!(0.0),
        json!(1.0),
    ];
    let (cx, cy) = center_from_quad(&quad).unwrap();
    assert!((cx - 0.5).abs() < 0.001);
    assert!((cy - 0.5).abs() < 0.001);
}

#[test]
fn center_from_quad_wrong_length() {
    let quad = vec![json!(1.0), json!(2.0)];
    let result = center_from_quad(&quad);
    assert!(result.is_err());
}

#[test]
fn center_from_quad_non_numeric() {
    let quad = vec![
        json!("a"),
        json!(0.0),
        json!(1.0),
        json!(0.0),
        json!(1.0),
        json!(1.0),
        json!(0.0),
        json!(1.0),
    ];
    let result = center_from_quad(&quad);
    assert!(result.is_err());
}

#[test]
fn mouse_event_params_single_click() {
    let params = mouse_event_params("mousePressed", 100.0, 200.0, "left", 1);
    assert_eq!(params["type"], "mousePressed");
    assert_eq!(params["x"], 100.0);
    assert_eq!(params["y"], 200.0);
    assert_eq!(params["button"], "left");
    assert_eq!(params["clickCount"], 1);
}

#[test]
fn mouse_event_params_double_click() {
    let params = mouse_event_params("mousePressed", 50.0, 75.0, "left", 2);
    assert_eq!(params["clickCount"], 2);
}

#[test]
fn mouse_event_params_hover() {
    let params = mouse_event_params("mouseMoved", 30.0, 40.0, "none", 0);
    assert_eq!(params["type"], "mouseMoved");
    assert_eq!(params["button"], "none");
    assert_eq!(params["clickCount"], 0);
}
