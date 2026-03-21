//! neo-extract — transforms DOM into structured data an AI can reason about.
//!
//! The AI never sees raw HTML. Instead it gets:
//! - **WOM** (Web Object Model): action map with semantic labels
//! - **Structured data**: tables, products, search results
//! - **Page classification**: article, form, search results, SPA
//! - **Delta**: what changed since last observation

pub mod classify;
pub mod delta;
mod mock;
pub mod semantic;
pub mod structured;
pub mod wom;

pub use classify::{PageClassification, PageType};
pub use delta::WomDelta;
pub use mock::MockExtractor;
pub use structured::StructuredData;
pub use wom::{WomDocument, WomNode};

use neo_dom::DomEngine;

/// Extractor trait — the interface for turning DOM into AI-consumable data.
pub trait Extractor {
    /// Extract WOM — the action map an AI uses to understand and interact.
    fn extract_wom(&self, dom: &dyn DomEngine) -> WomDocument;

    /// Extract structured data (tables, lists, products).
    fn extract_structured(&self, dom: &dyn DomEngine) -> Vec<StructuredData>;

    /// Classify the page type.
    fn classify(&self, dom: &dyn DomEngine) -> PageClassification;

    /// Compute delta between two WOM snapshots.
    fn delta(&self, before: &WomDocument, after: &WomDocument) -> WomDelta;

    /// Semantic text — compressed, noise-free text for AI consumption.
    fn semantic_text(&self, dom: &dyn DomEngine, max_chars: usize) -> String;
}
