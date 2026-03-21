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
    /// Stable ID: hash(tag + text_prefix + parent_tag + sibling_index).
    pub id: String,
    /// HTML tag name.
    pub tag: String,
    /// Semantic role: button, link, input, heading, text, image, form,
    /// navigation, banner, contentinfo, main, complementary, article.
    pub role: String,
    /// Computed accessible name.
    pub label: String,
    /// Current value (inputs, selects).
    pub value: Option<String>,
    /// Link target.
    pub href: Option<String>,
    /// Available actions: click, navigate, type, clear, check, uncheck,
    /// select, submit, fill.
    pub actions: Vec<String>,
    /// Whether the element is visible.
    pub visible: bool,
    /// Whether the element is interactive.
    pub interactive: bool,
}

/// The full WOM document -- what the AI sees for a page.
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
/// and informational elements (headings, images, landmarks) to create the action map.
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
        let id = stable_id("a", &link.text, "body", idx);
        nodes.push(WomNode {
            id,
            tag: "a".to_string(),
            role,
            label: link.text.clone(),
            value: None,
            href: Some(link.href.clone()),
            actions: vec!["click".to_string(), "navigate".to_string()],
            visible: true,
            interactive: true,
        });
        idx += 1;
    }

    // Collect form-level nodes
    for form in dom.get_forms() {
        let label = form.id.clone().unwrap_or_default();
        let id = stable_id("form", &label, "body", idx);
        nodes.push(WomNode {
            id,
            tag: "form".to_string(),
            role: "form".to_string(),
            label,
            value: None,
            href: None,
            actions: vec!["submit".to_string(), "fill".to_string()],
            visible: true,
            interactive: true,
        });
        idx += 1;
    }

    // Collect landmark elements
    for &(tag, role) in LANDMARK_TAGS {
        for el in dom.query_selector_all(tag) {
            let label = dom.accessible_name(el);
            let id = stable_id(tag, &label, "body", idx);
            nodes.push(WomNode {
                id,
                tag: tag.to_string(),
                role: role.to_string(),
                label,
                value: None,
                href: None,
                actions: vec![],
                visible: dom.is_visible(el),
                interactive: false,
            });
            idx += 1;
        }
    }

    // Collect selects
    for el in dom.query_selector_all("select") {
        if let Some(node) = build_node(dom, el, idx) {
            nodes.push(node);
            idx += 1;
        }
    }

    // Collect textareas
    for el in dom.query_selector_all("textarea") {
        if let Some(node) = build_node(dom, el, idx) {
            nodes.push(node);
            idx += 1;
        }
    }

    let title = dom.title();
    let summary = generate_summary(&title, &nodes);

    WomDocument {
        url: url.to_string(),
        title,
        nodes,
        page_type: "unknown".to_string(),
        summary,
    }
}

/// Landmark tag -> ARIA role mapping.
const LANDMARK_TAGS: &[(&str, &str)] = &[
    ("nav", "navigation"),
    ("header", "banner"),
    ("footer", "contentinfo"),
    ("main", "main"),
    ("aside", "complementary"),
    ("article", "article"),
];

/// Build a single WomNode from an element, or None if tag is unknown.
fn build_node(dom: &dyn DomEngine, el: ElementId, idx: usize) -> Option<WomNode> {
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
    if contenteditable.as_deref() == Some("true") || contenteditable.as_deref() == Some("") {
        if !actions.contains(&"type".to_string()) {
            actions.push("type".to_string());
        }
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
        // Landmark elements
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
fn stable_id(tag: &str, text: &str, parent_tag: &str, sibling_index: usize) -> String {
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
fn generate_summary(title: &str, nodes: &[WomNode]) -> String {
    let n_inputs = nodes.iter().filter(|n| n.role == "input").count();
    let n_buttons = nodes.iter().filter(|n| n.role == "button").count();
    let n_links = nodes.iter().filter(|n| n.role == "link").count();
    let n_checkboxes = nodes.iter().filter(|n| n.role == "checkbox").count();
    let n_selects = nodes.iter().filter(|n| n.role == "select").count();
    let n_forms = nodes.iter().filter(|n| n.role == "form").count();

    // Collect input labels for semantic context
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
            .filter(|n| {
                n.role == "button" && n.actions.contains(&"click".to_string())
            })
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
