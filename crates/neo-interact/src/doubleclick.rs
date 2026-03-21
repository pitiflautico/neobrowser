//! Double-click logic — dispatches the full browser event sequence.
//!
//! A double-click in a real browser fires seven events in order:
//! `mousedown`, `mouseup`, `click` (detail=1), `mousedown`, `mouseup`,
//! `click` (detail=2), `dblclick`. This module replicates that sequence
//! via DOM attribute markers and determines the resulting effect.

use neo_dom::DomEngine;

use crate::resolve::resolve;
use crate::{ClickResult, InteractError};

/// The seven mouse events a browser dispatches for a double-click, in order.
const DBLCLICK_EVENTS: &[&str] = &[
    "mousedown",
    "mouseup",
    "click",
    "mousedown",
    "mouseup",
    "click",
    "dblclick",
];

/// Double-click an element identified by `target`.
///
/// Resolves the target using the resolution cascade, validates visibility
/// and interactivity (with one stale-recovery retry), then dispatches the
/// full seven-event browser sequence. Returns the resulting effect:
///
/// - `<a href="...">` produces `Navigation(url)`
/// - other interactive elements produce `DomChanged(mutation_count)`
/// - non-interactive elements after retry produce `NotInteractive` error
pub fn doubleclick(dom: &mut dyn DomEngine, target: &str) -> Result<ClickResult, InteractError> {
    let mut el = resolve(dom, target)?;

    // Stale recovery: if element is gone or non-interactive, re-resolve once
    if !dom.is_visible(el) || !dom.is_interactive(el) {
        let el2 = resolve(dom, target)?;
        if !dom.is_interactive(el2) {
            return Err(InteractError::NotInteractive(target.to_string()));
        }
        el = el2;
    }

    // Dispatch the full 7-event sequence via attribute markers.
    // Each event is recorded as a comma-separated list so tests can verify
    // both the events and their order.
    let mut dispatched = Vec::with_capacity(DBLCLICK_EVENTS.len());
    for event in DBLCLICK_EVENTS {
        dispatched.push(*event);
        dom.set_attribute(el, "data-last-event", event);
    }
    dom.set_attribute(el, "data-events", &dispatched.join(","));
    dom.set_attribute(el, "data-dblclick", "true");

    // Determine the effect based on element type (same logic as click).
    let tag = dom.tag_name(el).unwrap_or_default();
    match tag.as_str() {
        "a" => {
            let href = dom
                .get_attribute(el, "href")
                .unwrap_or_else(|| "#".to_string());
            Ok(ClickResult::Navigation(href))
        }
        _ => {
            // Count mutations: 7 events + data-events + data-dblclick = 9
            let mutation_count = DBLCLICK_EVENTS.len() + 2;
            Ok(ClickResult::DomChanged(mutation_count))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use neo_dom::MockDomEngine;

    fn make_dom_with_interactive_button() -> MockDomEngine {
        let mut dom = MockDomEngine::new();
        let btn = dom.add_element("button", &[("type", "button")], "Open menu");
        dom.set_visible(btn, true);
        dom.set_interactive(btn, true);
        dom
    }

    #[test]
    fn test_doubleclick_dispatches_events() {
        let mut dom = make_dom_with_interactive_button();

        let result = doubleclick(&mut dom, "Open menu").expect("should doubleclick");

        // Verify all 7 events were dispatched in order
        let el = resolve(&dom, "Open menu").unwrap();
        let events = dom.get_attribute(el, "data-events").unwrap();
        assert_eq!(
            events,
            "mousedown,mouseup,click,mousedown,mouseup,click,dblclick"
        );

        // Last event should be dblclick
        let last = dom.get_attribute(el, "data-last-event").unwrap();
        assert_eq!(last, "dblclick");

        // dblclick marker set
        let marker = dom.get_attribute(el, "data-dblclick").unwrap();
        assert_eq!(marker, "true");

        // Result should be DomChanged with 9 mutations (7 events + 2 attrs)
        assert_eq!(result, ClickResult::DomChanged(9));
    }

    #[test]
    fn test_doubleclick_not_found() {
        let mut dom = MockDomEngine::new();
        dom.add_element("div", &[], "Something else");

        let err = doubleclick(&mut dom, "Nonexistent").unwrap_err();
        match err {
            InteractError::NotFound { target, .. } => {
                assert_eq!(target, "Nonexistent");
            }
            other => panic!("expected NotFound, got: {other:?}"),
        }
    }

    #[test]
    fn test_doubleclick_returns_dom_changed() {
        let mut dom = make_dom_with_interactive_button();

        let result = doubleclick(&mut dom, "Open menu").expect("should doubleclick");
        match result {
            ClickResult::DomChanged(count) => {
                assert!(count > 0, "mutation count should be positive");
            }
            other => panic!("expected DomChanged, got: {other:?}"),
        }
    }

    #[test]
    fn test_doubleclick_link_returns_navigation() {
        let mut dom = MockDomEngine::new();
        let link = dom.add_element("a", &[("href", "https://example.com")], "Details");
        dom.set_visible(link, true);
        dom.set_interactive(link, true);

        let result = doubleclick(&mut dom, "Details").expect("should doubleclick link");
        assert_eq!(
            result,
            ClickResult::Navigation("https://example.com".into())
        );
    }

    #[test]
    fn test_doubleclick_not_interactive() {
        let mut dom = MockDomEngine::new();
        let el = dom.add_element("div", &[], "Static text");
        dom.set_visible(el, true);
        dom.set_interactive(el, false);

        let err = doubleclick(&mut dom, "Static text").unwrap_err();
        assert!(matches!(err, InteractError::NotInteractive(_)));
    }
}
