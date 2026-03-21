//! Structured data extraction from DOM.
//!
//! Detects and extracts tables, lists, key-value pairs, and JSON-LD
//! embedded data from the page.

use neo_dom::DomEngine;
use serde::{Deserialize, Serialize};

/// Structured data extracted from a page.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum StructuredData {
    /// HTML table with headers and rows.
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    /// Ordered or unordered list items.
    List { items: Vec<String> },
    /// Key-value pairs (definition lists, label+value).
    KeyValue { pairs: Vec<(String, String)> },
    /// Product data (from JSON-LD or heuristics).
    Product {
        name: String,
        price: Option<String>,
        url: Option<String>,
    },
}

/// Extract all structured data from the DOM.
///
/// Looks for tables, lists, definition lists, and JSON-LD scripts.
pub fn extract_structured(dom: &dyn DomEngine) -> Vec<StructuredData> {
    let mut results = Vec::new();

    extract_tables(dom, &mut results);
    extract_lists(dom, &mut results);
    extract_definition_lists(dom, &mut results);
    extract_jsonld(dom, &mut results);

    results
}

/// Extract `<table>` elements into structured table data.
fn extract_tables(dom: &dyn DomEngine, results: &mut Vec<StructuredData>) {
    for table_el in dom.query_selector_all("table") {
        let html = dom.inner_html(table_el);
        if html.is_empty() {
            continue;
        }
        let headers = extract_row_cells(&html, "th");
        let rows = extract_table_rows(&html);
        if !headers.is_empty() || !rows.is_empty() {
            results.push(StructuredData::Table { headers, rows });
        }
    }
}

/// Extract `<ul>` and `<ol>` into list data.
fn extract_lists(dom: &dyn DomEngine, results: &mut Vec<StructuredData>) {
    for list_tag in &["ul", "ol"] {
        for list_el in dom.query_selector_all(list_tag) {
            let text = dom.text_content(list_el);
            let items: Vec<String> = text
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect();
            if !items.is_empty() {
                results.push(StructuredData::List { items });
            }
        }
    }
}

/// Extract `<dl>` definition lists into key-value pairs.
fn extract_definition_lists(dom: &dyn DomEngine, results: &mut Vec<StructuredData>) {
    for dl_el in dom.query_selector_all("dl") {
        let html = dom.inner_html(dl_el);
        if html.is_empty() {
            continue;
        }
        let pairs = parse_dt_dd_pairs(&html);
        if !pairs.is_empty() {
            results.push(StructuredData::KeyValue { pairs });
        }
    }
}

/// Extract JSON-LD scripts into structured data.
fn extract_jsonld(dom: &dyn DomEngine, results: &mut Vec<StructuredData>) {
    for script_el in dom.query_selector_all("script") {
        let script_type = dom.get_attribute(script_el, "type");
        if script_type.as_deref() != Some("application/ld+json") {
            continue;
        }
        let text = dom.text_content(script_el);
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(product) = parse_jsonld_product(&json) {
                results.push(product);
            }
        }
    }
}

/// Parse a JSON-LD value looking for Product schema.
fn parse_jsonld_product(json: &serde_json::Value) -> Option<StructuredData> {
    let obj_type = json.get("@type")?.as_str()?;
    if obj_type != "Product" {
        return None;
    }
    let name = json.get("name")?.as_str()?.to_string();
    let price = json
        .get("offers")
        .and_then(|o| o.get("price"))
        .and_then(|p| {
            p.as_str()
                .or_else(|| p.as_f64().map(|_| ""))
                .map(|_| p.to_string())
        });
    let url = json.get("url").and_then(|u| u.as_str()).map(String::from);
    Some(StructuredData::Product { name, price, url })
}

/// Simple HTML parser: extract cells from first row of `<th>` tags.
fn extract_row_cells(html: &str, cell_tag: &str) -> Vec<String> {
    let open = format!("<{cell_tag}");
    let close = format!("</{cell_tag}>");
    let mut cells = Vec::new();
    let mut pos = 0;
    while let Some(start) = html[pos..].find(&open) {
        let abs_start = pos + start;
        // Find end of opening tag
        if let Some(tag_end) = html[abs_start..].find('>') {
            let content_start = abs_start + tag_end + 1;
            if let Some(end) = html[content_start..].find(&close) {
                let text = strip_tags(&html[content_start..content_start + end]);
                cells.push(text.trim().to_string());
                pos = content_start + end + close.len();
            } else {
                break;
            }
        } else {
            break;
        }
    }
    cells
}

/// Extract all `<tr>` rows, returning cells from `<td>` tags.
fn extract_table_rows(html: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut pos = 0;
    while let Some(tr_start) = html[pos..].find("<tr") {
        let abs_start = pos + tr_start;
        if let Some(tr_end) = html[abs_start..].find("</tr>") {
            let row_html = &html[abs_start..abs_start + tr_end];
            let cells = extract_row_cells(row_html, "td");
            if !cells.is_empty() {
                rows.push(cells);
            }
            pos = abs_start + tr_end + 5;
        } else {
            break;
        }
    }
    rows
}

/// Parse `<dt>`/`<dd>` pairs from definition list HTML.
fn parse_dt_dd_pairs(html: &str) -> Vec<(String, String)> {
    let dts = extract_row_cells(html, "dt");
    let dds = extract_row_cells(html, "dd");
    dts.into_iter().zip(dds).collect()
}

/// Strip HTML tags from a string (simple regex-free approach).
fn strip_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut inside_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => inside_tag = true,
            '>' => inside_tag = false,
            _ if !inside_tag => result.push(ch),
            _ => {}
        }
    }
    result
}
