//! html5ever-based DOM engine implementation.
//!
//! Parses HTML with html5ever into RcDom, then flattens the tree
//! into a list of elements for indexed access.

use html5ever::parse_document;
use html5ever::tendril::TendrilSink;
use html5ever::ParseOpts;
use markup5ever_rcdom::{Handle, Node, NodeData, RcDom};
use std::cell::RefCell;
use std::default::Default;

use neo_types::{Form, FormField, Link};

use crate::query::{collect_text_content, find_by_role, find_by_text, matches_selector};
use crate::visibility;
use crate::{DomEngine, DomError, ElementId};

/// Parsed element metadata — extracted during tree walk.
#[derive(Debug, Clone)]
pub(crate) struct ElementInfo {
    pub tag: String,
    pub attrs: Vec<(String, String)>,
}

/// DOM engine backed by html5ever + RcDom.
pub struct Html5everDom {
    dom: Option<RcDom>,
    base_url: String,
    elements: Vec<ElementInfo>,
    handles: Vec<Handle>,
}

impl Html5everDom {
    /// Create a new empty DOM engine.
    pub fn new() -> Self {
        Self {
            dom: None,
            base_url: String::new(),
            elements: Vec::new(),
            handles: Vec::new(),
        }
    }

    /// Flatten the DOM tree into indexed element lists.
    fn flatten_tree(document: &Handle, elements: &mut Vec<ElementInfo>, handles: &mut Vec<Handle>) {
        elements.clear();
        handles.clear();
        Self::walk_node(document, elements, handles);
    }

    /// Recursive tree walk — collects element nodes only.
    fn walk_node(handle: &Handle, elements: &mut Vec<ElementInfo>, handles: &mut Vec<Handle>) {
        if let NodeData::Element {
            ref name,
            ref attrs,
            ..
        } = handle.data
        {
            let tag = name.local.to_string();
            let attr_list: Vec<(String, String)> = attrs
                .borrow()
                .iter()
                .map(|a| (a.name.local.to_string(), a.value.to_string()))
                .collect();
            elements.push(ElementInfo {
                tag,
                attrs: attr_list,
            });
            handles.push(handle.clone());
        }
        for child in handle.children.borrow().iter() {
            Self::walk_node(child, elements, handles);
        }
    }
}

impl Default for Html5everDom {
    fn default() -> Self {
        Self::new()
    }
}

impl DomEngine for Html5everDom {
    fn parse_html(&mut self, html: &str, url: &str) -> Result<(), DomError> {
        let _ = url::Url::parse(url).map_err(|e| DomError::InvalidUrl(e.to_string()))?;
        self.base_url = url.to_string();

        let dom = parse_document(RcDom::default(), ParseOpts::default()).one(html);

        Self::flatten_tree(&dom.document, &mut self.elements, &mut self.handles);
        self.dom = Some(dom);
        Ok(())
    }

    fn query_selector(&self, selector: &str) -> Option<ElementId> {
        self.elements
            .iter()
            .enumerate()
            .find(|(_, info)| matches_selector(info, selector))
            .map(|(i, _)| i)
    }

    fn query_selector_all(&self, selector: &str) -> Vec<ElementId> {
        self.elements
            .iter()
            .enumerate()
            .filter(|(_, info)| matches_selector(info, selector))
            .map(|(i, _)| i)
            .collect()
    }

    fn query_by_text(&self, text: &str) -> Option<ElementId> {
        find_by_text(&self.elements, &self.handles, text)
    }

    fn query_by_role(&self, role: &str, name: Option<&str>) -> Option<ElementId> {
        find_by_role(&self.elements, &self.handles, role, name)
    }

    fn tag_name(&self, el: ElementId) -> Option<String> {
        self.elements.get(el).map(|e| e.tag.clone())
    }

    fn get_attribute(&self, el: ElementId, name: &str) -> Option<String> {
        self.elements.get(el).and_then(|info| {
            info.attrs
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        })
    }

    fn text_content(&self, el: ElementId) -> String {
        self.handles
            .get(el)
            .map(collect_text_content)
            .unwrap_or_default()
    }

    fn inner_html(&self, el: ElementId) -> String {
        self.handles
            .get(el)
            .map(serialize_children)
            .unwrap_or_default()
    }

    fn outer_html(&self) -> String {
        self.dom
            .as_ref()
            .map(|d| serialize_children(&d.document))
            .unwrap_or_default()
    }

    fn title(&self) -> String {
        if let Some(idx) = self.query_selector("title") {
            self.text_content(idx)
        } else {
            String::new()
        }
    }

    fn get_links(&self) -> Vec<Link> {
        self.query_selector_all("a")
            .into_iter()
            .map(|i| {
                let info = &self.elements[i];
                Link {
                    text: collect_text_content(&self.handles[i]).trim().to_string(),
                    href: info
                        .attrs
                        .iter()
                        .find(|(k, _)| k == "href")
                        .map(|(_, v)| v.clone())
                        .unwrap_or_default(),
                    rel: info
                        .attrs
                        .iter()
                        .find(|(k, _)| k == "rel")
                        .map(|(_, v)| v.clone()),
                }
            })
            .collect()
    }

    fn get_forms(&self) -> Vec<Form> {
        self.query_selector_all("form")
            .into_iter()
            .map(|i| {
                let info = &self.elements[i];
                let fields = collect_form_fields(&self.handles[i], &self.elements, &self.handles);
                Form {
                    id: info
                        .attrs
                        .iter()
                        .find(|(k, _)| k == "id")
                        .map(|(_, v)| v.clone()),
                    action: info
                        .attrs
                        .iter()
                        .find(|(k, _)| k == "action")
                        .map(|(_, v)| v.clone())
                        .unwrap_or_default(),
                    method: info
                        .attrs
                        .iter()
                        .find(|(k, _)| k == "method")
                        .map(|(_, v)| v.to_uppercase())
                        .unwrap_or_else(|| "GET".to_string()),
                    fields,
                }
            })
            .collect()
    }

    fn get_buttons(&self) -> Vec<ElementId> {
        self.query_selector_all("button")
    }

    fn get_inputs(&self) -> Vec<ElementId> {
        self.query_selector_all("input")
    }

    fn is_visible(&self, el: ElementId) -> bool {
        self.elements
            .get(el)
            .map(visibility::is_visible)
            .unwrap_or(false)
    }

    fn is_interactive(&self, el: ElementId) -> bool {
        self.elements
            .get(el)
            .map(visibility::is_interactive)
            .unwrap_or(false)
    }

    fn set_attribute(&mut self, el: ElementId, name: &str, value: &str) {
        if let Some(info) = self.elements.get_mut(el) {
            if let Some(attr) = info.attrs.iter_mut().find(|(k, _)| k == name) {
                attr.1 = value.to_string();
            } else {
                info.attrs.push((name.to_string(), value.to_string()));
            }
        }
    }

    fn set_text_content(&mut self, el: ElementId, text: &str) {
        if let Some(handle) = self.handles.get(el) {
            let mut children = handle.children.borrow_mut();
            children.clear();
            let text_node = Node::new(NodeData::Text {
                contents: RefCell::new(text.into()),
            });
            children.push(text_node);
        }
    }

    fn accessible_name(&self, el: ElementId) -> String {
        if let (Some(info), Some(handle)) = (self.elements.get(el), self.handles.get(el)) {
            visibility::accessible_name_from(info, handle, &self.elements, &self.handles)
        } else {
            String::new()
        }
    }
}

/// Collect form fields (input, select, textarea) from a form subtree.
fn collect_form_fields(
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
fn serialize_children(handle: &Handle) -> String {
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

// Safety: RcDom uses Rc (not Arc) so it is !Send by default.
// Html5everDom is only accessed from one thread at a time in our
// architecture. The DomEngine trait requires Send for ergonomics
// with async runtimes that may move tasks between threads, but
// actual concurrent access never happens.
unsafe impl Send for Html5everDom {}
