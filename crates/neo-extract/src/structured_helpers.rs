//! HTML parsing helpers for structured data extraction.
//!
//! Low-level functions for extracting table cells, rows, definition lists,
//! JSON-LD products, price patterns, and pagination from HTML.

use neo_dom::DomEngine;

use crate::structured::StructuredData;

/// Extract JSON-LD scripts into structured data.
pub(crate) fn extract_jsonld(dom: &dyn DomEngine, results: &mut Vec<StructuredData>) {
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
pub(crate) fn extract_prices(dom: &dyn DomEngine, results: &mut Vec<StructuredData>) {
    for el in dom.query_selector_all("span") {
        let text = dom.text_content(el);
        if let Some(price) = extract_price_from_text(&text) {
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

/// Detect pagination patterns in links.
pub(crate) fn extract_pagination(dom: &dyn DomEngine, results: &mut Vec<StructuredData>) {
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

/// Try to extract a price string from text.
fn extract_price_from_text(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.len() > 30 {
        return None;
    }

    let currency_prefixes = ["$", "\u{20ac}", "\u{00a3}", "\u{00a5}", "USD", "EUR", "GBP"];
    for prefix in &currency_prefixes {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            if looks_like_number(rest.trim()) {
                return Some(trimmed.to_string());
            }
        }
    }

    let currency_suffixes = ["\u{20ac}", "\u{00a3}", "USD", "EUR", "GBP"];
    for suffix in &currency_suffixes {
        if let Some(rest) = trimmed.strip_suffix(suffix) {
            if looks_like_number(rest.trim()) {
                return Some(trimmed.to_string());
            }
        }
    }

    None
}

/// Check if a string looks like a decimal number.
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

/// Extract cells from a row using the given cell tag (th or td).
pub(crate) fn extract_row_cells(html: &str, cell_tag: &str) -> Vec<String> {
    let open = format!("<{cell_tag}");
    let close = format!("</{cell_tag}>");
    let mut cells = Vec::new();
    let mut pos = 0;
    while let Some(start) = html[pos..].find(&open) {
        let abs_start = pos + start;
        if let Some(tag_end) = html[abs_start..].find('>') {
            let content_start = abs_start + tag_end + 1;
            let tag_text = &html[abs_start..abs_start + tag_end];
            let colspan = parse_colspan(tag_text);

            if let Some(end) = html[content_start..].find(&close) {
                let text = strip_tags(&html[content_start..content_start + end]);
                let trimmed = text.trim().to_string();
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

/// Parse colspan attribute from a tag string.
fn parse_colspan(tag_text: &str) -> usize {
    if let Some(idx) = tag_text.find("colspan") {
        let rest = &tag_text[idx..];
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
pub(crate) fn extract_table_rows(html: &str) -> Vec<Vec<String>> {
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
pub(crate) fn parse_dt_dd_pairs(html: &str) -> Vec<(String, String)> {
    let dts = extract_row_cells(html, "dt");
    let dds = extract_row_cells(html, "dd");
    dts.into_iter().zip(dds).collect()
}

/// Strip HTML tags from a string (simple regex-free approach).
pub(crate) fn strip_tags(html: &str) -> String {
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
