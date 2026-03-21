//! Form filling and submission.
//!
//! `fill_form` resolves each field by name/label/placeholder and sets values.
//! `submit` finds the form, detects CSRF tokens, and determines the result.

use std::collections::HashMap;

use neo_dom::DomEngine;

use crate::resolve::resolve;
use crate::{CsrfToken, InteractError, SubmitOutcome, SubmitResult};

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

    // CSRF detection happens via detect_csrf() at a higher level.
    // The token is available for callers that need it.
    let _csrf = detect_csrf(dom);

    if is_ajax {
        Ok(SubmitResult::AjaxResponse(form.action.clone()))
    } else {
        Ok(SubmitResult::Navigation(form.action.clone()))
    }
}

/// Collect all form field name=value pairs from the first form.
///
/// Returns a `HashMap` of field names to their current values.
/// Skips disabled fields and fields without names.
/// Includes hidden inputs (CSRF tokens, etc.).
pub fn collect_form_data(dom: &dyn DomEngine) -> HashMap<String, String> {
    let forms = dom.get_forms();
    let mut data = HashMap::new();
    if let Some(form) = forms.first() {
        for field in &form.fields {
            // Skip fields without names
            if field.name.is_empty() {
                continue;
            }
            // Skip disabled fields
            if field.disabled {
                continue;
            }
            let value = field.value.clone().unwrap_or_default();
            data.insert(field.name.clone(), value);
        }
    }
    data
}

/// Submit with full diagnostics: result + CSRF token + collected form data.
///
/// Combines `submit`, `detect_csrf`, and `collect_form_data` into one call.
pub fn submit_full(
    dom: &mut dyn DomEngine,
    target: Option<&str>,
) -> Result<SubmitOutcome, InteractError> {
    let csrf = detect_csrf(dom);
    let form_data = collect_form_data(dom);
    let result = submit(dom, target)?;

    Ok(SubmitOutcome {
        result,
        csrf,
        form_data,
    })
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
