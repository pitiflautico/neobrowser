//! Structured data extraction from DOM.
//!
//! Detects and extracts tables, lists, key-value pairs, JSON-LD,
//! product prices, and pagination from the page.

use neo_dom::DomEngine;
use serde::{Deserialize, Serialize};

use crate::structured_helpers::{
    extract_jsonld, extract_pagination, extract_prices, extract_row_cells, extract_table_rows,
    parse_dt_dd_pairs, strip_tags,
};

/// Structured data extracted from a page.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum StructuredData {
    /// HTML table with headers and rows.
    Table {
        /// Column header names.
        headers: Vec<String>,
        /// Data rows (each row is a Vec of cell values).
        rows: Vec<Vec<String>>,
    },
    /// Ordered or unordered list items.
    List {
        /// List item text values.
        items: Vec<String>,
    },
    /// Key-value pairs (definition lists, label+value).
    KeyValue {
        /// Pairs of (key, value) extracted from the page.
        pairs: Vec<(String, String)>,
    },
    /// Product data (from JSON-LD or heuristics).
    Product {
        /// Product name.
        name: String,
        /// Price string, if found.
        price: Option<String>,
        /// Product page URL, if found.
        url: Option<String>,
    },
    /// Pagination links detected on the page.
    Pagination {
        /// Page numbers or labels found (e.g. ["1", "2", "3", "Next"]).
        pages: Vec<String>,
        /// URL of the "next" link, if found.
        next_url: Option<String>,
        /// URL of the "previous" link, if found.
        prev_url: Option<String>,
    },
}

/// Extract all structured data from the DOM.
///
/// Looks for tables, lists, definition lists, JSON-LD scripts,
/// price patterns, and pagination.
pub fn extract_structured(dom: &dyn DomEngine) -> Vec<StructuredData> {
    let mut results = Vec::new();

    extract_tables(dom, &mut results);
    extract_lists(dom, &mut results);
    extract_definition_lists(dom, &mut results);
    extract_jsonld(dom, &mut results);
    extract_prices(dom, &mut results);
    extract_pagination(dom, &mut results);

    results
}

/// Extract `<table>` elements into structured table data.
/// Handles thead/tbody and colspan/rowspan.
fn extract_tables(dom: &dyn DomEngine, results: &mut Vec<StructuredData>) {
    for table_el in dom.query_selector_all("table") {
        let html = dom.inner_html(table_el);
        if html.is_empty() {
            continue;
        }

        let headers = extract_headers_from_table(&html);
        let rows = extract_body_rows(&html);

        if !headers.is_empty() || !rows.is_empty() {
            results.push(StructuredData::Table { headers, rows });
        }
    }
}

/// Extract headers from <thead> or first <tr> with <th>.
fn extract_headers_from_table(html: &str) -> Vec<String> {
    // Try <thead> first
    if let Some(thead_start) = html.find("<thead") {
        if let Some(thead_end) = html[thead_start..].find("</thead>") {
            let thead = &html[thead_start..thead_start + thead_end];
            let headers = extract_row_cells(thead, "th");
            if !headers.is_empty() {
                return headers;
            }
        }
    }
    // Fallback: first row with <th>
    if let Some(tr_start) = html.find("<tr") {
        if let Some(tr_end) = html[tr_start..].find("</tr>") {
            let first_row = &html[tr_start..tr_start + tr_end];
            let headers = extract_row_cells(first_row, "th");
            if !headers.is_empty() {
                return headers;
            }
        }
    }
    Vec::new()
}

/// Extract body rows from <tbody> or all <tr> with <td>.
fn extract_body_rows(html: &str) -> Vec<Vec<String>> {
    // Try <tbody> first
    if let Some(tbody_start) = html.find("<tbody") {
        if let Some(tbody_end) = html[tbody_start..].find("</tbody>") {
            let tbody = &html[tbody_start..tbody_start + tbody_end];
            return extract_table_rows(tbody);
        }
    }
    // Fallback: all rows
    extract_table_rows(html)
}

/// Extract `<ul>` and `<ol>` lists.
fn extract_lists(dom: &dyn DomEngine, results: &mut Vec<StructuredData>) {
    for tag in &["ul", "ol"] {
        for list_el in dom.query_selector_all(tag) {
            let html = dom.inner_html(list_el);
            let items: Vec<String> = extract_row_cells(&html, "li")
                .into_iter()
                .map(|s| strip_tags(&s).trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if items.len() >= 2 {
                results.push(StructuredData::List { items });
            }
        }
    }
}

/// Extract `<dl>` definition lists as key-value pairs.
fn extract_definition_lists(dom: &dyn DomEngine, results: &mut Vec<StructuredData>) {
    for dl_el in dom.query_selector_all("dl") {
        let html = dom.inner_html(dl_el);
        let pairs = parse_dt_dd_pairs(&html);
        if !pairs.is_empty() {
            results.push(StructuredData::KeyValue { pairs });
        }
    }
}
