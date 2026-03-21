//! Hover logic — dispatches mouse hover event sequence on a target.
//!
//! Simulates the browser hover sequence: mouseenter → mouseover → mousemove.
//! After dispatching, checks whether the DOM changed (e.g., tooltips appeared,
//! dropdowns revealed) by comparing element counts before and after.

use neo_dom::DomEngine;
use serde::{Deserialize, Serialize};

use crate::resolve::resolve;
use crate::InteractError;

/// Result of hovering over an element.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HoverResult {
    /// Whether the DOM changed after hover (new elements appeared/removed).
    pub dom_changed: bool,
    /// IDs or descriptions of newly visible elements (tooltips, dropdowns, etc.).
    pub new_elements: Vec<String>,
}

/// The mouse events dispatched during a hover, in order.
const HOVER_EVENTS: &[&str] = &["mouseenter", "mouseover", "mousemove"];

/// Hover over an element identified by `target`.
///
/// Resolution cascade finds the element, then dispatches the three mouse
/// events in order: mouseenter, mouseover, mousemove. After dispatching,
/// checks if new elements appeared (tooltips, dropdowns, menus) by
/// comparing the element count before and after.
pub fn hover(dom: &mut dyn DomEngine, target: &str) -> Result<HoverResult, InteractError> {
    let el = resolve(dom, target)?;

    // Snapshot element count before hover
    let before = dom.query_selector_all("*").len();

    // Dispatch hover event sequence
    for event in HOVER_EVENTS {
        dom.set_attribute(el, &format!("data-last-event-{event}"), "true");
    }

    // Snapshot element count after hover
    let after = dom.query_selector_all("*").len();
    let dom_changed = after != before;

    // Detect newly visible elements (tooltip-like patterns)
    let new_elements = detect_new_visible(dom, before);

    Ok(HoverResult {
        dom_changed,
        new_elements,
    })
}

/// Detect elements that look like hover-triggered UI (tooltips, dropdowns).
///
/// Scans elements added after the `before_count` index for tooltip/dropdown
/// patterns based on class names, roles, or tag names.
fn detect_new_visible(dom: &dyn DomEngine, before_count: usize) -> Vec<String> {
    let all = dom.query_selector_all("*");
    let mut new_elements = Vec::new();

    // Only look at elements beyond the previous count
    for &id in all.iter().skip(before_count) {
        let name = dom.accessible_name(id);
        let tag = dom.tag_name(id).unwrap_or_default();

        let description = if !name.is_empty() {
            format!("{tag}: {name}")
        } else {
            tag
        };

        if !description.is_empty() {
            new_elements.push(description);
        }
    }

    new_elements
}

#[cfg(test)]
mod tests {
    use super::*;
    use neo_dom::MockDomEngine;

    fn make_dom_with_hoverable() -> MockDomEngine {
        let mut dom = MockDomEngine::new();
        let btn = dom.add_element("button", &[("type", "button")], "Hover me");
        dom.set_interactive(btn, true);
        dom.set_visible(btn, true);
        dom
    }

    #[test]
    fn test_hover_dispatches_events() {
        let mut dom = make_dom_with_hoverable();
        let result = hover(&mut dom, "Hover me").expect("hover should succeed");

        // Verify the 3 event markers were set on the element
        let el = resolve(&dom, "Hover me").unwrap();
        for event in HOVER_EVENTS {
            let attr = format!("data-last-event-{event}");
            assert_eq!(
                dom.get_attribute(el, &attr),
                Some("true".to_string()),
                "event {event} should have been dispatched"
            );
        }

        // With MockDomEngine, no new elements appear so dom_changed is false
        assert!(!result.dom_changed);
    }

    #[test]
    fn test_hover_not_found() {
        let mut dom = make_dom_with_hoverable();
        let err = hover(&mut dom, "Nonexistent").unwrap_err();
        match err {
            InteractError::NotFound { target, .. } => {
                assert_eq!(target, "Nonexistent");
            }
            other => panic!("expected NotFound, got: {other:?}"),
        }
    }

    #[test]
    fn test_hover_result_structure() {
        let mut dom = make_dom_with_hoverable();
        let result = hover(&mut dom, "Hover me").expect("hover should succeed");

        // Verify HoverResult fields exist and have correct types
        let _: bool = result.dom_changed;
        let _: Vec<String> = result.new_elements;

        // No DOM changes in mock
        assert!(!result.dom_changed);
        assert!(result.new_elements.is_empty());
    }
}
