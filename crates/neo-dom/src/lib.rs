//! neo-dom — DOM engine trait and html5ever implementation.
//!
//! Provides a trait-based abstraction over DOM parsing and querying.
//! The AI sees elements, attributes, text, forms, links, and actions —
//! not pixels.

mod dom_forms;
mod html5ever_dom;
mod mock;
mod query;
mod visibility;

pub use html5ever_dom::Html5everDom;
pub use mock::MockDomEngine;

use neo_types::{Form, Link};
use thiserror::Error;

/// Opaque element identifier — index into the flat element list.
pub type ElementId = usize;

/// Errors from DOM operations.
#[derive(Debug, Error)]
pub enum DomError {
    /// HTML parsing failed.
    #[error("parse error: {0}")]
    Parse(String),

    /// Invalid URL.
    #[error("invalid url: {0}")]
    InvalidUrl(String),

    /// Element not found for the given id.
    #[error("element not found: {0}")]
    ElementNotFound(ElementId),
}

/// DOM engine trait — the interface AI uses to see a web page.
///
/// Implementations wrap a DOM tree and expose query, read, and
/// mutation operations. No rendering or layout computation.
pub trait DomEngine: Send {
    // -- Parse --

    /// Parse HTML string and set it as the current document.
    fn parse_html(&mut self, html: &str, url: &str) -> Result<(), DomError>;

    // -- Query --

    /// Find first element matching a CSS selector.
    fn query_selector(&self, selector: &str) -> Option<ElementId>;

    /// Find all elements matching a CSS selector.
    fn query_selector_all(&self, selector: &str) -> Vec<ElementId>;

    /// Find first element whose text content contains `text` (case-insensitive).
    fn query_by_text(&self, text: &str) -> Option<ElementId>;

    /// Find first element matching an ARIA role and optional name.
    fn query_by_role(&self, role: &str, name: Option<&str>) -> Option<ElementId>;

    // -- Read --

    /// Tag name of the element (lowercase).
    fn tag_name(&self, el: ElementId) -> Option<String>;

    /// Get an attribute value.
    fn get_attribute(&self, el: ElementId, name: &str) -> Option<String>;

    /// Text content of the element and its descendants.
    fn text_content(&self, el: ElementId) -> String;

    /// Inner HTML of the element.
    fn inner_html(&self, el: ElementId) -> String;

    /// Outer HTML of the entire document.
    fn outer_html(&self) -> String;

    // -- Structure --

    /// Document title.
    fn title(&self) -> String;

    /// All links in the document.
    fn get_links(&self) -> Vec<Link>;

    /// All forms in the document.
    fn get_forms(&self) -> Vec<Form>;

    /// All button elements.
    fn get_buttons(&self) -> Vec<ElementId>;

    /// All input elements.
    fn get_inputs(&self) -> Vec<ElementId>;

    // -- Visibility heuristic --

    /// Whether the element is visible (heuristic, no layout).
    fn is_visible(&self, el: ElementId) -> bool;

    /// Whether the element is interactive (input, button, link, etc.).
    fn is_interactive(&self, el: ElementId) -> bool;

    // -- Mutation --

    /// Set an attribute on an element.
    fn set_attribute(&mut self, el: ElementId, name: &str, value: &str);

    /// Set the text content of an element.
    fn set_text_content(&mut self, el: ElementId, text: &str);

    // -- Accessibility --

    /// Compute the accessible name for an element.
    fn accessible_name(&self, el: ElementId) -> String;

    // -- Tree structure --

    /// Number of elements in the flattened DOM.
    fn element_count(&self) -> usize {
        0
    }

    /// Direct child element IDs of the given element.
    fn children(&self, _el: ElementId) -> Vec<ElementId> {
        Vec::new()
    }

    /// Get all attributes for an element as (name, value) pairs.
    fn get_attributes(&self, _el: ElementId) -> Vec<(String, String)> {
        Vec::new()
    }

    /// Find the body element (entry point for tree building).
    fn body(&self) -> Option<ElementId> {
        self.query_selector("body")
    }
}
