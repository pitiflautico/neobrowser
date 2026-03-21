//! neo-extract — transforms DOM into structured data an AI can reason about.
//!
//! The AI never sees raw HTML. Instead it gets:
//! - **WOM** (Web Object Model): action map with semantic labels
//! - **Structured data**: tables, products, search results
//! - **Page classification**: article, form, search results, SPA
//! - **Delta**: what changed since last observation

pub mod classify;
mod classify_signals;
pub mod delta;
mod mock;
pub mod semantic;
pub mod structured;
pub mod wom;
mod wom_builder;

pub use classify::{PageClassification, PageType};
pub use delta::WomDelta;
pub use mock::MockExtractor;
pub use structured::StructuredData;
pub use wom::{WomDocument, WomNode};

use neo_dom::DomEngine;

/// Default extractor — uses the real WOM builder, classifier, and structured extractors.
pub struct DefaultExtractor;

impl DefaultExtractor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for DefaultExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl Extractor for DefaultExtractor {
    fn extract_wom(&self, dom: &dyn DomEngine) -> WomDocument {
        let mut wom = wom::build_wom(dom, "");
        // Enrich with page classification.
        let classification = classify::classify(dom);
        wom.page_type = format!("{:?}", classification.page_type);
        wom
    }

    fn extract_structured(&self, dom: &dyn DomEngine) -> Vec<StructuredData> {
        structured::extract_structured(dom)
    }

    fn classify(&self, dom: &dyn DomEngine) -> PageClassification {
        classify::classify(dom)
    }

    fn delta(&self, before: &WomDocument, after: &WomDocument) -> WomDelta {
        delta::compute_delta(before, after)
    }

    fn semantic_text(&self, dom: &dyn DomEngine, max_chars: usize) -> String {
        semantic::semantic_text(dom, max_chars)
    }
}

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
