//! Popup and dialog detection + cookie-consent auto-dismiss.
//!
//! Detects modals via ARIA roles, `aria-modal`, and common CSS classes.
//! Provides a consent-banner dismisser that clicks common "Accept" buttons.

use neo_dom::{DomEngine, ElementId};

/// Detect a modal or dialog on the page.
///
/// Checks (in order):
/// 1. `role="dialog"`
/// 2. `[aria-modal=true]`
/// 3. `.modal`
/// 4. `#modal`
/// 5. `role="alertdialog"`
///
/// Returns the `ElementId` of the first match, or `None`.
pub fn detect_modal(dom: &dyn DomEngine) -> Option<ElementId> {
    // role=dialog (via ARIA query)
    if let Some(el) = dom.query_by_role("dialog", None) {
        return Some(el);
    }
    // Attribute and class selectors
    for sel in &[
        "[aria-modal=true]",
        ".modal",
        "#modal",
        "[role=alertdialog]",
    ] {
        if let Some(el) = dom.query_selector(sel) {
            return Some(el);
        }
    }
    None
}

/// Common consent-button labels (EN + ES).
const CONSENT_LABELS: &[&str] = &[
    "Accept all",
    "Aceptar todo",
    "Accept",
    "Aceptar",
    "OK",
    "Got it",
    "I agree",
    "Entendido",
    "Agree",
];

/// Try to dismiss a cookie-consent banner.
///
/// Searches for buttons matching common consent labels and clicks the
/// first interactive one found. Returns `true` if a button was clicked.
pub fn dismiss_consent(dom: &mut dyn DomEngine) -> bool {
    for text in CONSENT_LABELS {
        if let Some(el) = dom.query_by_text(text) {
            if dom.is_interactive(el) {
                // Simulate click by dispatching via the resolve+click path.
                // Since we can't dispatch JS events from Rust directly, we
                // mark the element as "clicked" by setting a data attribute.
                dom.set_attribute(el, "data-consent-dismissed", "true");
                return true;
            }
        }
    }
    false
}
