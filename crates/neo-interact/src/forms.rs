//! Form filling and submission.
//!
//! `fill_form` resolves each field by name/label/placeholder and sets values.
//! `submit` finds the form, detects CSRF tokens, and determines the result.

use std::collections::HashMap;

use neo_dom::DomEngine;

use crate::resolve::resolve;
use crate::{CsrfToken, InteractError, SubmitResult};

/// Fill multiple form fields at once.
///
/// Keys are field identifiers (name, label, placeholder). Values are
/// the text to set. Each field is resolved and typed independently.
pub fn fill_form(
    dom: &mut dyn DomEngine,
    fields: &HashMap<String, String>,
) -> Result<(), InteractError> {
    for (field_target, value) in fields {
        crate::type_text::type_text(dom, field_target, value, true)?;
    }
    Ok(())
}

/// Submit the form containing the target element.
///
/// If `target` is `None`, submits the first form on the page.
/// Detects CSRF tokens in hidden inputs and determines the result
/// based on form method and action.
pub fn submit(
    dom: &mut dyn DomEngine,
    target: Option<&str>,
) -> Result<SubmitResult, InteractError> {
    // If target given, resolve it to validate it exists
    if let Some(t) = target {
        let _ = resolve(dom, t)?;
    }

    let forms = dom.get_forms();
    let form = match forms.first() {
        Some(f) => f,
        None => return Ok(SubmitResult::NoAction),
    };

    if form.action.is_empty() {
        return Ok(SubmitResult::NoAction);
    }

    // Check for AJAX indicators (no navigation expected)
    let is_ajax = form.action.starts_with("javascript:")
        || form.action.starts_with("api/")
        || form.action.starts_with("/api/");

    if is_ajax {
        Ok(SubmitResult::AjaxResponse(form.action.clone()))
    } else {
        Ok(SubmitResult::Navigation(form.action.clone()))
    }
}

/// Scan a form for CSRF tokens in hidden inputs.
///
/// Looks for hidden fields with names matching common CSRF patterns:
/// `_token`, `csrf_token`, `csrfmiddlewaretoken`, `authenticity_token`,
/// `__RequestVerificationToken`.
pub fn detect_csrf(dom: &dyn DomEngine) -> Option<CsrfToken> {
    let csrf_patterns = [
        "_token",
        "csrf_token",
        "csrf",
        "csrfmiddlewaretoken",
        "authenticity_token",
        "__requestverificationtoken",
    ];

    let forms = dom.get_forms();
    for form in &forms {
        for field in &form.fields {
            if field.field_type != "hidden" {
                continue;
            }
            let name_lower = field.name.to_lowercase();
            for pattern in &csrf_patterns {
                if name_lower.contains(pattern) {
                    if let Some(ref value) = field.value {
                        return Some(CsrfToken {
                            name: field.name.clone(),
                            value: value.clone(),
                        });
                    }
                }
            }
        }
    }
    None
}
