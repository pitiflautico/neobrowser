//! Page classification -- determines what kind of page the AI is looking at.
//!
//! Uses heuristic signals (element counts, tag presence, input types)
//! to classify pages with a confidence score. Each signal is weighted
//! and combined for better accuracy.

use neo_dom::DomEngine;
use serde::{Deserialize, Serialize};

use crate::classify_signals::{
    check_documentation, check_pricing_page, check_profile, check_settings, has_price_pattern,
};

/// Classification result with confidence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PageClassification {
    /// Detected page type.
    pub page_type: PageType,
    /// Confidence score 0.0..1.0.
    pub confidence: f32,
    /// Signals that led to this classification.
    pub features: Vec<String>,
}

/// Known page types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PageType {
    /// Search engine results page.
    SearchResults,
    /// Long-form article or blog post.
    Article,
    /// E-commerce product detail page.
    ProductPage,
    /// Authentication / login form.
    LoginForm,
    /// Dashboard or admin panel.
    Dashboard,
    /// Data-heavy table page.
    DataTable,
    /// Site homepage.
    Homepage,
    /// Error page (404, 500, etc.).
    Error,
    /// Pricing / plans page.
    Pricing,
    /// Technical documentation.
    Documentation,
    /// User profile page.
    Profile,
    /// Settings / preferences page.
    Settings,
    /// Could not determine page type.
    Unknown,
}

/// Classify the page type from DOM signals.
pub fn classify(dom: &dyn DomEngine) -> PageClassification {
    let mut candidates: Vec<(PageType, f32, Vec<String>)> = Vec::new();

    check_login_form(dom, &mut candidates);
    check_search_results(dom, &mut candidates);
    check_article(dom, &mut candidates);
    check_data_table(dom, &mut candidates);
    check_product_page(dom, &mut candidates);
    check_error_page(dom, &mut candidates);
    check_pricing_page(dom, &mut candidates);
    check_documentation(dom, &mut candidates);
    check_profile(dom, &mut candidates);
    check_settings(dom, &mut candidates);

    // Pick highest confidence, default to Unknown
    candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    candidates
        .into_iter()
        .next()
        .map(|(pt, conf, feats)| PageClassification {
            page_type: pt,
            confidence: conf,
            features: feats,
        })
        .unwrap_or(PageClassification {
            page_type: PageType::Unknown,
            confidence: 0.0,
            features: vec![],
        })
}

/// Login form: has a password input.
fn check_login_form(dom: &dyn DomEngine, candidates: &mut Vec<(PageType, f32, Vec<String>)>) {
    let mut features = Vec::new();
    let mut score: f32 = 0.0;

    let inputs = dom.get_inputs();
    let has_password = inputs
        .iter()
        .any(|&el| dom.get_attribute(el, "type").as_deref() == Some("password"));

    if has_password {
        score += 0.6;
        features.push("password input found".to_string());
    }

    if !dom.get_forms().is_empty() && has_password {
        score += 0.2;
        features.push("form with password".to_string());
    }

    if score > 0.0 {
        candidates.push((PageType::LoginForm, score.min(1.0), features));
    }
}

/// Search results: search input + repeated result items.
fn check_search_results(dom: &dyn DomEngine, candidates: &mut Vec<(PageType, f32, Vec<String>)>) {
    let mut features = Vec::new();
    let mut score: f32 = 0.0;

    let inputs = dom.get_inputs();
    let has_search = inputs.iter().any(|&el| {
        dom.get_attribute(el, "type").as_deref() == Some("search")
            || dom.get_attribute(el, "role").as_deref() == Some("search")
    });

    if has_search {
        score += 0.4;
        features.push("search input found".to_string());
    }

    let links = dom.get_links();
    if links.len() > 10 {
        score += 0.3;
        features.push(format!("{} links (many results)", links.len()));
    }

    if score > 0.0 {
        candidates.push((PageType::SearchResults, score.min(1.0), features));
    }
}

/// Article: has `<article>` tag or long text blocks.
fn check_article(dom: &dyn DomEngine, candidates: &mut Vec<(PageType, f32, Vec<String>)>) {
    let mut features = Vec::new();
    let mut score: f32 = 0.0;

    let articles = dom.query_selector_all("article");
    if !articles.is_empty() {
        score += 0.6;
        features.push(format!("{} article elements", articles.len()));
    }

    let paragraphs = dom.query_selector_all("p");
    if paragraphs.len() > 3 {
        score += 0.2;
        features.push(format!("{} paragraphs", paragraphs.len()));
    }

    if score > 0.0 {
        candidates.push((PageType::Article, score.min(1.0), features));
    }
}

/// Data table: has `<table>` with more than 3 rows.
fn check_data_table(dom: &dyn DomEngine, candidates: &mut Vec<(PageType, f32, Vec<String>)>) {
    let mut features = Vec::new();
    let mut score: f32 = 0.0;

    let tables = dom.query_selector_all("table");
    if !tables.is_empty() {
        score += 0.3;
        features.push(format!("{} tables", tables.len()));

        for &table_el in &tables {
            let html = dom.inner_html(table_el);
            let row_count = html.matches("<tr").count();
            if row_count > 3 {
                score += 0.4;
                features.push(format!("table with {} rows", row_count));
                break;
            }
        }
    }

    if score > 0.0 {
        candidates.push((PageType::DataTable, score.min(1.0), features));
    }
}

/// Product page: JSON-LD Product or price-like elements.
fn check_product_page(dom: &dyn DomEngine, candidates: &mut Vec<(PageType, f32, Vec<String>)>) {
    let mut features = Vec::new();
    let mut score: f32 = 0.0;

    for el in dom.query_selector_all("script") {
        if dom.get_attribute(el, "type").as_deref() == Some("application/ld+json") {
            let text = dom.text_content(el);
            if text.contains("\"Product\"") {
                score += 0.7;
                features.push("JSON-LD Product schema".to_string());
            }
        }
    }

    let body_text = dom
        .query_selector("body")
        .map(|el| dom.text_content(el))
        .unwrap_or_default();
    if has_price_pattern(&body_text) && !features.is_empty() {
        score += 0.1;
        features.push("price pattern in text".to_string());
    }

    if score > 0.0 {
        candidates.push((PageType::ProductPage, score.min(1.0), features));
    }
}

/// Error page: title or heading containing error keywords.
fn check_error_page(dom: &dyn DomEngine, candidates: &mut Vec<(PageType, f32, Vec<String>)>) {
    let title = dom.title().to_lowercase();
    let error_keywords = ["404", "not found", "error", "500", "403"];

    let mut features = Vec::new();
    let mut score: f32 = 0.0;

    for kw in &error_keywords {
        if title.contains(kw) {
            score += 0.7;
            features.push(format!("title contains '{kw}'"));
        }
    }

    if score > 0.0 {
        candidates.push((PageType::Error, score.min(1.0), features));
    }
}
