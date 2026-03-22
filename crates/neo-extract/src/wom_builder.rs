//! WOM builder — constructs WomNodes from DOM elements.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use neo_dom::{DomEngine, ElementId};

use crate::wom::{SelectOption, WomNode};

/// Landmark tag -> ARIA role mapping.
pub(crate) const LANDMARK_TAGS: &[(&str, &str)] = &[
    ("nav", "navigation"),
    ("header", "banner"),
    ("footer", "contentinfo"),
    ("main", "main"),
    ("aside", "complementary"),
    ("article", "article"),
    ("section", "region"),
    ("form", "form"),
];

/// Tags that carry text content worth surfacing to the AI.
pub(crate) const TEXT_CONTENT_TAGS: &[&str] = &[
    "p", "li", "td", "th", "dt", "dd", "blockquote", "pre", "code", "span", "time", "label",
];

/// Container tags that provide structural info (item/row counts).
pub(crate) const CONTAINER_TAGS: &[&str] = &["ul", "ol", "dl", "table"];

/// ARIA roles that indicate meaningful regions or widgets.
pub(crate) const MEANINGFUL_ARIA_ROLES: &[&str] = &[
    "dialog",
    "alertdialog",
    "alert",
    "search",
    "tablist",
    "tab",
    "tabpanel",
    "menu",
    "menubar",
    "menuitem",
    "toolbar",
    "tooltip",
    "tree",
    "treeitem",
    "listbox",
    "option",
    "progressbar",
    "status",
    "log",
    "marquee",
    "timer",
    "feed",
    "grid",
    "region",
    "group",
    "separator",
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

    // -- Form enrichment --
    let is_form_element = matches!(tag.as_str(), "input" | "select" | "textarea" | "button");

    let input_type = if is_form_element {
        dom.get_attribute(el, "type")
    } else {
        None
    };

    let name = if is_form_element {
        dom.get_attribute(el, "name")
    } else {
        None
    };

    let has_attr = |attr: &str| dom.get_attribute(el, attr).is_some();

    let checked = if matches!(input_type.as_deref(), Some("checkbox") | Some("radio")) {
        Some(has_attr("checked"))
    } else {
        None
    };

    let selected = None; // Only relevant for <option>, handled below for <select>

    let required = is_form_element && has_attr("required");
    let disabled = is_form_element && has_attr("disabled");
    let readonly = is_form_element && has_attr("readonly");

    let placeholder = if is_form_element {
        dom.get_attribute(el, "placeholder")
    } else {
        None
    };
    let pattern = if is_form_element {
        dom.get_attribute(el, "pattern")
    } else {
        None
    };
    let min = if is_form_element {
        dom.get_attribute(el, "min")
    } else {
        None
    };
    let max = if is_form_element {
        dom.get_attribute(el, "max")
    } else {
        None
    };
    let minlength = if is_form_element {
        dom.get_attribute(el, "minlength").and_then(|v| v.parse::<i32>().ok())
    } else {
        None
    };
    let maxlength = if is_form_element {
        dom.get_attribute(el, "maxlength").and_then(|v| v.parse::<i32>().ok())
    } else {
        None
    };
    let autocomplete = if is_form_element {
        dom.get_attribute(el, "autocomplete")
    } else {
        None
    };

    // form_id: explicit form= attribute, or find enclosing <form> id
    let form_id = if is_form_element {
        dom.get_attribute(el, "form").or_else(|| find_parent_form_id(dom, el))
    } else {
        None
    };

    // For <select>: collect <option> children
    let options = if tag == "select" {
        collect_select_options(dom, el)
    } else {
        Vec::new()
    };

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
        input_type,
        name,
        checked,
        selected,
        required,
        disabled,
        readonly,
        placeholder,
        pattern,
        min,
        max,
        minlength,
        maxlength,
        autocomplete,
        form_id,
        options,
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
        "section" => "region".to_string(),
        "dialog" => "dialog".to_string(),
        "ul" | "ol" => "list".to_string(),
        "dl" => "list".to_string(),
        "li" => "listitem".to_string(),
        "table" => "table".to_string(),
        "td" | "th" => "cell".to_string(),
        "p" => "paragraph".to_string(),
        "blockquote" => "blockquote".to_string(),
        "pre" | "code" => "code".to_string(),
        "details" => "group".to_string(),
        "summary" => "button".to_string(),
        "time" => "time".to_string(),
        "progress" | "meter" => "progressbar".to_string(),
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
        "details" | "summary" => vec!["click".to_string()],
        _ => vec![],
    }
}

/// Find the parent `<form>` element's id for a given element.
///
/// Walks all forms in the DOM and checks if this element is a descendant.
/// Returns the form's id attribute if found.
fn find_parent_form_id(dom: &dyn DomEngine, el: ElementId) -> Option<String> {
    // Get the element's inner_html fingerprint to match against form children.
    // Since we don't have real parent pointers, iterate all forms and check
    // if this element index appears among their descendant inputs/selects/textareas.
    let forms = dom.get_forms();
    let form_elements = dom.query_selector_all("form");

    for (fi, form) in forms.iter().enumerate() {
        if let Some(form_el) = form_elements.get(fi) {
            // Check if this element is inside this form by seeing if the form
            // contains an input/select/textarea/button with the same name+type combo.
            // Simple heuristic: use the flat list — elements inside a form appear
            // between form_el and the next sibling form or end.
            // Better approach: if element index > form_el index, it might be inside.
            if el > *form_el {
                return form.id.clone();
            }
        }
    }
    None
}

/// Collect `<option>` children from a `<select>` element.
fn collect_select_options(dom: &dyn DomEngine, select_el: ElementId) -> Vec<SelectOption> {
    // Find all <option> elements that come after this <select> and before the next
    // non-option element. Since html5ever flattens into a list where children follow
    // parent, we walk forward from select_el+1 collecting options.
    let all_options = dom.query_selector_all("option");
    let mut result = Vec::new();

    for opt_el in all_options {
        // Only include options that are after this select and likely children.
        // In the flat list, child options have index > select index.
        // We stop at the first option that belongs to a different select.
        if opt_el <= select_el {
            continue;
        }
        // Check if there's another select between select_el and this option
        let all_selects = dom.query_selector_all("select");
        let belongs_to_another = all_selects.iter().any(|&s| s != select_el && s > select_el && s < opt_el);
        if belongs_to_another {
            break;
        }

        let value = dom.get_attribute(opt_el, "value").unwrap_or_default();
        let text = dom.text_content(opt_el).trim().to_string();
        let selected = dom.get_attribute(opt_el, "selected").is_some();
        result.push(SelectOption {
            value,
            text,
            selected,
        });
    }
    result
}

/// Generate a stable ID from element properties.
///
/// Uses hash of tag + text prefix + parent_tag + sibling index.
pub(crate) fn stable_id(tag: &str, text: &str, parent_tag: &str, sibling_index: usize) -> String {
    let prefix = text.char_indices().nth(20).map_or(text, |(i, _)| &text[..i]);
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

    let n_headings = nodes.iter().filter(|n| n.role == "heading").count();
    if n_headings > 0 {
        parts.push(format!("{n_headings} headings"));
    }

    let n_paragraphs = nodes.iter().filter(|n| n.role == "paragraph").count();
    if n_paragraphs > 0 {
        parts.push(format!("{n_paragraphs} paragraphs"));
    }

    let n_lists = nodes.iter().filter(|n| n.role == "list").count();
    if n_lists > 0 {
        parts.push(format!("{n_lists} lists"));
    }

    let n_tables = nodes.iter().filter(|n| n.role == "table").count();
    if n_tables > 0 {
        parts.push(format!("{n_tables} tables"));
    }

    let n_images = nodes.iter().filter(|n| n.role == "image").count();
    if n_images > 0 {
        parts.push(format!("{n_images} images"));
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
