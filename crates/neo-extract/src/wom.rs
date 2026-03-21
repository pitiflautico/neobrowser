//! WOM (Web Object Model) — the structured page representation for AI agents.
//!
//! Walks the DOM tree and builds a flat list of [`WomNode`]s representing
//! every interactive or informational element the AI should know about.

use neo_dom::DomEngine;
use serde::{Deserialize, Serialize};

use crate::wom_builder::{build_node, generate_summary, stable_id, LANDMARK_TAGS};

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
