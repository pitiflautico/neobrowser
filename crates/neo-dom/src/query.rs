//! Selector resolution — CSS selectors, text match, role match.
//!
//! All queries operate on the flat element list built during parsing.

use markup5ever_rcdom::{Handle, NodeData};

use crate::html5ever_dom::ElementInfo;

/// Match a CSS selector against an element.
///
/// Supports: tag, .class, #id, [attr], [attr=val], tag.class,
/// tag#id, and combinations. Not a full CSS selector engine.
pub(crate) fn matches_selector(info: &ElementInfo, selector: &str) -> bool {
    let selector = selector.trim();
    if selector.is_empty() {
        return false;
    }
    // Split compound selectors on commas
    if selector.contains(',') {
        return selector
            .split(',')
            .any(|s| matches_selector(info, s.trim()));
    }
    // Split descendant selectors on space — only match last segment
    // (we don't track ancestry, so descendant combinator is approximate)
    let parts: Vec<&str> = selector.split_whitespace().collect();
    let last = parts.last().copied().unwrap_or(selector);
    matches_simple_selector(info, last)
}

/// Match a single simple selector (no combinators).
fn matches_simple_selector(info: &ElementInfo, sel: &str) -> bool {
    let sel = sel.trim();

    // [attr] or [attr=val]
    if sel.starts_with('[') && sel.ends_with(']') {
        let inner = &sel[1..sel.len() - 1];
        return match_attr_selector(info, inner);
    }

    // #id
    if let Some(id_val) = sel.strip_prefix('#') {
        return info.attrs.iter().any(|(k, v)| k == "id" && v == id_val);
    }

    // .class
    if let Some(cls) = sel.strip_prefix('.') {
        return has_class(info, cls);
    }

    // tag.class or tag#id or tag[attr]
    if let Some(dot_pos) = sel.find('.') {
        let tag = &sel[..dot_pos];
        let cls = &sel[dot_pos + 1..];
        return info.tag == tag && has_class(info, cls);
    }
    if let Some(hash_pos) = sel.find('#') {
        let tag = &sel[..hash_pos];
        let id_val = &sel[hash_pos + 1..];
        let id_match = info.attrs.iter().any(|(k, v)| k == "id" && v == id_val);
        return info.tag == tag && id_match;
    }
    if let Some(bracket_pos) = sel.find('[') {
        let tag = &sel[..bracket_pos];
        if sel.ends_with(']') {
            let inner = &sel[bracket_pos + 1..sel.len() - 1];
            return info.tag == tag && match_attr_selector(info, inner);
        }
    }

    // Plain tag name
    info.tag == sel
}

/// Check if element has a CSS class.
fn has_class(info: &ElementInfo, cls: &str) -> bool {
    info.attrs
        .iter()
        .any(|(k, v)| k == "class" && v.split_whitespace().any(|c| c == cls))
}

/// Match [attr] or [attr=val] or [attr="val"].
fn match_attr_selector(info: &ElementInfo, inner: &str) -> bool {
    if let Some(eq_pos) = inner.find('=') {
        let attr_name = &inner[..eq_pos];
        let attr_val = inner[eq_pos + 1..].trim_matches('"').trim_matches('\'');
        info.attrs
            .iter()
            .any(|(k, v)| k == attr_name && v == attr_val)
    } else {
        info.attrs.iter().any(|(k, _)| k == inner)
    }
}

/// Find the deepest element whose text contains `needle` (case-insensitive).
///
/// Prefers leaf elements over ancestors to return the most specific match.
pub(crate) fn find_by_text(
    elements: &[ElementInfo],
    handles: &[Handle],
    needle: &str,
) -> Option<usize> {
    let lower = needle.to_lowercase();
    let mut best: Option<usize> = None;
    // Iterate in reverse (deeper elements come later in tree walk order)
    // and pick the last matching element — the deepest one.
    for (i, _info) in elements.iter().enumerate() {
        let text = collect_text_content(&handles[i]);
        if text.to_lowercase().contains(&lower) {
            best = Some(i);
        }
    }
    best
}

/// Find first element matching role and optional name.
pub(crate) fn find_by_role(
    elements: &[ElementInfo],
    handles: &[Handle],
    role: &str,
    name: Option<&str>,
) -> Option<usize> {
    let role_lower = role.to_lowercase();
    elements.iter().enumerate().find_map(|(i, info)| {
        let explicit_role = info
            .attrs
            .iter()
            .find(|(k, _)| k == "role")
            .map(|(_, v)| v.to_lowercase());
        let implicit_role = implicit_aria_role(&info.tag);
        let el_role = explicit_role.unwrap_or(implicit_role);

        if el_role != role_lower {
            return None;
        }
        if let Some(n) = name {
            let acc = crate::visibility::accessible_name_from(info, &handles[i], elements, handles);
            if !acc.to_lowercase().contains(&n.to_lowercase()) {
                return None;
            }
        }
        Some(i)
    })
}

/// Get implicit ARIA role from tag name.
fn implicit_aria_role(tag: &str) -> String {
    match tag {
        "button" => "button",
        "a" => "link",
        "input" => "textbox",
        "select" => "listbox",
        "textarea" => "textbox",
        "img" => "img",
        "nav" => "navigation",
        "main" => "main",
        "header" => "banner",
        "footer" => "contentinfo",
        "form" => "form",
        "table" => "table",
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => "heading",
        _ => "generic",
    }
    .to_string()
}

/// Recursively collect text content from a node handle.
pub(crate) fn collect_text_content(handle: &Handle) -> String {
    let mut result = String::new();
    collect_text_recursive(handle, &mut result);
    result
}

/// Recursive helper for text collection.
fn collect_text_recursive(handle: &Handle, buf: &mut String) {
    match &handle.data {
        NodeData::Text { contents } => {
            buf.push_str(&contents.borrow());
        }
        _ => {
            for child in handle.children.borrow().iter() {
                collect_text_recursive(child, buf);
            }
        }
    }
}
