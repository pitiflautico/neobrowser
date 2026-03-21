//! Mock extractor for testing downstream consumers.
//!
//! Returns configurable pre-built results without touching the DOM.

use neo_dom::DomEngine;

use crate::classify::{PageClassification, PageType};
use crate::delta::WomDelta;
use crate::structured::StructuredData;
use crate::wom::WomDocument;
use crate::Extractor;

/// Mock extractor — returns pre-configured results.
pub struct MockExtractor {
    /// WOM to return from `extract_wom`.
    pub wom: Option<WomDocument>,
    /// Structured data to return.
    pub structured: Vec<StructuredData>,
    /// Page classification to return.
    pub classification: Option<PageClassification>,
}

impl MockExtractor {
    /// Create a new mock extractor with empty defaults.
    pub fn new() -> Self {
        Self {
            wom: None,
            structured: Vec::new(),
            classification: None,
        }
    }
}

impl Default for MockExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl Extractor for MockExtractor {
    fn extract_wom(&self, _dom: &dyn DomEngine) -> WomDocument {
        self.wom.clone().unwrap_or_else(|| WomDocument {
            url: String::new(),
            title: String::new(),
            nodes: Vec::new(),
            page_type: "unknown".to_string(),
            summary: String::new(),
        })
    }

    fn extract_structured(&self, _dom: &dyn DomEngine) -> Vec<StructuredData> {
        self.structured.clone()
    }

    fn classify(&self, _dom: &dyn DomEngine) -> PageClassification {
        self.classification.clone().unwrap_or(PageClassification {
            page_type: PageType::Unknown,
            confidence: 0.0,
            features: Vec::new(),
        })
    }

    fn delta(&self, before: &WomDocument, after: &WomDocument) -> WomDelta {
        crate::delta::compute_delta(before, after)
    }

    fn semantic_text(&self, _dom: &dyn DomEngine, _max_chars: usize) -> String {
        String::new()
    }
}
