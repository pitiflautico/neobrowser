//! Type text into an input, textarea, or contenteditable element.
//!
//! Validates the target is a typeable element before mutating.

use neo_dom::DomEngine;

use crate::resolve::resolve;
use crate::InteractError;

/// Type `text` into the element identified by `target`.
///
/// If `clear` is true, replaces existing content. Otherwise appends.
/// Only works on `input`, `textarea`, or elements with `contenteditable`.
pub fn type_text(
    dom: &mut dyn DomEngine,
    target: &str,
    text: &str,
    clear: bool,
) -> Result<(), InteractError> {
    let el = resolve(dom, target)?;
    let tag = dom.tag_name(el).unwrap_or_default();

    let is_typeable = matches!(tag.as_str(), "input" | "textarea")
        || dom.get_attribute(el, "contenteditable").is_some();

    if !is_typeable {
        return Err(InteractError::TypeMismatch {
            expected: "input, textarea, or contenteditable".to_string(),
            actual: tag,
        });
    }

    if matches!(tag.as_str(), "input" | "textarea") {
        let current = if clear {
            String::new()
        } else {
            dom.get_attribute(el, "value").unwrap_or_default()
        };
        let new_value = format!("{current}{text}");
        dom.set_attribute(el, "value", &new_value);
    } else {
        // contenteditable
        let current = if clear {
            String::new()
        } else {
            dom.text_content(el)
        };
        let new_value = format!("{current}{text}");
        dom.set_text_content(el, &new_value);
    }

    Ok(())
}
