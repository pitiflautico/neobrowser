//! Semantic tree builder — converts DOM into an AI-navigable tree.
//!
//! Walks the DOM via `DomEngine` trait methods and produces a
//! `SemanticNode` tree with heuristic roles, computed names,
//! and available actions.
//!
//! Limitations (documented per PDR):
//! - No real ARIA computation (heuristic role mapping)
//! - No CSS visibility (uses DomEngine::is_visible heuristic)
//! - No layout information

use neo_dom::{DomEngine, ElementId};

use crate::types::{HtmlMetadata, NodeProperties, SemanticNode};

/// Tags to skip entirely — they produce no semantic content.
const SKIP_TAGS: &[&str] = &[
    "script", "style", "noscript", "template", "meta", "link", "head",
];

/// Build a semantic tree from the current DOM state.
///
/// Returns a root `SemanticNode` representing `<body>` (or the
/// document root if no body is found). Children are built recursively.
pub fn build_semantic_tree(dom: &dyn DomEngine) -> SemanticNode {
    let root_id = dom.body().unwrap_or(0);
    let tag = dom.tag_name(root_id).unwrap_or_else(|| "body".to_string());
    let attrs = dom.get_attributes(root_id);

    build_node(dom, root_id, &tag, &attrs)
}

/// Build a `SemanticNode` for a single element and recurse into children.
fn build_node(
    dom: &dyn DomEngine,
    el: ElementId,
    tag: &str,
    attrs: &[(String, String)],
) -> SemanticNode {
    let role = compute_role(tag, attrs);
    let name = compute_name(dom, el, tag, attrs);
    let actions = compute_actions(tag, attrs);
    let properties = compute_properties(tag, attrs);
    let html_metadata = compute_html_metadata(tag, attrs);
    let value = compute_value(tag, attrs);
    let description = get_attr(attrs, "aria-describedby")
        .or_else(|| get_attr(attrs, "title"));

    let children = build_children(dom, el);

    SemanticNode {
        node_id: format!("n{el}"),
        role,
        name,
        value,
        description,
        tag: tag.to_string(),
        properties,
        html_metadata,
        actions,
        children,
    }
}

/// Recursively build child semantic nodes, skipping invisible and non-semantic elements.
fn build_children(dom: &dyn DomEngine, parent: ElementId) -> Vec<SemanticNode> {
    let child_ids = dom.children(parent);
    let mut children = Vec::new();

    for child_id in child_ids {
        let tag = match dom.tag_name(child_id) {
            Some(t) => t,
            None => continue,
        };

        // Skip non-semantic tags
        if SKIP_TAGS.contains(&tag.as_str()) {
            continue;
        }

        // Skip invisible elements
        if !dom.is_visible(child_id) {
            continue;
        }

        let attrs = dom.get_attributes(child_id);
        children.push(build_node(dom, child_id, &tag, &attrs));
    }

    children
}

/// Map HTML tag + attributes to an ARIA role.
///
/// Explicit `role` attribute overrides the implicit mapping.
fn compute_role(tag: &str, attrs: &[(String, String)]) -> String {
    // Explicit role attribute takes priority
    if let Some(role) = get_attr(attrs, "role") {
        return role;
    }

    match tag {
        "button" => "button".to_string(),
        "a" => {
            if has_attr(attrs, "href") {
                "link".to_string()
            } else {
                "generic".to_string()
            }
        }
        "input" => {
            let input_type = get_attr(attrs, "type")
                .unwrap_or_else(|| "text".to_string())
                .to_lowercase();
            match input_type.as_str() {
                "text" | "email" | "tel" | "url" | "search" | "password" | "number" => {
                    "textbox".to_string()
                }
                "checkbox" => "checkbox".to_string(),
                "radio" => "radio".to_string(),
                "submit" | "reset" | "button" | "image" => "button".to_string(),
                "range" => "slider".to_string(),
                "file" => "button".to_string(),
                "hidden" => "none".to_string(),
                _ => "textbox".to_string(),
            }
        }
        "textarea" => "textbox".to_string(),
        "select" => "combobox".to_string(),
        "option" => "option".to_string(),
        "img" => "img".to_string(),
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => "heading".to_string(),
        "nav" => "navigation".to_string(),
        "main" => "main".to_string(),
        "header" => "banner".to_string(),
        "footer" => "contentinfo".to_string(),
        "aside" => "complementary".to_string(),
        "section" => {
            if has_attr(attrs, "aria-label") || has_attr(attrs, "aria-labelledby") {
                "region".to_string()
            } else {
                "generic".to_string()
            }
        }
        "article" => "article".to_string(),
        "form" => "form".to_string(),
        "table" => "table".to_string(),
        "thead" => "rowgroup".to_string(),
        "tbody" => "rowgroup".to_string(),
        "tfoot" => "rowgroup".to_string(),
        "tr" => "row".to_string(),
        "th" => "columnheader".to_string(),
        "td" => "cell".to_string(),
        "ul" | "ol" => "list".to_string(),
        "li" => "listitem".to_string(),
        "dl" => "list".to_string(),
        "dt" => "term".to_string(),
        "dd" => "definition".to_string(),
        "details" => "group".to_string(),
        "summary" => "button".to_string(),
        "dialog" => "dialog".to_string(),
        "label" => "label".to_string(),
        "fieldset" => "group".to_string(),
        "legend" => "legend".to_string(),
        "p" | "div" | "span" => "generic".to_string(),
        _ => "generic".to_string(),
    }
}

/// Compute the accessible name for an element.
///
/// Priority: aria-label > aria-labelledby (not resolved) >
/// DomEngine::accessible_name > text_content (buttons/links) >
/// title > placeholder > alt.
fn compute_name(
    dom: &dyn DomEngine,
    el: ElementId,
    tag: &str,
    attrs: &[(String, String)],
) -> String {
    // aria-label is highest priority
    if let Some(label) = get_attr(attrs, "aria-label") {
        return label;
    }

    // Use DomEngine's accessible_name which handles aria-label, label[for], text
    let acc_name = dom.accessible_name(el);
    if !acc_name.is_empty() {
        return acc_name;
    }

    // For buttons and links, use text content
    if matches!(tag, "button" | "a" | "summary") {
        let text = dom.text_content(el);
        let trimmed = text.trim().to_string();
        if !trimmed.is_empty() {
            return trimmed;
        }
    }

    // Input-specific: value for submit buttons
    if tag == "input" {
        let input_type = get_attr(attrs, "type").unwrap_or_default();
        if matches!(input_type.as_str(), "submit" | "reset" | "button") {
            if let Some(val) = get_attr(attrs, "value") {
                return val;
            }
        }
    }

    // Fallbacks: title > placeholder > alt
    if let Some(title) = get_attr(attrs, "title") {
        return title;
    }
    if let Some(ph) = get_attr(attrs, "placeholder") {
        return ph;
    }
    if let Some(alt) = get_attr(attrs, "alt") {
        return alt;
    }

    String::new()
}

/// Compute available actions based on tag and attributes.
fn compute_actions(tag: &str, attrs: &[(String, String)]) -> Vec<String> {
    let mut actions = Vec::new();
    let disabled = get_attr(attrs, "disabled").is_some();

    if disabled {
        return actions;
    }

    match tag {
        "button" | "summary" => {
            actions.push("click".to_string());
            actions.push("focus".to_string());
        }
        "a" => {
            if has_attr(attrs, "href") {
                actions.push("click".to_string());
                actions.push("focus".to_string());
            }
        }
        "input" => {
            let input_type = get_attr(attrs, "type")
                .unwrap_or_else(|| "text".to_string())
                .to_lowercase();
            match input_type.as_str() {
                "text" | "email" | "tel" | "url" | "search" | "password" | "number" => {
                    actions.push("type".to_string());
                    actions.push("clear".to_string());
                    actions.push("focus".to_string());
                }
                "checkbox" => {
                    actions.push("check".to_string());
                    actions.push("focus".to_string());
                }
                "radio" => {
                    actions.push("check".to_string());
                    actions.push("focus".to_string());
                }
                "submit" | "reset" | "button" | "image" => {
                    actions.push("click".to_string());
                    actions.push("focus".to_string());
                }
                "file" => {
                    actions.push("upload".to_string());
                    actions.push("focus".to_string());
                }
                "range" => {
                    actions.push("select".to_string());
                    actions.push("focus".to_string());
                }
                _ => {
                    actions.push("type".to_string());
                    actions.push("focus".to_string());
                }
            }
        }
        "textarea" => {
            actions.push("type".to_string());
            actions.push("clear".to_string());
            actions.push("focus".to_string());
        }
        "select" => {
            actions.push("select".to_string());
            actions.push("focus".to_string());
        }
        "form" => {
            actions.push("submit".to_string());
        }
        "details" => {
            actions.push("expand".to_string());
        }
        _ => {
            // Contenteditable elements
            if get_attr(attrs, "contenteditable").as_deref() == Some("true") {
                actions.push("type".to_string());
                actions.push("clear".to_string());
                actions.push("focus".to_string());
            }
            // Elements with tabindex are focusable
            if has_attr(attrs, "tabindex")
                && !actions.contains(&"focus".to_string())
            {
                actions.push("focus".to_string());
            }
            // Elements with onclick are clickable
            if has_attr(attrs, "onclick") {
                actions.push("click".to_string());
            }
        }
    }

    actions
}

/// Compute interaction-relevant properties.
fn compute_properties(tag: &str, attrs: &[(String, String)]) -> NodeProperties {
    let disabled = has_attr(attrs, "disabled");
    let required = has_attr(attrs, "required");
    let readonly = has_attr(attrs, "readonly");
    let editable = get_attr(attrs, "contenteditable").as_deref() == Some("true")
        || matches!(tag, "input" | "textarea")
            && !readonly
            && !disabled
            && !matches!(
                get_attr(attrs, "type").as_deref(),
                Some("checkbox") | Some("radio") | Some("submit") | Some("reset")
                    | Some("button") | Some("hidden") | Some("file") | Some("image")
            );

    let checked = if matches!(tag, "input")
        && matches!(
            get_attr(attrs, "type").as_deref(),
            Some("checkbox") | Some("radio")
        ) {
        Some(has_attr(attrs, "checked"))
    } else {
        None
    };

    let selected = if tag == "option" {
        Some(has_attr(attrs, "selected"))
    } else {
        None
    };

    let expanded = if tag == "details" {
        Some(has_attr(attrs, "open"))
    } else {
        get_attr(attrs, "aria-expanded").map(|val| val == "true")
    };

    NodeProperties {
        disabled,
        required,
        checked,
        selected,
        expanded,
        focused: false, // Cannot determine from static DOM
        editable,
        readonly,
    }
}

/// Extract HTML-specific metadata.
fn compute_html_metadata(tag: &str, attrs: &[(String, String)]) -> HtmlMetadata {
    let input_type = if matches!(tag, "input") {
        Some(
            get_attr(attrs, "type")
                .unwrap_or_else(|| "text".to_string()),
        )
    } else {
        None
    };

    HtmlMetadata {
        input_type,
        href: get_attr(attrs, "href"),
        action: get_attr(attrs, "action"),
        method: get_attr(attrs, "method"),
        name: get_attr(attrs, "name"),
        placeholder: get_attr(attrs, "placeholder"),
        form_id: get_attr(attrs, "form"),
    }
}

/// Extract the current value of an element.
fn compute_value(tag: &str, attrs: &[(String, String)]) -> Option<String> {
    match tag {
        "input" | "textarea" => get_attr(attrs, "value"),
        "select" => get_attr(attrs, "value"),
        "option" => get_attr(attrs, "value"),
        _ => None,
    }
}

// ─── Attribute helpers ───

fn get_attr(attrs: &[(String, String)], name: &str) -> Option<String> {
    attrs
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.clone())
}

fn has_attr(attrs: &[(String, String)], name: &str) -> bool {
    attrs.iter().any(|(k, _)| k == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_role_explicit_override() {
        let attrs = vec![("role".to_string(), "navigation".to_string())];
        assert_eq!(compute_role("div", &attrs), "navigation");
    }

    #[test]
    fn test_compute_role_implicit_mappings() {
        let empty: Vec<(String, String)> = vec![];
        assert_eq!(compute_role("button", &empty), "button");
        assert_eq!(compute_role("h1", &empty), "heading");
        assert_eq!(compute_role("nav", &empty), "navigation");
        assert_eq!(compute_role("form", &empty), "form");
        assert_eq!(compute_role("ul", &empty), "list");
        assert_eq!(compute_role("li", &empty), "listitem");
        assert_eq!(compute_role("div", &empty), "generic");

        let href_attrs = vec![("href".to_string(), "/about".to_string())];
        assert_eq!(compute_role("a", &href_attrs), "link");
        assert_eq!(compute_role("a", &empty), "generic");
    }

    #[test]
    fn test_compute_role_input_types() {
        let text = vec![("type".to_string(), "text".to_string())];
        assert_eq!(compute_role("input", &text), "textbox");

        let checkbox = vec![("type".to_string(), "checkbox".to_string())];
        assert_eq!(compute_role("input", &checkbox), "checkbox");

        let submit = vec![("type".to_string(), "submit".to_string())];
        assert_eq!(compute_role("input", &submit), "button");

        let hidden = vec![("type".to_string(), "hidden".to_string())];
        assert_eq!(compute_role("input", &hidden), "none");
    }

    #[test]
    fn test_compute_actions_button() {
        let empty: Vec<(String, String)> = vec![];
        let actions = compute_actions("button", &empty);
        assert!(actions.contains(&"click".to_string()));
        assert!(actions.contains(&"focus".to_string()));
    }

    #[test]
    fn test_compute_actions_text_input() {
        let attrs = vec![("type".to_string(), "text".to_string())];
        let actions = compute_actions("input", &attrs);
        assert!(actions.contains(&"type".to_string()));
        assert!(actions.contains(&"clear".to_string()));
        assert!(actions.contains(&"focus".to_string()));
    }

    #[test]
    fn test_compute_actions_disabled() {
        let attrs = vec![("disabled".to_string(), String::new())];
        let actions = compute_actions("button", &attrs);
        assert!(actions.is_empty());
    }

    #[test]
    fn test_compute_properties_checkbox() {
        let attrs = vec![
            ("type".to_string(), "checkbox".to_string()),
            ("checked".to_string(), String::new()),
            ("required".to_string(), String::new()),
        ];
        let props = compute_properties("input", &attrs);
        assert_eq!(props.checked, Some(true));
        assert!(props.required);
        assert!(!props.disabled);
    }

    #[test]
    fn test_html_metadata_input() {
        let attrs = vec![
            ("type".to_string(), "email".to_string()),
            ("name".to_string(), "user_email".to_string()),
            ("placeholder".to_string(), "Enter email".to_string()),
        ];
        let meta = compute_html_metadata("input", &attrs);
        assert_eq!(meta.input_type.as_deref(), Some("email"));
        assert_eq!(meta.name.as_deref(), Some("user_email"));
        assert_eq!(meta.placeholder.as_deref(), Some("Enter email"));
    }

    #[test]
    fn test_build_semantic_tree_with_mock() {
        use neo_dom::MockDomEngine;

        let mut dom = MockDomEngine::new();

        // body (id=0)
        let body = dom.add_element("body", &[], "");
        dom.set_visible(body, true);

        // button (id=1)
        let btn = dom.add_element("button", &[], "Submit");
        dom.set_visible(btn, true);
        dom.set_interactive(btn, true);

        // input (id=2)
        let input = dom.add_element(
            "input",
            &[("type", "text"), ("placeholder", "Name")],
            "",
        );
        dom.set_visible(input, true);
        dom.set_interactive(input, true);

        dom.add_child(body, btn);
        dom.add_child(body, input);

        let tree = build_semantic_tree(&dom);
        assert_eq!(tree.node_id, "n0");
        assert_eq!(tree.children.len(), 2);

        let btn_node = &tree.children[0];
        assert_eq!(btn_node.role, "button");
        assert_eq!(btn_node.name, "Submit");
        assert!(btn_node.actions.contains(&"click".to_string()));

        let input_node = &tree.children[1];
        assert_eq!(input_node.role, "textbox");
        assert_eq!(input_node.html_metadata.placeholder.as_deref(), Some("Name"));
        assert!(input_node.actions.contains(&"type".to_string()));
    }
}
