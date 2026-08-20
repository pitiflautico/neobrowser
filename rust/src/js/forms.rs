//! Snippets that put data into a page, and find the control to press.
//!
//! The counterpart of [`crate::ops::forms`] and [`crate::ops::target`]. Every one of these
//! goes through the element's own prototype setter and dispatches `input`/`change`, because
//! assigning `.value` updates the DOM and leaves a React or Vue component's state stale —
//! the field looks filled and the form submits empty.

use super::Snippet;

/// Set one field's value the way a framework will notice (`fill`).
pub fn fill_control() -> Snippet {
    Snippet::new(include_str!("../../js/fill_control.js"))
}

/// Match one form field by label/name/placeholder/aria and fill it (`form_fill`).
pub fn form_fill_fields() -> Snippet {
    Snippet::new(include_str!("../../js/form_fill_fields.js"))
}

/// Click a form's own submit control, reporting which one it found (`submit`).
pub fn submit_form() -> Snippet {
    Snippet::new(include_str!("../../js/submit_form.js"))
}

/// Pick the visible clickable that best matches an intent, and stash it for the real
/// mouse-click path (`find_and_click`).
pub fn find_and_click() -> Snippet {
    Snippet::new(include_str!("../../js/find_and_click.js"))
}
