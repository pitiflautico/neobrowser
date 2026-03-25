//! Unit tests for navigation types and serialization.
//!
//! These tests validate the data structures and enums used by the
//! navigation module without requiring a live Chrome instance.

use neo_chrome::navigation::{NavigationType, PageInfo};

#[test]
fn page_info_serialize_roundtrip() {
    let info = PageInfo {
        id: "ABC123".to_string(),
        title: "Test Page".to_string(),
        url: "https://example.com".to_string(),
        page_type: "page".to_string(),
    };

    let json = serde_json::to_string(&info).unwrap();
    let deser: PageInfo = serde_json::from_str(&json).unwrap();

    assert_eq!(deser.id, "ABC123");
    assert_eq!(deser.title, "Test Page");
    assert_eq!(deser.url, "https://example.com");
    assert_eq!(deser.page_type, "page");
}

#[test]
fn page_info_deserialize_from_chrome_json() {
    let chrome_json = r#"{
        "id": "DEADBEEF",
        "title": "Google",
        "url": "https://www.google.com/",
        "page_type": "page"
    }"#;

    let info: PageInfo = serde_json::from_str(chrome_json).unwrap();
    assert_eq!(info.id, "DEADBEEF");
    assert_eq!(info.title, "Google");
    assert_eq!(info.url, "https://www.google.com/");
}

#[test]
fn page_info_debug_impl() {
    let info = PageInfo {
        id: "X".to_string(),
        title: "T".to_string(),
        url: "u".to_string(),
        page_type: "page".to_string(),
    };
    let dbg = format!("{info:?}");
    assert!(dbg.contains("PageInfo"));
    assert!(dbg.contains("X"));
}

#[test]
fn page_info_clone() {
    let info = PageInfo {
        id: "1".to_string(),
        title: "Clone Test".to_string(),
        url: "https://clone.test".to_string(),
        page_type: "page".to_string(),
    };
    let cloned = info.clone();
    assert_eq!(cloned.id, info.id);
    assert_eq!(cloned.title, info.title);
}

#[test]
fn navigation_type_equality() {
    assert_eq!(NavigationType::Url, NavigationType::Url);
    assert_eq!(NavigationType::Back, NavigationType::Back);
    assert_eq!(NavigationType::Forward, NavigationType::Forward);
    assert_eq!(NavigationType::Reload, NavigationType::Reload);
    assert_ne!(NavigationType::Url, NavigationType::Back);
}

#[test]
fn navigation_type_debug() {
    assert_eq!(format!("{:?}", NavigationType::Url), "Url");
    assert_eq!(format!("{:?}", NavigationType::Back), "Back");
    assert_eq!(format!("{:?}", NavigationType::Forward), "Forward");
    assert_eq!(format!("{:?}", NavigationType::Reload), "Reload");
}

#[test]
fn navigation_type_copy() {
    let nav = NavigationType::Reload;
    let copied = nav; // Copy
    assert_eq!(nav, copied); // original still usable
}

#[test]
fn page_info_empty_fields() {
    let info = PageInfo {
        id: String::new(),
        title: String::new(),
        url: String::new(),
        page_type: String::new(),
    };
    let json = serde_json::to_string(&info).unwrap();
    let deser: PageInfo = serde_json::from_str(&json).unwrap();
    assert!(deser.id.is_empty());
}
