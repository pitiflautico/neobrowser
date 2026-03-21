//! Script extraction and meta-refresh detection for HTML pages.

/// Script extracted from HTML for execution.
pub(crate) enum ScriptInfo {
    /// Inline `<script>` tag with JS source.
    Inline {
        content: String,
        #[allow(dead_code)]
        is_module: bool,
    },
    /// External `<script src="...">` tag.
    External {
        url: String,
        #[allow(dead_code)]
        is_module: bool,
    },
}

/// Extract `<script>` tags from HTML.
///
/// Returns inline content and external URLs in document order.
/// Skips non-JS types (JSON, importmap, template, etc.).
pub(crate) fn extract_scripts(html: &str, base_url: &str) -> Vec<ScriptInfo> {
    use html5ever::parse_document;
    use html5ever::tendril::TendrilSink;
    use markup5ever_rcdom::RcDom;

    let dom = parse_document(RcDom::default(), Default::default()).one(html);

    let mut scripts = Vec::new();
    collect_scripts(&dom.document, base_url, &mut scripts);
    scripts
}

/// Recursively walk the DOM tree collecting script elements.
fn collect_scripts(node: &markup5ever_rcdom::Handle, base: &str, scripts: &mut Vec<ScriptInfo>) {
    use markup5ever_rcdom::NodeData;

    if let NodeData::Element {
        ref name,
        ref attrs,
        ..
    } = node.data
    {
        if name.local.as_ref() == "script" {
            let attrs_ref = attrs.borrow();
            let script_type = attrs_ref
                .iter()
                .find(|a| a.name.local.as_ref() == "type")
                .map(|a| a.value.to_string())
                .unwrap_or_default();

            // Skip non-JS script types.
            let st = script_type.to_lowercase();
            if st.contains("json")
                || st.contains("importmap")
                || st.contains("template")
                || st.contains("html")
                || st.contains("x-")
            {
                for child in node.children.borrow().iter() {
                    collect_scripts(child, base, scripts);
                }
                return;
            }

            let is_module = script_type == "module";
            let src = attrs_ref
                .iter()
                .find(|a| a.name.local.as_ref() == "src")
                .map(|a| a.value.to_string());

            if let Some(src) = src {
                let full = resolve_script_url(&src, base);
                scripts.push(ScriptInfo::External {
                    url: full,
                    is_module,
                });
            } else {
                drop(attrs_ref);
                let text: String = node
                    .children
                    .borrow()
                    .iter()
                    .filter_map(|c| match &c.data {
                        NodeData::Text { contents } => Some(contents.borrow().to_string()),
                        _ => None,
                    })
                    .collect();
                if !text.trim().is_empty() {
                    scripts.push(ScriptInfo::Inline {
                        content: text,
                        is_module,
                    });
                }
            }
        }
    }
    for child in node.children.borrow().iter() {
        collect_scripts(child, base, scripts);
    }
}

/// Detect `<meta http-equiv="refresh" content="...;url=...">` in HTML.
pub(crate) fn detect_meta_refresh(html: &str, base_url: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let needle = "http-equiv";
    let mut search_from = 0;
    while let Some(pos) = lower[search_from..].find(needle) {
        let abs_pos = search_from + pos;
        search_from = abs_pos + needle.len();
        let surrounding = &lower[abs_pos..std::cmp::min(abs_pos + 100, lower.len())];
        if !surrounding.contains("refresh") {
            continue;
        }
        let tag_start = lower[..abs_pos].rfind('<').unwrap_or(abs_pos);
        let tag_end = lower[tag_start..]
            .find('>')
            .map(|p| tag_start + p)
            .unwrap_or(lower.len());
        let tag = &html[tag_start..tag_end];
        let tag_lower = tag.to_lowercase();
        if let Some(ci) = tag_lower.find("content=") {
            let after = &tag[ci + 8..];
            let (delim, start_offset) = if after.starts_with('"') {
                ('"', 1)
            } else if after.starts_with('\'') {
                ('\'', 1)
            } else {
                continue;
            };
            let content_str = &after[start_offset..];
            if let Some(end) = content_str.find(delim) {
                let content_val = &content_str[..end];
                let content_lower = content_val.to_lowercase();
                if let Some(url_pos) = content_lower.find("url=") {
                    let target = content_val[url_pos + 4..].trim();
                    if !target.is_empty() {
                        return Some(resolve_script_url(target, base_url));
                    }
                }
            }
        }
    }
    None
}

/// Resolve a script src URL against a base URL.
fn resolve_script_url(src: &str, base: &str) -> String {
    if src.starts_with("http") {
        src.to_string()
    } else if src.starts_with("//") {
        format!("https:{src}")
    } else if let Ok(base_url) = url::Url::parse(base) {
        base_url
            .join(src)
            .map(|u| u.to_string())
            .unwrap_or_else(|_| src.to_string())
    } else {
        src.to_string()
    }
}
