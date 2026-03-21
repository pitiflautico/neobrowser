//! Semantic text compression — noise-free text for AI consumption.
//!
//! Strips navigation, footers, ads, and tracking elements.
//! Keeps headings, paragraphs, list items, and form labels.

use neo_dom::DomEngine;

/// Tags whose content should be kept in semantic text.
const KEEP_TAGS: &[&str] = &[
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "p",
    "li",
    "label",
    "td",
    "th",
    "blockquote",
    "figcaption",
    "summary",
    "dt",
    "dd",
];

/// Tags to skip entirely (noise). Reserved for future parent-aware filtering.
const _SKIP_TAGS: &[&str] = &["nav", "footer", "script", "style", "noscript"];

/// Extract compressed semantic text from the DOM.
///
/// Walks informational elements, compresses whitespace, and truncates
/// to `max_chars`.
pub fn semantic_text(dom: &dyn DomEngine, max_chars: usize) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut total_len: usize = 0;

    // Collect text from meaningful elements
    for &tag in KEEP_TAGS {
        for el in dom.query_selector_all(tag) {
            if !dom.is_visible(el) {
                continue;
            }
            let text = dom.text_content(el);
            let compressed = compress_whitespace(&text);
            if compressed.is_empty() {
                continue;
            }

            // Prefix headings for structure
            let line = if tag.starts_with('h') {
                format!("## {compressed}")
            } else {
                compressed
            };

            let line_len = line.len();
            if total_len + line_len > max_chars {
                // Truncate last piece to fit
                let remaining = max_chars.saturating_sub(total_len);
                if remaining > 10 {
                    parts.push(truncate_str(&line, remaining));
                }
                break;
            }

            total_len += line_len + 1; // +1 for newline
            parts.push(line);
        }

        if total_len >= max_chars {
            break;
        }
    }

    parts.join("\n")
}

/// Compress consecutive whitespace into single spaces, trim.
fn compress_whitespace(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut last_was_space = true; // trim leading
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !last_was_space {
                result.push(' ');
                last_was_space = true;
            }
        } else {
            result.push(ch);
            last_was_space = false;
        }
    }
    // Trim trailing space
    if result.ends_with(' ') {
        result.pop();
    }
    result
}

/// Truncate a string to at most `max_len` bytes on a char boundary.
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        return s.to_string();
    }
    let mut end = max_len;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &s[..end])
}
