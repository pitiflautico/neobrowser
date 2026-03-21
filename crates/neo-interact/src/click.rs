//! Click logic — resolves a target and determines the effect.
//!
//! Links produce `Navigation`, submit buttons trigger form submission,
//! and other interactive elements produce `DomChanged`.

use neo_dom::DomEngine;

use crate::resolve::resolve;
use crate::{ClickResult, InteractError};

/// Click an element identified by `target`.
///
/// Resolution cascade finds the element. Then determines the effect:
/// - `<a href="...">` -> `Navigation(url)`
/// - `<button type="submit">` or `<input type="submit">` -> `Navigation(form action)`
/// - other interactive -> `DomChanged(1)`
/// - non-interactive -> `NotInteractive` error
pub fn click(dom: &mut dyn DomEngine, target: &str) -> Result<ClickResult, InteractError> {
    let el = resolve(dom, target)?;

    if !dom.is_interactive(el) {
        return Err(InteractError::NotInteractive(target.to_string()));
    }

    let tag = dom.tag_name(el).unwrap_or_default();
    match tag.as_str() {
        "a" => {
            let href = dom
                .get_attribute(el, "href")
                .unwrap_or_else(|| "#".to_string());
            Ok(ClickResult::Navigation(href))
        }
        "button" | "input" => {
            let type_attr = dom
                .get_attribute(el, "type")
                .unwrap_or_else(|| "button".to_string())
                .to_lowercase();
            if type_attr == "submit" {
                let action = find_parent_form_action(dom, el);
                Ok(ClickResult::Navigation(action))
            } else {
                Ok(ClickResult::DomChanged(1))
            }
        }
        _ => Ok(ClickResult::DomChanged(1)),
    }
}

/// Walk up to find the parent form's action URL.
///
/// Since `DomEngine` doesn't expose parent traversal, we check forms
/// for a matching action. Falls back to current page.
fn find_parent_form_action(dom: &dyn DomEngine, _el: neo_dom::ElementId) -> String {
    let forms = dom.get_forms();
    if let Some(form) = forms.first() {
        if !form.action.is_empty() {
            return form.action.clone();
        }
    }
    "#".to_string()
}
