//! AI page view — the "screen" for the AI agent.
//! Minimal tokens. Zero post-processing needed. Ready to act on.

use neo_extract::WomDocument;

/// Full page render after navigation.
pub fn render_page(url: &str, title: &str, _render_ms: u64, wom: &WomDocument, errors: &[String]) -> String {
    let mut out = String::with_capacity(1500);

    // Title + URL (one line)
    out.push_str(title);
    out.push_str(" | ");
    out.push_str(url);
    out.push('\n');

    body(&mut out, wom, url);

    if !errors.is_empty() {
        out.push_str(&format!("\n! {}\n", errors.join("; ")));
    }
    out
}

/// Lighter render after interaction.
pub fn render_wom(url: &str, wom: &WomDocument) -> String {
    let mut out = String::with_capacity(1000);
    body(&mut out, wom, url);
    out
}

fn body(out: &mut String, wom: &WomDocument, base_url: &str) {
    let base_origin = extract_origin(base_url);

    // 1. Headings (structure of the page)
    let headings: Vec<_> = wom.nodes.iter()
        .filter(|n| n.tag.len() == 2 && n.tag.starts_with('h') && !n.label.is_empty())
        .collect();
    if !headings.is_empty() {
        out.push('\n');
        for h in headings.iter().take(8) {
            out.push_str(&format!("  {}\n", trunc(&h.label, 80)));
        }
    }

    // 2. Forms — only show field names, compact
    let fields: Vec<_> = wom.nodes.iter()
        .filter(|n| n.interactive && matches!(n.tag.as_str(), "input" | "textarea" | "select"))
        .filter(|n| n.input_type.as_deref() != Some("hidden"))
        .collect();
    if !fields.is_empty() {
        let field_names: Vec<String> = fields.iter().map(|f| {
            let name = if !f.label.is_empty() { f.label.clone() }
                else if let Some(ref ph) = f.placeholder { ph.clone() }
                else if let Some(ref nm) = f.name { nm.clone() }
                else { f.input_type.clone().unwrap_or_else(|| "text".into()) };
            let req = if f.required { "*" } else { "" };
            format!("{name}{req}", )
        }).collect();
        out.push_str(&format!("\n[form] {}\n", field_names.join(", ")));
    }

    // 3. Buttons — one line
    let btns: Vec<&str> = wom.nodes.iter()
        .filter(|n| n.tag == "button" && n.interactive && !n.label.is_empty())
        .map(|n| n.label.as_str())
        .take(8)
        .collect();
    if !btns.is_empty() {
        out.push_str(&format!("[btn] {}\n", btns.join(" | ")));
    }

    // 4. Content links — only external/meaningful, with absolute URLs
    let mut seen = std::collections::HashSet::new();
    let links: Vec<(String, String)> = wom.nodes.iter()
        .filter_map(|n| {
            let href = n.href.as_deref()?;
            if n.label.is_empty() || n.label.len() < 5 { return None; }
            if href.is_empty() || href == "#" || href.starts_with("javascript:") { return None; }
            // Skip nav links (short labels like "new", "past", "login")
            if n.label.len() < 15 && !href.starts_with("http") { return None; }
            let full_url = resolve_url(href, &base_origin);
            if !seen.insert(full_url.clone()) { return None; }
            Some((trunc(&n.label, 60).to_string(), full_url))
        })
        .collect();

    if !links.is_empty() {
        let show = links.len().min(15);
        out.push('\n');
        for (label, href) in links.iter().take(show) {
            out.push_str(&format!("  {} → {}\n", label, shorten_url(&href)));
        }
        if links.len() > show {
            out.push_str(&format!("  +{} more\n", links.len() - show));
        }
    }

    // 5. Key text — paragraphs only, meaningful content
    let texts: Vec<&str> = wom.nodes.iter()
        .filter(|n| {
            matches!(n.tag.as_str(), "p" | "li" | "td")
                && !n.interactive
                && n.label.len() > 40
                && !n.label.starts_with("http")
        })
        .map(|n| n.label.as_str())
        .take(3)
        .collect();
    if !texts.is_empty() {
        out.push('\n');
        for t in texts {
            out.push_str(&format!("> {}\n", trunc(t, 150)));
        }
    }
}

fn trunc(s: &str, max: usize) -> &str {
    if s.len() <= max { return s; }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) { end -= 1; }
    &s[..end]
}

fn extract_origin(url: &str) -> String {
    // Simple origin extraction without url crate
    if let Some(rest) = url.strip_prefix("https://") {
        let host = rest.split('/').next().unwrap_or("");
        format!("https://{host}")
    } else if let Some(rest) = url.strip_prefix("http://") {
        let host = rest.split('/').next().unwrap_or("");
        format!("http://{host}")
    } else {
        String::new()
    }
}

fn resolve_url(href: &str, base_origin: &str) -> String {
    if href.starts_with("http") {
        href.to_string()
    } else if href.starts_with('/') {
        format!("{base_origin}{href}")
    } else {
        href.to_string()
    }
}

fn shorten_url(url: &str) -> String {
    // Remove protocol, keep domain + path (max 60 chars)
    let short = url.replace("https://", "").replace("http://", "");
    if short.len() > 60 {
        format!("{}...", &short[..57])
    } else {
        short
    }
}
