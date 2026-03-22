//! WOM (Web Object Model) — the structured page representation for AI agents.
//!
//! Walks the DOM tree and builds a flat list of [`WomNode`]s representing
//! every interactive or informational element the AI should know about.

use neo_dom::DomEngine;
use serde::{Deserialize, Serialize};

use crate::wom_builder::{
    build_node, generate_summary, stable_id, CONTAINER_TAGS, LANDMARK_TAGS,
    MEANINGFUL_ARIA_ROLES, TEXT_CONTENT_TAGS,
};

/// A single `<option>` inside a `<select>`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SelectOption {
    pub value: String,
    pub text: String,
    pub selected: bool,
}

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
    // -- Form enrichment fields --
    /// Input type attribute: "text", "email", "password", "checkbox", etc.
    pub input_type: Option<String>,
    /// Input name attribute.
    pub name: Option<String>,
    /// Checkbox/radio checked state.
    pub checked: Option<bool>,
    /// Option selected state.
    pub selected: Option<bool>,
    /// Whether the field is required.
    pub required: bool,
    /// Whether the field is disabled.
    pub disabled: bool,
    /// Whether the field is readonly.
    pub readonly: bool,
    /// Placeholder text.
    pub placeholder: Option<String>,
    /// Validation pattern.
    pub pattern: Option<String>,
    /// Minimum value (for number/date inputs).
    pub min: Option<String>,
    /// Maximum value (for number/date inputs).
    pub max: Option<String>,
    /// Minimum length.
    pub minlength: Option<i32>,
    /// Maximum length.
    pub maxlength: Option<i32>,
    /// Autocomplete hint.
    pub autocomplete: Option<String>,
    /// Associated form id.
    pub form_id: Option<String>,
    /// Options for `<select>` elements.
    pub options: Vec<SelectOption>,
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
/// Iterates all interactive elements (buttons, inputs, links, selects),
/// informational elements (headings, images, text, lists, tables),
/// and landmark/ARIA-role elements to create the action map.
pub fn build_wom(dom: &dyn DomEngine, url: &str) -> WomDocument {
    let mut nodes = Vec::new();
    let mut idx: usize = 0;
    // Track element IDs already collected to avoid duplicates across passes.
    let mut seen: std::collections::HashSet<usize> = std::collections::HashSet::new();

    // --- Interactive elements ---

    // Collect buttons
    for el in dom.get_buttons() {
        if seen.insert(el) {
            if let Some(node) = build_node(dom, el, idx) {
                nodes.push(node);
                idx += 1;
            }
        }
    }

    // Collect inputs
    for el in dom.get_inputs() {
        if seen.insert(el) {
            if let Some(node) = build_node(dom, el, idx) {
                nodes.push(node);
                idx += 1;
            }
        }
    }

    // Collect links
    for link in dom.get_links() {
        let id = stable_id("a", &link.text, "body", idx);
        nodes.push(WomNode {
            id,
            tag: "a".to_string(),
            role: "link".to_string(),
            label: link.text.clone(),
            value: None,
            href: Some(link.href.clone()),
            actions: vec!["click".to_string(), "navigate".to_string()],
            visible: true,
            interactive: true,
            input_type: None,
            name: None,
            checked: None,
            selected: None,
            required: false,
            disabled: false,
            readonly: false,
            placeholder: None,
            pattern: None,
            min: None,
            max: None,
            minlength: None,
            maxlength: None,
            autocomplete: None,
            form_id: None,
            options: Vec::new(),
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
            input_type: None,
            name: None,
            checked: None,
            selected: None,
            required: false,
            disabled: false,
            readonly: false,
            placeholder: None,
            pattern: None,
            min: None,
            max: None,
            minlength: None,
            maxlength: None,
            autocomplete: None,
            form_id: None,
            options: Vec::new(),
        });
        idx += 1;
    }

    // Collect selects
    for el in dom.query_selector_all("select") {
        if seen.insert(el) {
            if let Some(node) = build_node(dom, el, idx) {
                nodes.push(node);
                idx += 1;
            }
        }
    }

    // Collect textareas
    for el in dom.query_selector_all("textarea") {
        if seen.insert(el) {
            if let Some(node) = build_node(dom, el, idx) {
                nodes.push(node);
                idx += 1;
            }
        }
    }

    // Collect details/summary (interactive collapsible sections)
    for tag in &["details", "summary"] {
        for el in dom.query_selector_all(tag) {
            if seen.insert(el) {
                if let Some(node) = build_node(dom, el, idx) {
                    nodes.push(node);
                    idx += 1;
                }
            }
        }
    }

    // --- Landmark elements ---

    for &(tag, role) in LANDMARK_TAGS {
        for el in dom.query_selector_all(tag) {
            if seen.insert(el) {
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
                    input_type: None,
                    name: None,
                    checked: None,
                    selected: None,
                    required: false,
                    disabled: false,
                    readonly: false,
                    placeholder: None,
                    pattern: None,
                    min: None,
                    max: None,
                    minlength: None,
                    maxlength: None,
                    autocomplete: None,
                    form_id: None,
                    options: Vec::new(),
                });
                idx += 1;
            }
        }
    }

    // --- Headings ---

    for el in dom.query_selector_all("h1, h2, h3, h4, h5, h6") {
        if seen.insert(el) {
            if let Some(node) = build_node(dom, el, idx) {
                nodes.push(node);
                idx += 1;
            }
        }
    }

    // --- Images ---

    for el in dom.query_selector_all("img") {
        if seen.insert(el) {
            if let Some(node) = build_node(dom, el, idx) {
                // Only include images with meaningful alt/label
                if !node.label.is_empty() {
                    nodes.push(node);
                    idx += 1;
                }
            }
        }
    }

    // --- Text content elements ---

    for tag in TEXT_CONTENT_TAGS {
        for el in dom.query_selector_all(tag) {
            if seen.insert(el) {
                let text = dom.text_content(el);
                let trimmed = text.trim();
                // Only include text nodes with meaningful content (>1 char)
                if trimmed.len() > 1 {
                    let label = if trimmed.len() > 200 {
                        format!("{}...", &trimmed[..trimmed.char_indices().nth(200).map_or(trimmed.len(), |(i, _)| i)])
                    } else {
                        trimmed.to_string()
                    };
                    let id = stable_id(tag, &label, "body", idx);
                    let role = match *tag {
                        "li" => "listitem",
                        "td" | "th" => "cell",
                        "p" => "paragraph",
                        "blockquote" => "blockquote",
                        "pre" | "code" => "code",
                        "dt" => "term",
                        "dd" => "definition",
                        "time" => "time",
                        "label" => "label",
                        "span" => {
                            // Only include spans with aria-label or role
                            let has_aria = dom.get_attribute(el, "aria-label").is_some();
                            let has_role = dom.get_attribute(el, "role").is_some();
                            if has_aria || has_role {
                                "text"
                            } else {
                                continue; // Skip plain spans
                            }
                        }
                        _ => "text",
                    };
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
                        input_type: None,
                        name: None,
                        checked: None,
                        selected: None,
                        required: false,
                        disabled: false,
                        readonly: false,
                        placeholder: None,
                        pattern: None,
                        min: None,
                        max: None,
                        minlength: None,
                        maxlength: None,
                        autocomplete: None,
                        form_id: None,
                        options: Vec::new(),
                    });
                    idx += 1;
                }
            }
        }
    }

    // --- Container elements (lists, tables) ---

    for tag in CONTAINER_TAGS {
        for el in dom.query_selector_all(tag) {
            if seen.insert(el) {
                let label = dom.accessible_name(el);
                let child_selector = match *tag {
                    "ul" | "ol" => "li",
                    "dl" => "dt",
                    "table" => "tr",
                    _ => continue,
                };
                let child_count = dom.query_selector_all(child_selector).len();
                let count_label = if label.is_empty() {
                    format!("{child_count} items")
                } else {
                    format!("{label} ({child_count} items)")
                };
                let role = match *tag {
                    "table" => "table",
                    _ => "list",
                };
                let id = stable_id(tag, &count_label, "body", idx);
                nodes.push(WomNode {
                    id,
                    tag: tag.to_string(),
                    role: role.to_string(),
                    label: count_label,
                    value: None,
                    href: None,
                    actions: vec![],
                    visible: dom.is_visible(el),
                    interactive: false,
                    input_type: None,
                    name: None,
                    checked: None,
                    selected: None,
                    required: false,
                    disabled: false,
                    readonly: false,
                    placeholder: None,
                    pattern: None,
                    min: None,
                    max: None,
                    minlength: None,
                    maxlength: None,
                    autocomplete: None,
                    form_id: None,
                    options: Vec::new(),
                });
                idx += 1;
            }
        }
    }

    // --- Dialog elements ---

    for el in dom.query_selector_all("dialog") {
        if seen.insert(el) {
            let label = dom.accessible_name(el);
            let id = stable_id("dialog", &label, "body", idx);
            nodes.push(WomNode {
                id,
                tag: "dialog".to_string(),
                role: "dialog".to_string(),
                label,
                value: None,
                href: None,
                actions: vec![],
                visible: dom.is_visible(el),
                interactive: false,
                input_type: None,
                name: None,
                checked: None,
                selected: None,
                required: false,
                disabled: false,
                readonly: false,
                placeholder: None,
                pattern: None,
                min: None,
                max: None,
                minlength: None,
                maxlength: None,
                autocomplete: None,
                form_id: None,
                options: Vec::new(),
            });
            idx += 1;
        }
    }

    // --- Elements with meaningful ARIA roles ---
    // Catch divs/spans with role=dialog, role=alert, role=search, etc.

    for role_name in MEANINGFUL_ARIA_ROLES {
        let selector = format!("[role=\"{role_name}\"]");
        for el in dom.query_selector_all(&selector) {
            if seen.insert(el) {
                let label = dom.accessible_name(el);
                let tag = dom.tag_name(el).unwrap_or_else(|| "div".to_string());
                let id = stable_id(&tag, &label, "body", idx);
                let interactive = dom.is_interactive(el);
                let actions = if interactive {
                    vec!["click".to_string()]
                } else {
                    vec![]
                };
                nodes.push(WomNode {
                    id,
                    tag,
                    role: role_name.to_string(),
                    label,
                    value: None,
                    href: None,
                    actions,
                    visible: dom.is_visible(el),
                    interactive,
                    input_type: None,
                    name: None,
                    checked: None,
                    selected: None,
                    required: false,
                    disabled: false,
                    readonly: false,
                    placeholder: None,
                    pattern: None,
                    min: None,
                    max: None,
                    minlength: None,
                    maxlength: None,
                    autocomplete: None,
                    form_id: None,
                    options: Vec::new(),
                });
                idx += 1;
            }
        }
    }

    // --- Divs/spans with aria-label (SPA pseudo-landmarks) ---
    // Modern SPAs like ChatGPT use div[aria-label] extensively instead of
    // semantic HTML. These are meaningful named regions the AI should see.

    for el in dom.query_selector_all("[aria-label]") {
        if seen.insert(el) {
            let tag = dom.tag_name(el).unwrap_or_else(|| "div".to_string());
            // Skip tags we already have dedicated passes for
            if matches!(
                tag.as_str(),
                "button" | "input" | "a" | "select" | "textarea" | "img" | "form"
                    | "nav" | "header" | "footer" | "main" | "aside" | "article"
                    | "section" | "dialog" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
            ) {
                continue;
            }
            let label = dom.accessible_name(el);
            if label.is_empty() {
                continue;
            }
            let role = dom
                .get_attribute(el, "role")
                .unwrap_or_else(|| "region".to_string());
            let interactive = dom.is_interactive(el);
            let actions = if interactive {
                vec!["click".to_string()]
            } else {
                vec![]
            };
            let id = stable_id(&tag, &label, "body", idx);
            nodes.push(WomNode {
                id,
                tag,
                role,
                label,
                value: None,
                href: None,
                actions,
                visible: dom.is_visible(el),
                interactive,
                input_type: None,
                name: None,
                checked: None,
                selected: None,
                required: false,
                disabled: false,
                readonly: false,
                placeholder: None,
                pattern: None,
                min: None,
                max: None,
                minlength: None,
                maxlength: None,
                autocomplete: None,
                form_id: None,
                options: Vec::new(),
            });
            idx += 1;
        }
    }

    // --- Contenteditable elements (not yet collected) ---

    for el in dom.query_selector_all("[contenteditable=\"true\"], [contenteditable=\"\"]") {
        if seen.insert(el) {
            if let Some(node) = build_node(dom, el, idx) {
                nodes.push(node);
                idx += 1;
            }
        }
    }

    // --- Progress/meter indicators ---

    for tag in &["progress", "meter"] {
        for el in dom.query_selector_all(tag) {
            if seen.insert(el) {
                if let Some(node) = build_node(dom, el, idx) {
                    nodes.push(node);
                    idx += 1;
                }
            }
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
