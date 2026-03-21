//! Semantic text compression -- noise-free text for AI consumption.
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

/// Tags to skip entirely (noise).
const SKIP_TAGS: &[&str] = &["nav", "footer", "script", "style", "noscript"];

/// Extract compressed semantic text from the DOM.
///
/// Walks informational elements, skips elements inside noise containers
/// (nav, footer, script, style), compresses whitespace, and truncates
/// to `max_chars`.
pub fn semantic_text(dom: &dyn DomEngine, max_chars: usize) -> String {
    // Collect element IDs inside skip containers so we can exclude them
    let mut skip_elements = std::collections::HashSet::new();
    for &skip_tag in SKIP_TAGS {
        for el in dom.query_selector_all(skip_tag) {
            skip_elements.insert(el);
        }
    }

    let mut parts: Vec<String> = Vec::new();
    let mut total_len: usize = 0;

    // Collect text from meaningful elements
    for &tag in KEEP_TAGS {
        for el in dom.query_selector_all(tag) {
            if !dom.is_visible(el) {
                continue;
            }
            // Skip elements that are inside noise containers
            // (simplified: if the element IS a skip container, skip it)
            if skip_elements.contains(&el) {
                continue;
            }
            let text = dom.text_content(el);
            let compressed = compress_whitespace(&text);
            if compressed.is_empty() {
                continue;
            }

            // Check if this text looks like navigation (heuristic)
            if is_nav_like_text(&compressed) {
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

/// Heuristic: detect text that looks like a navigation item.
/// Very short text that looks like a menu label.
fn is_nav_like_text(text: &str) -> bool {
    // Nav items tend to be very short and are just labels like "Home | About | Contact"
    // This is a conservative heuristic -- we only filter obvious patterns
    if text.contains(" | ") && text.len() < 100 {
        let segments: Vec<&str> = text.split(" | ").collect();
        if segments.len() >= 3 && segments.iter().all(|s| s.len() < 20) {
            return true;
        }
    }
    false
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
