//! AI page view — the "screen" for the AI agent.
//! Compact, actionable text. AI-oriented, no redundancy.

use neo_extract::WomDocument;

/// After navigation: full context.
pub fn render_page(url: &str, title: &str, render_ms: u64, wom: &WomDocument, errors: &[String]) -> String {
    let mut out = String::with_capacity(2000);
    out.push_str(&format!("[{}] {}\n", wom.page_type, title));
    out.push_str(&format!("url: {}\n", url));
    if render_ms > 0 {
        out.push_str(&format!("load: {}ms\n", render_ms));
    }
    body(&mut out, wom);
    if !errors.is_empty() {
        out.push_str(&format!("\n[errors] {}\n", errors.join("; ")));
    }
    out
}

/// After interaction: lighter context.
pub fn render_wom(url: &str, wom: &WomDocument) -> String {
    let mut out = String::with_capacity(2000);
    out.push_str(&format!("[{}]\n", wom.page_type));
    out.push_str(&format!("url: {}\n", url));
    body(&mut out, wom);
    out
}

fn body(out: &mut String, wom: &WomDocument) {
    // Headings
    let mut has_headings = false;
    for n in &wom.nodes {
        if n.tag.len() == 2 && n.tag.starts_with('h') && !n.label.is_empty() {
            if !has_headings { out.push('\n'); has_headings = true; }
            let d = n.tag.as_bytes()[1].wrapping_sub(b'0') as usize;
            let prefix: String = "#".repeat(d.min(6));
            out.push_str(&format!("{prefix} {}\n", trunc(&n.label, 80)));
        }
    }

    // Forms & fields
    let fields: Vec<_> = wom.nodes.iter()
        .filter(|n| n.interactive && matches!(n.tag.as_str(), "input" | "textarea" | "select"))
        .filter(|n| n.input_type.as_deref() != Some("hidden"))
        .collect();

    if !fields.is_empty() {
        out.push('\n');
        for f in &fields {
            let itype = f.input_type.as_deref().unwrap_or("text");
            let display = if !f.label.is_empty() { &f.label }
                else if let Some(ref ph) = f.placeholder { ph }
                else if let Some(ref nm) = f.name { nm }
                else { itype };
            let req = if f.required { " *" } else { "" };
            out.push_str(&format!("  {display}: [{itype}]{req}\n"));
        }
    }

    // Buttons
    let btns: Vec<&str> = wom.nodes.iter()
        .filter(|n| n.tag == "button" && n.interactive && !n.label.is_empty())
        .map(|n| n.label.as_str())
        .take(12)
        .collect();
    if !btns.is_empty() {
        out.push_str(&format!("\n[btn] {}\n", btns.join(" | ")));
    }

    // Links
    let mut seen = std::collections::HashSet::new();
    let links: Vec<(&str, &str)> = wom.nodes.iter()
        .filter_map(|n| {
            let href = n.href.as_deref()?;
            if n.label.is_empty() || href.is_empty() || href == "#"
                || href.starts_with("javascript:") { return None; }
            if !seen.insert(href) { return None; }
            Some((n.label.as_str(), href))
        })
        .collect();

    if !links.is_empty() {
        let show = links.len().min(10);
        out.push_str(&format!("\n[links] {}\n", links.len()));
        for (label, href) in links.iter().take(show) {
            out.push_str(&format!("  {} → {}\n", trunc(label, 30), trunc(href, 50)));
        }
        if links.len() > show {
            out.push_str(&format!("  +{} more\n", links.len() - show));
        }
    }

    // Key text
    let texts: Vec<&str> = wom.nodes.iter()
        .filter(|n| matches!(n.tag.as_str(), "p" | "li") && !n.interactive && n.label.len() > 30)
        .map(|n| n.label.as_str())
        .take(2)
        .collect();
    if !texts.is_empty() {
        out.push('\n');
        for t in texts {
            out.push_str(&format!("> {}\n", trunc(t, 120)));
        }
    }
}

fn trunc(s: &str, max: usize) -> &str {
    if s.len() <= max { return s; }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) { end -= 1; }
    &s[..end]
}
