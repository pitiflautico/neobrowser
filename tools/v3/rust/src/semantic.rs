//! Semantic extraction — HTML DOM → structured text for AI.
//!
//! Takes an html5ever DOM tree and produces compact, structured output
//! that an AI can reason about without needing pixels.

use html5ever::LocalName;
use markup5ever_rcdom::{Handle, NodeData};

/// Semantic role of an HTML element.
pub fn semantic_role(tag: &LocalName) -> Option<&'static str> {
    match &**tag {
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => Some("heading"),
        "p" => Some("paragraph"),
        "a" => Some("link"),
        "button" => Some("button"),
        "input" => Some("textbox"),
        "textarea" => Some("textbox"),
        "select" => Some("combobox"),
        "img" => Some("image"),
        "nav" => Some("navigation"),
        "main" => Some("main"),
        "article" => Some("article"),
        "section" => Some("section"),
        "form" => Some("form"),
        "table" => Some("table"),
        "li" => Some("listitem"),
        "ul" | "ol" => Some("list"),
        "header" => Some("banner"),
        "footer" => Some("contentinfo"),
        _ => None,
    }
}

pub fn is_hidden_tag(tag: &LocalName) -> bool {
    matches!(
        &**tag,
        "script" | "style" | "noscript" | "svg" | "path" | "meta" | "link" | "head"
        | "defs" | "clippath" | "lineargradient" | "radialgradient"
    )
}

fn heading_level(tag: &LocalName) -> u8 {
    match &**tag {
        "h1" => 1,
        "h2" => 2,
        "h3" => 3,
        "h4" => 4,
        "h5" => 5,
        "h6" => 6,
        _ => 1,
    }
}

/// Check if text looks like CSS, JS, or other noise.
fn is_noise(text: &str) -> bool {
    // CSS-like content
    if text.contains('{') && text.contains('}') && text.contains(':') {
        return true;
    }
    // Base64 data
    if text.starts_with("data:") {
        return true;
    }
    // Very long strings with no spaces are likely encoded/minified
    if text.len() > 200 && !text.contains(' ') {
        return true;
    }
    false
}

/// Extract all text content from a node and its children.
pub fn extract_text(handle: &Handle) -> String {
    let mut text = String::new();
    if let NodeData::Text { contents } = &handle.data {
        text.push_str(&contents.borrow());
    }
    for child in handle.children.borrow().iter() {
        text.push_str(&extract_text(child));
    }
    text
}

/// Get an attribute value from an element node.
pub fn get_attr(handle: &Handle, name: &str) -> Option<String> {
    if let NodeData::Element { attrs, .. } = &handle.data {
        for attr in attrs.borrow().iter() {
            if &*attr.name.local == name {
                return Some(attr.value.to_string());
            }
        }
    }
    None
}

/// Walk the DOM tree and produce semantic output lines.
pub fn walk(handle: &Handle, depth: usize, output: &mut Vec<String>) {
    match &handle.data {
        NodeData::Element { name, .. } => {
            let tag = &name.local;

            if is_hidden_tag(tag) {
                return;
            }

            let role = semantic_role(tag);
            let indent = "  ".repeat(depth.min(6));

            match role {
                Some("heading") => {
                    let level = heading_level(tag);
                    let text = extract_text(handle).trim().to_string();
                    if !text.is_empty() {
                        let hashes = "#".repeat(level as usize);
                        output.push(format!("{indent}{hashes} {text}"));
                    }
                    return;
                }
                Some("link") => {
                    let text = extract_text(handle).trim().to_string();
                    let href = get_attr(handle, "href").unwrap_or_default();
                    if !text.is_empty() {
                        output.push(format!("{indent}[link: {text}]({href})"));
                    }
                    return;
                }
                Some("button") => {
                    let text = extract_text(handle).trim().to_string();
                    if !text.is_empty() && !is_noise(&text) {
                        // Truncate long button text
                        let display = if text.len() > 60 {
                            format!("{}...", &text[..60])
                        } else {
                            text
                        };
                        output.push(format!("{indent}[button: {display}]"));
                    }
                    return;
                }
                Some("image") => {
                    let alt = get_attr(handle, "alt").unwrap_or_default();
                    if !alt.is_empty() {
                        output.push(format!("{indent}[image: {alt}]"));
                    }
                    return;
                }
                Some("textbox") => {
                    let placeholder = get_attr(handle, "placeholder").unwrap_or_default();
                    let itype = get_attr(handle, "type").unwrap_or("text".into());
                    output.push(format!("{indent}[textbox: {placeholder}] (type={itype})"));
                    return;
                }
                Some("form") => {
                    let action = get_attr(handle, "action").unwrap_or_default();
                    output.push(format!("{indent}[form: action={action}]"));
                }
                Some("navigation") => {
                    output.push(format!("{indent}--- nav ---"));
                }
                Some(r) => {
                    output.push(format!("{indent}[{r}]"));
                }
                None => {}
            }

            for child in handle.children.borrow().iter() {
                walk(child, depth + 1, output);
            }
        }
        NodeData::Text { contents } => {
            let text = contents.borrow().trim().to_string();
            if !text.is_empty() && text.len() > 1 && !is_noise(&text) {
                let indent = "  ".repeat(depth.min(6));
                output.push(format!("{indent}{text}"));
            }
        }
        _ => {
            for child in handle.children.borrow().iter() {
                walk(child, depth + 1, output);
            }
        }
    }
}

/// Page stats from DOM analysis.
pub struct PageStats {
    pub total_nodes: usize,
    pub semantic_nodes: usize,
    pub links: usize,
    pub buttons: usize,
    pub forms: usize,
    pub images: usize,
    pub headings: usize,
    pub textboxes: usize,
}

impl PageStats {
    pub fn new() -> Self {
        Self {
            total_nodes: 0,
            semantic_nodes: 0,
            links: 0,
            buttons: 0,
            forms: 0,
            images: 0,
            headings: 0,
            textboxes: 0,
        }
    }
}

pub fn count_nodes(handle: &Handle, stats: &mut PageStats) {
    stats.total_nodes += 1;
    if let NodeData::Element { name, .. } = &handle.data {
        let tag = &name.local;
        if semantic_role(tag).is_some() {
            stats.semantic_nodes += 1;
        }
        match &**tag {
            "a" => stats.links += 1,
            "button" => stats.buttons += 1,
            "form" => stats.forms += 1,
            "img" => stats.images += 1,
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => stats.headings += 1,
            "input" | "textarea" => stats.textboxes += 1,
            _ => {}
        }
    }
    for child in handle.children.borrow().iter() {
        count_nodes(child, stats);
    }
}
