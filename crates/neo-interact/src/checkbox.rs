//! Checkbox and radio button handling.
//!
//! Resolves a target to an `input[type=checkbox]` or `input[type=radio]`,
//! sets or removes the `checked` attribute, and dispatches a `change` event
//! via attribute mutation.

use neo_dom::DomEngine;

use crate::resolve::resolve;
use crate::InteractError;

/// Check or uncheck a checkbox/radio button.
///
/// `target` identifies the input element (CSS selector, text, aria-label, etc.).
/// `checked` sets the desired state — `true` to check, `false` to uncheck.
pub fn check(dom: &mut dyn DomEngine, target: &str, checked: bool) -> Result<(), InteractError> {
    let el = resolve(dom, target)?;

    // Verify it's an input[type=checkbox] or input[type=radio]
    let tag = dom.tag_name(el).unwrap_or_default();
    if tag != "input" {
        return Err(InteractError::TypeMismatch {
            expected: "input[type=checkbox|radio]".to_string(),
            actual: tag,
        });
    }

    let input_type = dom
        .get_attribute(el, "type")
        .unwrap_or_else(|| "text".to_string())
        .to_lowercase();

    if input_type != "checkbox" && input_type != "radio" {
        return Err(InteractError::TypeMismatch {
            expected: "input[type=checkbox|radio]".to_string(),
            actual: format!("input[type={input_type}]"),
        });
    }

    // Set or remove checked attribute
    if checked {
        dom.set_attribute(el, "checked", "checked");
    } else {
        // Set to empty string to indicate unchecked (DOM engines should interpret this)
        dom.set_attribute(el, "checked", "");
    }

    // Dispatch change event via synthetic attribute
    dom.set_attribute(el, "data-changed", "true");

    Ok(())
}
