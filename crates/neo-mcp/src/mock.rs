//! Mock helpers for testing neo-mcp without a real browser.
//!
//! Re-exports [`MockBrowserEngine`] from neo-engine and provides
//! convenience constructors for common test scenarios.

pub use neo_engine::MockBrowserEngine;

use neo_extract::WomDocument;
use neo_extract::WomNode;

/// Create a [`MockBrowserEngine`] with a page already "loaded".
pub fn mock_with_page(url: &str, title: &str) -> MockBrowserEngine {
    let mut engine = MockBrowserEngine::new();
    engine.wom = WomDocument {
        url: url.to_string(),
        title: title.to_string(),
        nodes: vec![
            WomNode {
                id: "w1".into(),
                tag: "a".into(),
                role: "link".into(),
                label: "Home".into(),
                value: None,
                href: Some("/".into()),
                actions: vec!["click".into()],
                visible: true,
                interactive: true,
                input_type: None,
                name: None,
                checked: None,
                selected: None,
                required: false,
                disabled: false,
                readonly: false,
                placeholder: None,
                pattern: None,
                min: None,
                max: None,
                minlength: None,
                maxlength: None,
                autocomplete: None,
                form_id: None,
                options: Vec::new(),
            },
            WomNode {
                id: "w2".into(),
                tag: "input".into(),
                role: "input".into(),
                label: "email".into(),
                value: None,
                href: None,
                actions: vec!["type".into()],
                visible: true,
                interactive: true,
                input_type: None,
                name: None,
                checked: None,
                selected: None,
                required: false,
                disabled: false,
                readonly: false,
                placeholder: None,
                pattern: None,
                min: None,
                max: None,
                minlength: None,
                maxlength: None,
                autocomplete: None,
                form_id: None,
                options: Vec::new(),
            },
        ],
        page_type: "article".into(),
        summary: "2 interactive elements".into(),
    };
    engine
}
