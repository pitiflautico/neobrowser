//! Target resolution — translates human-readable targets into `ElementId`.
//!
//! Resolution cascade: CSS selector -> text match -> aria-label ->
//! placeholder -> test-id. On failure, suggests alternatives.

use neo_dom::{DomEngine, ElementId};

use crate::InteractError;

/// Which strategy resolved the target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveStrategy {
    CssSelector,
    TextMatch,
    AriaLabel,
    Placeholder,
    TestId,
}

/// Resolve a human-readable target to an `ElementId`.
///
/// Tries strategies in order: CSS selector, text content, aria-label,
/// placeholder attribute, data-testid. Returns the first match.
pub fn resolve(dom: &dyn DomEngine, target: &str) -> Result<ElementId, InteractError> {
    // 1. CSS selector (starts with tag, #, or .)
    if looks_like_selector(target) {
        if let Some(id) = dom.query_selector(target) {
            return Ok(id);
        }
    }

    // 2. Text match
    if let Some(id) = dom.query_by_text(target) {
        return Ok(id);
    }

    // 3. Aria-label: scan all interactive elements
    if let Some(id) = find_by_attribute(dom, "aria-label", target) {
        return Ok(id);
    }

    // 4. Placeholder
    if let Some(id) = find_by_attribute(dom, "placeholder", target) {
        return Ok(id);
    }

    // 5. data-testid
    if let Some(id) = find_by_attribute(dom, "data-testid", target) {
        return Ok(id);
    }

    // Not found — gather suggestions
    let suggestions = gather_suggestions(dom, target);
    Err(InteractError::NotFound {
        target: target.to_string(),
        suggestions,
    })
}

/// Check if a string looks like a CSS selector (vs. plain text).
fn looks_like_selector(target: &str) -> bool {
    target.starts_with('#')
        || target.starts_with('.')
        || target.contains('[')
        || target.contains('>')
        || target.contains('+')
        || target.contains('~')
        || is_html_tag(target.split_whitespace().next().unwrap_or(""))
}

/// Rough check for common HTML tags.
fn is_html_tag(s: &str) -> bool {
    matches!(
        s,
        "a" | "button"
            | "input"
            | "select"
            | "textarea"
            | "form"
            | "div"
            | "span"
            | "p"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "li"
            | "ul"
            | "ol"
            | "table"
            | "tr"
            | "td"
            | "th"
            | "img"
            | "label"
            | "nav"
            | "section"
            | "header"
            | "footer"
            | "main"
    )
}

/// Find element by a specific attribute value (case-insensitive).
fn find_by_attribute(dom: &dyn DomEngine, attr_name: &str, value: &str) -> Option<ElementId> {
    let lower = value.to_lowercase();
    // Search buttons and inputs — most common interactive elements
    let candidates: Vec<ElementId> = dom
        .get_buttons()
        .into_iter()
        .chain(dom.get_inputs())
        .collect();

    for id in candidates {
        if let Some(attr_val) = dom.get_attribute(id, attr_name) {
            if attr_val.to_lowercase().contains(&lower) {
                return Some(id);
            }
        }
    }
    None
}

/// Gather suggestions for stale/missing targets.
fn gather_suggestions(dom: &dyn DomEngine, _target: &str) -> Vec<String> {
    let mut suggestions = Vec::new();
    for id in dom.get_buttons() {
        let name = dom.accessible_name(id);
        if !name.is_empty() {
            suggestions.push(name);
        }
    }
    for id in dom.get_inputs() {
        let name = dom.accessible_name(id);
        if !name.is_empty() {
            suggestions.push(name);
        }
    }
    suggestions.truncate(5);
    suggestions
}
