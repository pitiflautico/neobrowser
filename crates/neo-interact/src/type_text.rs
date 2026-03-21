//! Type text into an input, textarea, or contenteditable element.
//!
//! Validates the target is a typeable element before mutating.
//! `type_slowly` types character by character for debounce-sensitive sites.

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

    let is_contenteditable = dom.get_attribute(el, "contenteditable").is_some();
    let is_typeable = matches!(tag.as_str(), "input" | "textarea") || is_contenteditable;

    if !is_typeable {
        return Err(InteractError::TypeMismatch {
            expected: "input, textarea, or contenteditable".to_string(),
            actual: tag,
        });
    }

    if is_contenteditable && !matches!(tag.as_str(), "input" | "textarea") {
        // contenteditable element — use text_content
        let current = if clear {
            String::new()
        } else {
            dom.text_content(el)
        };
        let new_value = format!("{current}{text}");
        dom.set_text_content(el, &new_value);
    } else {
        // input or textarea — use value attribute
        let current = if clear {
            String::new()
        } else {
            dom.get_attribute(el, "value").unwrap_or_default()
        };
        let new_value = format!("{current}{text}");
        dom.set_attribute(el, "value", &new_value);
    }

    Ok(())
}

/// Type text character by character, triggering input events per char.
///
/// Useful for sites with debounce or autocomplete that need incremental
/// input events. Each character appends to the current value.
/// The `_delay_ms` parameter is accepted for API compatibility but
/// has no effect in the DOM-only layer (no real timing).
pub fn type_slowly(
    dom: &mut dyn DomEngine,
    target: &str,
    text: &str,
    _delay_ms: u64,
) -> Result<usize, InteractError> {
    let el = resolve(dom, target)?;
    let tag = dom.tag_name(el).unwrap_or_default();

    let is_contenteditable = dom.get_attribute(el, "contenteditable").is_some();
    let is_typeable = matches!(tag.as_str(), "input" | "textarea") || is_contenteditable;

    if !is_typeable {
        return Err(InteractError::TypeMismatch {
            expected: "input, textarea, or contenteditable".to_string(),
            actual: tag,
        });
    }

    let use_text_content = is_contenteditable && !matches!(tag.as_str(), "input" | "textarea");
    let mut char_count = 0;

    for ch in text.chars() {
        if use_text_content {
            let current = dom.text_content(el);
            dom.set_text_content(el, &format!("{current}{ch}"));
        } else {
            let current = dom.get_attribute(el, "value").unwrap_or_default();
            dom.set_attribute(el, "value", &format!("{current}{ch}"));
        }
        char_count += 1;
        // In a real runtime, dispatch_event("input", true) would fire here
        // and we'd sleep(delay_ms). The DOM layer just accumulates.
    }

    Ok(char_count)
}
