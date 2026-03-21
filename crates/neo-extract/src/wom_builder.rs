//! WOM builder — constructs WomNodes from DOM elements.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use neo_dom::{DomEngine, ElementId};

use crate::wom::WomNode;

/// Landmark tag -> ARIA role mapping.
pub(crate) const LANDMARK_TAGS: &[(&str, &str)] = &[
    ("nav", "navigation"),
    ("header", "banner"),
    ("footer", "contentinfo"),
    ("main", "main"),
    ("aside", "complementary"),
    ("article", "article"),
];

/// Build a single WomNode from an element, or None if tag is unknown.
pub(crate) fn build_node(dom: &dyn DomEngine, el: ElementId, idx: usize) -> Option<WomNode> {
    let tag = dom.tag_name(el)?;
    let role = infer_role(dom, el, &tag);
    let label = dom.accessible_name(el);
    let visible = dom.is_visible(el);
    let interactive = dom.is_interactive(el);
    let value = dom.get_attribute(el, "value");
    let href = dom.get_attribute(el, "href");
    let contenteditable = dom.get_attribute(el, "contenteditable");
    let mut actions = infer_actions(&tag, dom.get_attribute(el, "type").as_deref());
    // contenteditable adds "type" action
    if (contenteditable.as_deref() == Some("true") || contenteditable.as_deref() == Some(""))
        && !actions.contains(&"type".to_string())
    {
        actions.push("type".to_string());
    }
    let parent_tag = "body"; // simplified -- real parent tracking needs tree walk
    let id = stable_id(&tag, &label, parent_tag, idx);

    Some(WomNode {
        id,
        tag,
        role,
        label,
        value,
        href,
        actions,
        visible,
        interactive,
    })
}

/// Infer semantic role from tag + attributes.
fn infer_role(dom: &dyn DomEngine, el: ElementId, tag: &str) -> String {
    if let Some(role) = dom.get_attribute(el, "role") {
        return role;
    }
    match tag {
        "button" => "button".to_string(),
        "a" => "link".to_string(),
        "input" => {
            let input_type = dom
                .get_attribute(el, "type")
                .unwrap_or_else(|| "text".to_string());
            match input_type.as_str() {
                "submit" => "button".to_string(),
                "checkbox" => "checkbox".to_string(),
                "radio" => "radio".to_string(),
                _ => "input".to_string(),
            }
        }
        "select" => "select".to_string(),
        "textarea" => "input".to_string(),
        "img" => "image".to_string(),
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => "heading".to_string(),
        "form" => "form".to_string(),
        "nav" => "navigation".to_string(),
        "header" => "banner".to_string(),
        "footer" => "contentinfo".to_string(),
        "main" => "main".to_string(),
        "aside" => "complementary".to_string(),
        "article" => "article".to_string(),
        _ => "text".to_string(),
    }
}

/// Infer available actions from tag and input type.
fn infer_actions(tag: &str, input_type: Option<&str>) -> Vec<String> {
    match tag {
        "button" => vec!["click".to_string()],
        "a" => vec!["click".to_string(), "navigate".to_string()],
        "input" => match input_type {
            Some("submit") => vec!["click".to_string()],
            Some("checkbox") => vec!["check".to_string(), "uncheck".to_string()],
            Some("radio") => vec!["select".to_string()],
            _ => vec!["type".to_string(), "clear".to_string()],
        },
        "select" => vec!["select".to_string()],
        "textarea" => vec!["type".to_string(), "clear".to_string()],
        "form" => vec!["submit".to_string(), "fill".to_string()],
        _ => vec![],
    }
}

/// Generate a stable ID from element properties.
///
/// Uses hash of tag + text prefix + parent_tag + sibling index.
pub(crate) fn stable_id(tag: &str, text: &str, parent_tag: &str, sibling_index: usize) -> String {
    let prefix = if text.len() > 20 { &text[..20] } else { text };
    let mut hasher = DefaultHasher::new();
    tag.hash(&mut hasher);
    prefix.hash(&mut hasher);
    parent_tag.hash(&mut hasher);
    sibling_index.hash(&mut hasher);
    format!("w{:x}", hasher.finish())
}

/// Generate an AI-readable one-line summary of the WOM.
///
/// Example: "Login page: 2 inputs (email, password), 1 submit button, 3 links"
pub(crate) fn generate_summary(title: &str, nodes: &[WomNode]) -> String {
    let n_inputs = nodes.iter().filter(|n| n.role == "input").count();
    let n_buttons = nodes.iter().filter(|n| n.role == "button").count();
    let n_links = nodes.iter().filter(|n| n.role == "link").count();
    let n_checkboxes = nodes.iter().filter(|n| n.role == "checkbox").count();
    let n_selects = nodes.iter().filter(|n| n.role == "select").count();
    let n_forms = nodes.iter().filter(|n| n.role == "form").count();

    let input_labels: Vec<&str> = nodes
        .iter()
        .filter(|n| n.role == "input" && !n.label.is_empty())
        .map(|n| n.label.as_str())
        .collect();

    let mut parts = Vec::new();

    if n_inputs > 0 {
        if input_labels.is_empty() {
            parts.push(format!("{n_inputs} inputs"));
        } else {
            let labels = input_labels.join(", ");
            parts.push(format!("{n_inputs} inputs ({labels})"));
        }
    }
    if n_checkboxes > 0 {
        parts.push(format!("{n_checkboxes} checkboxes"));
    }
    if n_selects > 0 {
        parts.push(format!("{n_selects} selects"));
    }
    if n_buttons > 0 {
        let submit_count = nodes
            .iter()
            .filter(|n| n.role == "button" && n.actions.contains(&"click".to_string()))
            .count();
        if submit_count > 0 && submit_count == n_buttons {
            parts.push(format!("{n_buttons} submit buttons"));
        } else {
            parts.push(format!("{n_buttons} buttons"));
        }
    }
    if n_links > 0 {
        parts.push(format!("{n_links} links"));
    }
    if n_forms > 0 {
        parts.push(format!("{n_forms} forms"));
    }

    let elements = if parts.is_empty() {
        "empty page".to_string()
    } else {
        parts.join(", ")
    };

    if title.is_empty() {
        elements
    } else {
        format!("{title}: {elements}")
    }
}
