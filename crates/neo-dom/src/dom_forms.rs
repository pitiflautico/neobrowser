//! Form field collection and HTML serialization helpers for Html5everDom.

use markup5ever_rcdom::{Handle, NodeData};
use neo_types::FormField;

use crate::html5ever_dom::ElementInfo;
use crate::query::collect_text_content;

/// Collect form fields (input, select, textarea) from a form subtree.
pub(crate) fn collect_form_fields(
    form_handle: &Handle,
    all_elements: &[ElementInfo],
    all_handles: &[Handle],
) -> Vec<FormField> {
    let mut fields = Vec::new();
    collect_fields_recursive(form_handle, all_elements, all_handles, &mut fields);
    fields
}

/// Recursive form field collector.
fn collect_fields_recursive(
    handle: &Handle,
    all_elements: &[ElementInfo],
    all_handles: &[Handle],
    fields: &mut Vec<FormField>,
) {
    if let NodeData::Element {
        ref name,
        ref attrs,
        ..
    } = handle.data
    {
        let tag = name.local.to_string();
        if tag == "input" || tag == "select" || tag == "textarea" {
            let attrs = attrs.borrow();
            let get = |n: &str| -> Option<String> {
                attrs
                    .iter()
                    .find(|a| a.name.local.as_ref() == n)
                    .map(|a| a.value.to_string())
            };
            let field_name = get("name").unwrap_or_default();
            let field_type = if tag == "input" {
                get("type").unwrap_or_else(|| "text".to_string())
            } else {
                tag.clone()
            };
            let label = get("id").and_then(|id| find_label_text(&id, all_elements, all_handles));
            fields.push(FormField {
                name: field_name,
                field_type,
                value: get("value"),
                required: attrs.iter().any(|a| a.name.local.as_ref() == "required"),
                placeholder: get("placeholder"),
                label,
                disabled: attrs.iter().any(|a| a.name.local.as_ref() == "disabled"),
            });
        }
    }
    for child in handle.children.borrow().iter() {
        collect_fields_recursive(child, all_elements, all_handles, fields);
    }
}

/// Find label text for a given element id.
fn find_label_text(
    target_id: &str,
    elements: &[ElementInfo],
    handles: &[Handle],
) -> Option<String> {
    for (i, info) in elements.iter().enumerate() {
        if info.tag == "label" {
            if let Some((_, v)) = info.attrs.iter().find(|(k, _)| k == "for") {
                if v == target_id {
                    let text = collect_text_content(&handles[i]).trim().to_string();
                    if !text.is_empty() {
                        return Some(text);
                    }
                }
            }
        }
    }
    None
}

/// Serialize children of a node to HTML string.
pub(crate) fn serialize_children(handle: &Handle) -> String {
    let mut buf = String::new();
    for child in handle.children.borrow().iter() {
        serialize_node(child, &mut buf);
    }
    buf
}

/// Serialize a single node to HTML string.
fn serialize_node(handle: &Handle, buf: &mut String) {
    match &handle.data {
        NodeData::Text { contents } => {
            buf.push_str(&contents.borrow());
        }
        NodeData::Element { name, attrs, .. } => {
            let tag = name.local.to_string();
            buf.push('<');
            buf.push_str(&tag);
            for attr in attrs.borrow().iter() {
                buf.push(' ');
                buf.push_str(&attr.name.local);
                buf.push_str("=\"");
                buf.push_str(&attr.value);
                buf.push('"');
            }
            buf.push('>');
            for child in handle.children.borrow().iter() {
                serialize_node(child, buf);
            }
            buf.push_str("</");
            buf.push_str(&tag);
            buf.push('>');
        }
        NodeData::Document => {
            for child in handle.children.borrow().iter() {
                serialize_node(child, buf);
            }
        }
        _ => {}
    }
}
