//! WOM (Web Object Model) builder.
//!
//! Walks the DOM tree and builds a flat list of [`WomNode`]s representing
//! every interactive or informational element the AI should know about.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use neo_dom::{DomEngine, ElementId};
use serde::{Deserialize, Serialize};

/// One interactive or informational element in the WOM.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WomNode {
    /// Stable ID: hash(tag + role + text_prefix + sibling_index).
    pub id: String,
    /// HTML tag name.
    pub tag: String,
    /// Semantic role: button, link, input, heading, text, image, form.
    pub role: String,
    /// Computed accessible name.
    pub label: String,
    /// Current value (inputs, selects).
    pub value: Option<String>,
    /// Link target.
    pub href: Option<String>,
    /// Available actions: click, type, select, submit.
    pub actions: Vec<String>,
    /// Whether the element is visible.
    pub visible: bool,
    /// Whether the element is interactive.
    pub interactive: bool,
}

/// The full WOM document — what the AI sees for a page.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WomDocument {
    /// Current URL.
    pub url: String,
    /// Document title.
    pub title: String,
    /// All WOM nodes.
    pub nodes: Vec<WomNode>,
    /// Classified page type.
    pub page_type: String,
    /// One-line summary for AI context.
    pub summary: String,
}

/// Build a WOM document from the DOM.
///
/// Iterates all interactive elements (buttons, inputs, links, selects)
/// and informational elements (headings, images) to create the action map.
pub fn build_wom(dom: &dyn DomEngine, url: &str) -> WomDocument {
    let mut nodes = Vec::new();
    let mut idx: usize = 0;

    // Collect buttons
    for el in dom.get_buttons() {
        if let Some(node) = build_node(dom, el, idx) {
            nodes.push(node);
            idx += 1;
        }
    }

    // Collect inputs
    for el in dom.get_inputs() {
        if let Some(node) = build_node(dom, el, idx) {
            nodes.push(node);
            idx += 1;
        }
    }

    // Collect links
    for link in dom.get_links() {
        let role = "link".to_string();
        let id = stable_id("a", &role, &link.text, idx);
        nodes.push(WomNode {
            id,
            tag: "a".to_string(),
            role,
            label: link.text.clone(),
            value: None,
            href: Some(link.href.clone()),
            actions: vec!["click".to_string()],
            visible: true,
            interactive: true,
        });
        idx += 1;
    }

    // Collect form-level nodes
    for form in dom.get_forms() {
        let label = form.id.clone().unwrap_or_default();
        let id = stable_id("form", "form", &label, idx);
        nodes.push(WomNode {
            id,
            tag: "form".to_string(),
            role: "form".to_string(),
            label,
            value: None,
            href: None,
            actions: vec!["submit".to_string()],
            visible: true,
            interactive: true,
        });
        idx += 1;
    }

    let title = dom.title();
    let n_links = dom.get_links().len();
    let n_buttons = dom.get_buttons().len();
    let n_forms = dom.get_forms().len();
    let summary = format!("{n_links} links, {n_buttons} buttons, {n_forms} forms. {title}");

    WomDocument {
        url: url.to_string(),
        title,
        nodes,
        page_type: "unknown".to_string(),
        summary,
    }
}

/// Build a single WomNode from an element, or None if tag is unknown.
fn build_node(dom: &dyn DomEngine, el: ElementId, idx: usize) -> Option<WomNode> {
    let tag = dom.tag_name(el)?;
    let role = infer_role(dom, el, &tag);
    let label = dom.accessible_name(el);
    let visible = dom.is_visible(el);
    let interactive = dom.is_interactive(el);
    let value = dom.get_attribute(el, "value");
    let href = dom.get_attribute(el, "href");
    let actions = infer_actions(&tag, dom.get_attribute(el, "type").as_deref());
    let id = stable_id(&tag, &role, &label, idx);

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
    // Explicit ARIA role takes precedence
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
        _ => "text".to_string(),
    }
}

/// Infer available actions from tag and input type.
fn infer_actions(tag: &str, input_type: Option<&str>) -> Vec<String> {
    match tag {
        "button" => vec!["click".to_string()],
        "a" => vec!["click".to_string()],
        "input" => match input_type {
            Some("submit") => vec!["click".to_string()],
            Some("checkbox") | Some("radio") => vec!["click".to_string()],
            _ => vec!["type".to_string()],
        },
        "select" => vec!["select".to_string()],
        "textarea" => vec!["type".to_string()],
        "form" => vec!["submit".to_string()],
        _ => vec![],
    }
}

/// Generate a stable ID from element properties.
///
/// Uses hash of tag + role + text prefix + sibling index.
fn stable_id(tag: &str, role: &str, text: &str, sibling_index: usize) -> String {
    let prefix = if text.len() > 20 { &text[..20] } else { text };
    let mut hasher = DefaultHasher::new();
    tag.hash(&mut hasher);
    role.hash(&mut hasher);
    prefix.hash(&mut hasher);
    sibling_index.hash(&mut hasher);
    format!("w{:x}", hasher.finish())
}
