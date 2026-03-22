//! Mock DOM engine for testing.
//!
//! Configurable mock that returns pre-set elements and attributes
//! without parsing real HTML.

use neo_types::{Form, Link};

use crate::{DomEngine, DomError, ElementId};

/// A mock element for testing.
#[derive(Debug, Clone)]
struct MockElement {
    tag: String,
    attrs: Vec<(String, String)>,
    text: String,
    visible: bool,
    interactive: bool,
}

/// Mock DOM engine — returns configured elements.
///
/// Use `add_element` to populate, then query as normal.
pub struct MockDomEngine {
    elements: Vec<MockElement>,
    title: String,
    links: Vec<Link>,
    forms: Vec<Form>,
    parsed: bool,
    children_map: std::collections::HashMap<ElementId, Vec<ElementId>>,
}

impl MockDomEngine {
    /// Create a new empty mock engine.
    pub fn new() -> Self {
        Self {
            elements: Vec::new(),
            title: String::new(),
            links: Vec::new(),
            forms: Vec::new(),
            parsed: false,
            children_map: std::collections::HashMap::new(),
        }
    }

    /// Add an element to the mock DOM. Returns its ElementId.
    pub fn add_element(&mut self, tag: &str, attrs: &[(&str, &str)], text: &str) -> ElementId {
        let id = self.elements.len();
        self.elements.push(MockElement {
            tag: tag.to_string(),
            attrs: attrs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            text: text.to_string(),
            visible: true,
            interactive: false,
        });
        id
    }

    /// Set visibility for an element.
    pub fn set_visible(&mut self, el: ElementId, visible: bool) {
        if let Some(e) = self.elements.get_mut(el) {
            e.visible = visible;
        }
    }

    /// Set interactivity for an element.
    pub fn set_interactive(&mut self, el: ElementId, interactive: bool) {
        if let Some(e) = self.elements.get_mut(el) {
            e.interactive = interactive;
        }
    }

    /// Set the document title.
    pub fn set_title(&mut self, title: &str) {
        self.title = title.to_string();
    }

    /// Add a link.
    pub fn add_link(&mut self, text: &str, href: &str) {
        self.links.push(Link {
            text: text.to_string(),
            href: href.to_string(),
            rel: None,
        });
    }

    /// Add a parent-child relationship.
    pub fn add_child(&mut self, parent: ElementId, child: ElementId) {
        self.children_map.entry(parent).or_default().push(child);
    }

    /// Add a form.
    pub fn add_form(&mut self, id: Option<&str>, action: &str) {
        self.forms.push(Form {
            id: id.map(|s| s.to_string()),
            action: action.to_string(),
            method: "POST".to_string(),
            fields: Vec::new(),
        });
    }
}

impl Default for MockDomEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl DomEngine for MockDomEngine {
    fn parse_html(&mut self, _html: &str, _url: &str) -> Result<(), DomError> {
        self.parsed = true;
        Ok(())
    }

    fn query_selector(&self, selector: &str) -> Option<ElementId> {
        self.elements
            .iter()
            .enumerate()
            .find(|(_, e)| e.tag == selector)
            .map(|(i, _)| i)
    }

    fn query_selector_all(&self, selector: &str) -> Vec<ElementId> {
        self.elements
            .iter()
            .enumerate()
            .filter(|(_, e)| e.tag == selector)
            .map(|(i, _)| i)
            .collect()
    }

    fn query_by_text(&self, text: &str) -> Option<ElementId> {
        let lower = text.to_lowercase();
        self.elements
            .iter()
            .enumerate()
            .find(|(_, e)| e.text.to_lowercase().contains(&lower))
            .map(|(i, _)| i)
    }

    fn query_by_role(&self, role: &str, _name: Option<&str>) -> Option<ElementId> {
        self.elements.iter().enumerate().find_map(|(i, e)| {
            let el_role = e
                .attrs
                .iter()
                .find(|(k, _)| k == "role")
                .map(|(_, v)| v.as_str());
            if el_role == Some(role) {
                Some(i)
            } else {
                None
            }
        })
    }

    fn tag_name(&self, el: ElementId) -> Option<String> {
        self.elements.get(el).map(|e| e.tag.clone())
    }

    fn get_attribute(&self, el: ElementId, name: &str) -> Option<String> {
        self.elements.get(el).and_then(|e| {
            e.attrs
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        })
    }

    fn text_content(&self, el: ElementId) -> String {
        self.elements
            .get(el)
            .map(|e| e.text.clone())
            .unwrap_or_default()
    }

    fn inner_html(&self, _el: ElementId) -> String {
        String::new()
    }

    fn outer_html(&self) -> String {
        String::new()
    }

    fn title(&self) -> String {
        self.title.clone()
    }

    fn get_links(&self) -> Vec<Link> {
        self.links.clone()
    }

    fn get_forms(&self) -> Vec<Form> {
        self.forms.clone()
    }

    fn get_buttons(&self) -> Vec<ElementId> {
        self.query_selector_all("button")
    }

    fn get_inputs(&self) -> Vec<ElementId> {
        self.query_selector_all("input")
    }

    fn is_visible(&self, el: ElementId) -> bool {
        self.elements.get(el).map(|e| e.visible).unwrap_or(false)
    }

    fn is_interactive(&self, el: ElementId) -> bool {
        self.elements
            .get(el)
            .map(|e| e.interactive)
            .unwrap_or(false)
    }

    fn set_attribute(&mut self, el: ElementId, name: &str, value: &str) {
        if let Some(e) = self.elements.get_mut(el) {
            if let Some(attr) = e.attrs.iter_mut().find(|(k, _)| k == name) {
                attr.1 = value.to_string();
            } else {
                e.attrs.push((name.to_string(), value.to_string()));
            }
        }
    }

    fn set_text_content(&mut self, el: ElementId, text: &str) {
        if let Some(e) = self.elements.get_mut(el) {
            e.text = text.to_string();
        }
    }

    fn accessible_name(&self, el: ElementId) -> String {
        if let Some(e) = self.elements.get(el) {
            // Simplified: aria-label > text
            e.attrs
                .iter()
                .find(|(k, _)| k == "aria-label")
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| e.text.clone())
        } else {
            String::new()
        }
    }

    fn element_count(&self) -> usize {
        self.elements.len()
    }

    fn children(&self, el: ElementId) -> Vec<ElementId> {
        self.children_map.get(&el).cloned().unwrap_or_default()
    }

    fn get_attributes(&self, el: ElementId) -> Vec<(String, String)> {
        self.elements
            .get(el)
            .map(|e| e.attrs.clone())
            .unwrap_or_default()
    }
}
