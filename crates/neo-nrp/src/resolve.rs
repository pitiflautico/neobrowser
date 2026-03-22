//! Target resolver — resolves `Target` variants to concrete node_id strings.
//!
//! This is an agent helper (not part of the core wire protocol).
//! Core `Interact` commands only accept node_id; the resolver bridges
//! the gap between human-friendly targeting and the protocol.

use neo_dom::DomEngine;

use crate::types::{NrpError, SemanticNode, Target};

/// Resolve a `Target` to a node_id string.
///
/// - `NodeId` passes through directly.
/// - `Css` uses `DomEngine::query_selector`.
/// - `Text` walks the semantic tree for name/text matches.
/// - `Role` walks the semantic tree for role + optional name matches.
/// - `Label` finds the associated input via label text.
pub fn resolve_target(
    dom: &dyn DomEngine,
    tree: &SemanticNode,
    target: &Target,
) -> Result<String, NrpError> {
    match target {
        Target::NodeId(id) => Ok(id.clone()),

        Target::Css(selector) => {
            dom.query_selector(selector)
                .map(|el| format!("n{el}"))
                .ok_or_else(|| NrpError {
                    code: NrpError::TARGET_NOT_FOUND,
                    message: format!("no element matches CSS selector: {selector}"),
                })
        }

        Target::Text { value, exact } => {
            let exact = exact.unwrap_or(false);
            find_by_text(tree, value, exact).ok_or_else(|| NrpError {
                code: NrpError::TARGET_NOT_FOUND,
                message: format!("no element matches text: {value:?} (exact={exact})"),
            })
        }

        Target::Role { value, name } => {
            find_by_role(tree, value, name.as_deref()).ok_or_else(|| NrpError {
                code: NrpError::TARGET_NOT_FOUND,
                message: format!(
                    "no element matches role={value:?} name={name:?}"
                ),
            })
        }

        Target::Label(label_text) => {
            // Strategy 1: find a label element, get its `for` attribute,
            // then look up the referenced element.
            if let Some(node_id) = find_by_label(dom, tree, label_text) {
                return Ok(node_id);
            }
            // Strategy 2: find label in tree, return its first interactive child
            if let Some(node_id) = find_label_child(tree, label_text) {
                return Ok(node_id);
            }
            Err(NrpError {
                code: NrpError::TARGET_NOT_FOUND,
                message: format!("no input associated with label: {label_text:?}"),
            })
        }
    }
}

/// Walk the tree to find a node whose name matches `text`.
fn find_by_text(node: &SemanticNode, text: &str, exact: bool) -> Option<String> {
    let matches = if exact {
        node.name == text
    } else {
        node.name.to_lowercase().contains(&text.to_lowercase())
    };

    if matches && node.role != "generic" {
        return Some(node.node_id.clone());
    }

    // Depth-first: prefer deeper matches (more specific)
    for child in &node.children {
        if let Some(id) = find_by_text(child, text, exact) {
            return Some(id);
        }
    }

    // If we matched at this level but skipped because of generic role, still return
    if matches {
        return Some(node.node_id.clone());
    }

    None
}

/// Walk the tree to find a node matching role and optional name.
fn find_by_role(node: &SemanticNode, role: &str, name: Option<&str>) -> Option<String> {
    let role_matches = node.role == role;
    let name_matches = match name {
        Some(n) => node.name.to_lowercase().contains(&n.to_lowercase()),
        None => true,
    };

    if role_matches && name_matches {
        return Some(node.node_id.clone());
    }

    for child in &node.children {
        if let Some(id) = find_by_role(child, role, name) {
            return Some(id);
        }
    }

    None
}

/// Find an input associated with a label via `for` attribute.
fn find_by_label(dom: &dyn DomEngine, tree: &SemanticNode, label_text: &str) -> Option<String> {
    // Find all label nodes in the tree
    let label_node = find_node_by_role_and_text(tree, "label", label_text)?;

    // Parse the node_id to get the element index
    let el_id: usize = label_node.node_id.strip_prefix('n')?.parse().ok()?;

    // Get the `for` attribute
    let for_id = dom.get_attribute(el_id, "for")?;

    // Find the element with that ID via CSS
    dom.query_selector(&format!("#{for_id}"))
        .map(|el| format!("n{el}"))
}

/// Find a label node's first interactive child (for wrapped inputs).
fn find_label_child(tree: &SemanticNode, label_text: &str) -> Option<String> {
    let label_node = find_node_by_role_and_text(tree, "label", label_text)?;

    // Return first interactive child
    for child in &label_node.children {
        if !child.actions.is_empty() {
            return Some(child.node_id.clone());
        }
    }

    None
}

/// Find a node by role and text content.
fn find_node_by_role_and_text<'a>(
    node: &'a SemanticNode,
    role: &str,
    text: &str,
) -> Option<&'a SemanticNode> {
    if node.role == role && node.name.to_lowercase().contains(&text.to_lowercase()) {
        return Some(node);
    }
    for child in &node.children {
        if let Some(found) = find_node_by_role_and_text(child, role, text) {
            return Some(found);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{HtmlMetadata, NodeProperties};

    fn mock_props() -> NodeProperties {
        NodeProperties {
            disabled: false,
            required: false,
            checked: None,
            selected: None,
            expanded: None,
            focused: false,
            editable: false,
            readonly: false,
        }
    }

    fn mock_meta() -> HtmlMetadata {
        HtmlMetadata {
            input_type: None,
            href: None,
            action: None,
            method: None,
            name: None,
            placeholder: None,
            form_id: None,
        }
    }

    fn leaf(id: &str, role: &str, name: &str, actions: &[&str]) -> SemanticNode {
        SemanticNode {
            node_id: id.to_string(),
            role: role.to_string(),
            name: name.to_string(),
            value: None,
            description: None,
            tag: "div".to_string(),
            properties: mock_props(),
            html_metadata: mock_meta(),
            actions: actions.iter().map(|s| s.to_string()).collect(),
            children: vec![],
        }
    }

    fn make_tree() -> SemanticNode {
        SemanticNode {
            node_id: "n0".to_string(),
            role: "generic".to_string(),
            name: String::new(),
            value: None,
            description: None,
            tag: "body".to_string(),
            properties: mock_props(),
            html_metadata: mock_meta(),
            actions: vec![],
            children: vec![
                leaf("n1", "button", "Submit", &["click", "focus"]),
                leaf("n2", "textbox", "Email", &["type", "clear", "focus"]),
                leaf("n3", "link", "About Us", &["click", "focus"]),
            ],
        }
    }

    #[test]
    fn test_resolve_node_id() {
        let dom = neo_dom::MockDomEngine::new();
        let tree = make_tree();
        let target = Target::NodeId("n1".to_string());
        let result = resolve_target(&dom, &tree, &target).unwrap();
        assert_eq!(result, "n1");
    }

    #[test]
    fn test_resolve_css() {
        let mut dom = neo_dom::MockDomEngine::new();
        dom.add_element("button", &[], "Submit");

        let tree = make_tree();
        let target = Target::Css("button".to_string());
        let result = resolve_target(&dom, &tree, &target).unwrap();
        assert_eq!(result, "n0"); // MockDomEngine returns index 0
    }

    #[test]
    fn test_resolve_text_substring() {
        let dom = neo_dom::MockDomEngine::new();
        let tree = make_tree();
        let target = Target::Text {
            value: "submit".to_string(),
            exact: Some(false),
        };
        let result = resolve_target(&dom, &tree, &target).unwrap();
        assert_eq!(result, "n1");
    }

    #[test]
    fn test_resolve_text_exact() {
        let dom = neo_dom::MockDomEngine::new();
        let tree = make_tree();
        let target = Target::Text {
            value: "Submit".to_string(),
            exact: Some(true),
        };
        let result = resolve_target(&dom, &tree, &target).unwrap();
        assert_eq!(result, "n1");
    }

    #[test]
    fn test_resolve_text_not_found() {
        let dom = neo_dom::MockDomEngine::new();
        let tree = make_tree();
        let target = Target::Text {
            value: "Nonexistent".to_string(),
            exact: None,
        };
        let result = resolve_target(&dom, &tree, &target);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, NrpError::TARGET_NOT_FOUND);
    }

    #[test]
    fn test_resolve_role() {
        let dom = neo_dom::MockDomEngine::new();
        let tree = make_tree();
        let target = Target::Role {
            value: "button".to_string(),
            name: Some("Submit".to_string()),
        };
        let result = resolve_target(&dom, &tree, &target).unwrap();
        assert_eq!(result, "n1");
    }

    #[test]
    fn test_resolve_role_without_name() {
        let dom = neo_dom::MockDomEngine::new();
        let tree = make_tree();
        let target = Target::Role {
            value: "textbox".to_string(),
            name: None,
        };
        let result = resolve_target(&dom, &tree, &target).unwrap();
        assert_eq!(result, "n2");
    }
}
