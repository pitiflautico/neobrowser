//! Select/option handling for `<select>` dropdowns.
//!
//! Resolves a target to a `<select>` element, finds the matching `<option>`
//! by value or display text, sets the `selected` attribute, and dispatches
//! a `change` event via attribute mutation.

use neo_dom::DomEngine;

use crate::resolve::resolve;
use crate::InteractError;

/// Select an option in a `<select>` dropdown.
///
/// `target` identifies the `<select>` element (CSS selector, text, aria-label, etc.).
/// `value` matches against option `value` attributes first, then display text.
pub fn select(dom: &mut dyn DomEngine, target: &str, value: &str) -> Result<(), InteractError> {
    let el = resolve(dom, target)?;

    // Verify it's a <select>
    let tag = dom.tag_name(el).unwrap_or_default();
    if tag != "select" {
        return Err(InteractError::TypeMismatch {
            expected: "select".to_string(),
            actual: tag,
        });
    }

    // Find matching <option> — first by value attribute, then by text
    let options = dom.query_selector_all("option");
    let mut matched = false;

    // Pass 1: match by value attribute
    for &opt_id in &options {
        if let Some(opt_val) = dom.get_attribute(opt_id, "value") {
            if opt_val == value {
                dom.set_attribute(el, "value", value);
                // Mark this option as selected
                dom.set_attribute(opt_id, "selected", "selected");
                matched = true;
                break;
            }
        }
    }

    // Pass 2: match by display text
    if !matched {
        let lower_value = value.to_lowercase();
        for &opt_id in &options {
            let text = dom.text_content(opt_id);
            if text.trim().to_lowercase() == lower_value {
                let opt_val = dom
                    .get_attribute(opt_id, "value")
                    .unwrap_or_else(|| text.trim().to_string());
                dom.set_attribute(el, "value", &opt_val);
                dom.set_attribute(opt_id, "selected", "selected");
                matched = true;
                break;
            }
        }
    }

    if !matched {
        return Err(InteractError::NotFound {
            target: format!("option '{value}' in select '{target}'"),
            suggestions: options
                .iter()
                .filter_map(|&id| {
                    let text = dom.text_content(id);
                    if text.trim().is_empty() {
                        None
                    } else {
                        Some(text.trim().to_string())
                    }
                })
                .take(5)
                .collect(),
        });
    }

    // Dispatch change event by setting a synthetic attribute
    dom.set_attribute(el, "data-changed", "true");

    Ok(())
}
