//! Visibility and interactivity heuristics.
//!
//! No layout computation — uses attributes and inline styles only.

use markup5ever_rcdom::Handle;

use crate::html5ever_dom::ElementInfo;
use crate::query::collect_text_content;

/// Check if an element is heuristically visible.
///
/// Returns false if: hidden attr, aria-hidden="true",
/// style contains display:none or visibility:hidden.
pub(crate) fn is_visible(info: &ElementInfo) -> bool {
    // Check hidden attribute
    if info.attrs.iter().any(|(k, _)| k == "hidden") {
        return false;
    }
    // Check aria-hidden
    if info
        .attrs
        .iter()
        .any(|(k, v)| k == "aria-hidden" && v == "true")
    {
        return false;
    }
    // Check inline style
    if let Some((_, style)) = info.attrs.iter().find(|(k, _)| k == "style") {
        let s = style.to_lowercase().replace(' ', "");
        if s.contains("display:none") || s.contains("visibility:hidden") {
            return false;
        }
    }
    // Check type="hidden" for inputs
    if info.tag == "input" && info.attrs.iter().any(|(k, v)| k == "type" && v == "hidden") {
        return false;
    }
    true
}

/// Check if an element is interactive.
///
/// Interactive elements: input, button, a, select, textarea,
/// elements with contenteditable, elements with role=button.
pub(crate) fn is_interactive(info: &ElementInfo) -> bool {
    let interactive_tags = [
        "input", "button", "a", "select", "textarea", "details", "summary",
    ];
    if interactive_tags.contains(&info.tag.as_str()) {
        return true;
    }
    // Check contenteditable
    if info
        .attrs
        .iter()
        .any(|(k, v)| k == "contenteditable" && v != "false")
    {
        return true;
    }
    // Check role=button or role=link
    if info
        .attrs
        .iter()
        .any(|(k, v)| k == "role" && (v == "button" || v == "link" || v == "tab"))
    {
        return true;
    }
    // Check tabindex (makes anything focusable)
    if info.attrs.iter().any(|(k, _)| k == "tabindex") {
        return true;
    }
    false
}

/// Compute accessible name for an element.
///
/// Priority: aria-label > aria-labelledby (skip) > label[for] >
/// placeholder > title > text content.
pub(crate) fn accessible_name_from(
    info: &ElementInfo,
    handle: &Handle,
    all_elements: &[ElementInfo],
    all_handles: &[Handle],
) -> String {
    // 1. aria-label
    if let Some((_, v)) = info.attrs.iter().find(|(k, _)| k == "aria-label") {
        if !v.is_empty() {
            return v.clone();
        }
    }

    // 2. label[for=id] — find a label element referencing this id
    if let Some((_, el_id)) = info.attrs.iter().find(|(k, _)| k == "id") {
        let label_text = find_label_for(el_id, all_elements, all_handles);
        if !label_text.is_empty() {
            return label_text;
        }
    }

    // 3. placeholder
    if let Some((_, v)) = info.attrs.iter().find(|(k, _)| k == "placeholder") {
        if !v.is_empty() {
            return v.clone();
        }
    }

    // 4. title attribute
    if let Some((_, v)) = info.attrs.iter().find(|(k, _)| k == "title") {
        if !v.is_empty() {
            return v.clone();
        }
    }

    // 5. alt (for images)
    if info.tag == "img" {
        if let Some((_, v)) = info.attrs.iter().find(|(k, _)| k == "alt") {
            if !v.is_empty() {
                return v.clone();
            }
        }
    }

    // 6. Text content
    let text = collect_text_content(handle).trim().to_string();
    text
}

/// Find label element with for=id and return its text content.
fn find_label_for(target_id: &str, elements: &[ElementInfo], handles: &[Handle]) -> String {
    for (i, info) in elements.iter().enumerate() {
        if info.tag == "label" {
            if let Some((_, v)) = info.attrs.iter().find(|(k, _)| k == "for") {
                if v == target_id {
                    return collect_text_content(&handles[i]).trim().to_string();
                }
            }
        }
    }
    String::new()
}
