//! Structured data extraction from DOM.
//!
//! Detects and extracts tables, lists, key-value pairs, JSON-LD,
//! product prices, and pagination from the page.

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

/// Extract headers: prefer thead > th, fall back to first row th.
fn extract_headers_from_table(html: &str) -> Vec<String> {
    // Try thead first
    if let Some(thead_start) = html.find("<thead") {
        if let Some(thead_end) = html[thead_start..].find("</thead>") {
            let thead_html = &html[thead_start..thead_start + thead_end];
            let headers = extract_row_cells(thead_html, "th");
            if !headers.is_empty() {
                return headers;
            }
        }
    }
    // Fall back to first row th
    extract_row_cells(html, "th")
}

/// Extract body rows: prefer tbody > tr > td, fall back to tr > td.
fn extract_body_rows(html: &str) -> Vec<Vec<String>> {
    // Try tbody first
    if let Some(tbody_start) = html.find("<tbody") {
        if let Some(tbody_end) = html[tbody_start..].find("</tbody>") {
            let tbody_html = &html[tbody_start..tbody_start + tbody_end];
            let rows = extract_table_rows(tbody_html);
            if !rows.is_empty() {
                return rows;
            }
        }
    }
    // Fall back to all tr > td
    extract_table_rows(html)
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

/// Detect price patterns in text content (heuristic product detection).
///
/// Looks for currency symbols ($, EUR, GBP, JPY) followed by numbers.
fn extract_prices(dom: &dyn DomEngine, results: &mut Vec<StructuredData>) {
    // Look for elements with price-related attributes or classes
    for el in dom.query_selector_all("span") {
        let text = dom.text_content(el);
        if let Some(price) = extract_price_from_text(&text) {
            // Check if there's a nearby product name (parent text or aria-label)
            let label = dom.accessible_name(el);
            if !label.is_empty() && label != text {
                results.push(StructuredData::Product {
                    name: label,
                    price: Some(price),
                    url: None,
                });
            }
        }
    }
}

/// Try to extract a price string from text.
/// Matches patterns like $19.99, EUR29.99, 19,99, etc.
fn extract_price_from_text(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.len() > 30 {
        return None;
    }

    let currency_prefixes = ["$", "\u{20ac}", "\u{00a3}", "\u{00a5}", "USD", "EUR", "GBP"];
    for prefix in &currency_prefixes {
        if trimmed.starts_with(prefix) {
            let rest = trimmed[prefix.len()..].trim();
            if looks_like_number(rest) {
                return Some(trimmed.to_string());
            }
        }
    }

    // Also check for number followed by currency
    let currency_suffixes = ["\u{20ac}", "\u{00a3}", "USD", "EUR", "GBP"];
    for suffix in &currency_suffixes {
        if trimmed.ends_with(suffix) {
            let rest = trimmed[..trimmed.len() - suffix.len()].trim();
            if looks_like_number(rest) {
                return Some(trimmed.to_string());
            }
        }
    }

    None
}

/// Check if a string looks like a decimal number (with . or , separators).
fn looks_like_number(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let mut has_digit = false;
    for ch in s.chars() {
        match ch {
            '0'..='9' => has_digit = true,
            '.' | ',' | ' ' => {}
            _ => return false,
        }
    }
    has_digit
}

/// Detect pagination patterns in links.
fn extract_pagination(dom: &dyn DomEngine, results: &mut Vec<StructuredData>) {
    let links = dom.get_links();
    if links.is_empty() {
        return;
    }

    let mut pages = Vec::new();
    let mut next_url = None;
    let mut prev_url = None;

    let next_keywords = ["next", "siguiente", "suivant", ">>", "\u{203a}", "\u{00bb}"];
    let prev_keywords = [
        "prev",
        "previous",
        "anterior",
        "pr\u{00e9}c\u{00e9}dent",
        "<<",
        "\u{2039}",
        "\u{00ab}",
    ];

    for link in &links {
        let text_lower = link.text.to_lowercase().trim().to_string();

        // Check for next/prev
        for kw in &next_keywords {
            if text_lower.contains(kw) {
                next_url = Some(link.href.clone());
                pages.push(link.text.trim().to_string());
                break;
            }
        }
        for kw in &prev_keywords {
            if text_lower.contains(kw) {
                prev_url = Some(link.href.clone());
                pages.push(link.text.trim().to_string());
                break;
            }
        }

        // Check for page numbers
        let trimmed = link.text.trim();
        if !trimmed.is_empty() && trimmed.len() <= 5 && trimmed.chars().all(|c| c.is_ascii_digit())
        {
            pages.push(trimmed.to_string());
        }
    }

    if !pages.is_empty() || next_url.is_some() || prev_url.is_some() {
        results.push(StructuredData::Pagination {
            pages,
            next_url,
            prev_url,
        });
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
            // Handle colspan attribute
            let tag_text = &html[abs_start..abs_start + tag_end];
            let colspan = parse_colspan(tag_text);

            if let Some(end) = html[content_start..].find(&close) {
                let text = strip_tags(&html[content_start..content_start + end]);
                let trimmed = text.trim().to_string();
                // Push the cell, and empty cells for colspan > 1
                cells.push(trimmed.clone());
                for _ in 1..colspan {
                    cells.push(trimmed.clone());
                }
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

/// Parse colspan attribute from a tag string like `<td colspan="3"`.
fn parse_colspan(tag_text: &str) -> usize {
    if let Some(idx) = tag_text.find("colspan") {
        let rest = &tag_text[idx..];
        // Find the value after =
        if let Some(eq) = rest.find('=') {
            let val_start = &rest[eq + 1..].trim_start();
            let val = val_start.trim_start_matches('"').trim_start_matches('\'');
            if let Some(end) = val.find(|c: char| !c.is_ascii_digit()) {
                if let Ok(n) = val[..end].parse::<usize>() {
                    return n.max(1);
                }
            } else if let Ok(n) = val
                .trim_end_matches('"')
                .trim_end_matches('\'')
                .parse::<usize>()
            {
                return n.max(1);
            }
        }
    }
    1
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
