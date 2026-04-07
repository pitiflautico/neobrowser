//! DOM export — converts between HTML and the NeoRender JS DOM.
//!
//! html_to_dom_js: parses HTML → generates JS that populates document.head/body
//! reparse_html: takes exported HTML string → html5ever rcdom Handle for WOM

use html5ever::parse_document;
use html5ever::tendril::TendrilSink;
use markup5ever_rcdom::{RcDom, Handle, NodeData};

/// Convert raw HTML into JS code that populates the NeoRender document.
/// Skips script/style/link tags (scripts are executed separately).
pub fn html_to_dom_js(html: &str) -> String {
    let dom = parse_document(RcDom::default(), Default::default())
        .from_utf8()
        .read_from(&mut html.as_bytes())
        .unwrap();

    let mut js = String::with_capacity(html.len() / 3);
    js.push_str("(function(){\nconst d=document,head=d.head,body=d.body;\n");

    let (head_node, body_node) = find_head_body(&dom.document);

    if let Some(h) = head_node {
        // Copy head attributes
        copy_attrs(&h, "d.head", &mut js);
        emit_children(&h, "head", &mut js, 0);
    }
    if let Some(b) = body_node {
        copy_attrs(&b, "d.body", &mut js);
        emit_children(&b, "body", &mut js, 0);
    }

    js.push_str("})();\n");
    js
}

fn find_head_body(node: &Handle) -> (Option<Handle>, Option<Handle>) {
    let mut head = None;
    let mut body = None;
    for child in node.children.borrow().iter() {
        if let NodeData::Element { name, .. } = &child.data {
            let tag = name.local.as_ref();
            if tag == "head" { head = Some(child.clone()); }
            if tag == "body" { body = Some(child.clone()); }
            if tag == "html" {
                let (h, b) = find_head_body(child);
                if h.is_some() { head = h; }
                if b.is_some() { body = b; }
            }
        }
    }
    (head, body)
}

fn copy_attrs(node: &Handle, var: &str, js: &mut String) {
    if let NodeData::Element { attrs, .. } = &node.data {
        for attr in attrs.borrow().iter() {
            let name = attr.name.local.as_ref();
            let val = attr.value.to_string();
            if !name.is_empty() {
                js.push_str(&format!("{}.setAttribute('{}','{}');\n", var, escape_js(name), escape_js(&val)));
            }
        }
    }
}

fn emit_children(node: &Handle, parent_var: &str, js: &mut String, depth: usize) {
    if depth > 30 { return; }
    for (i, child) in node.children.borrow().iter().enumerate() {
        let var_name = format!("e{}_{}", depth, i);
        match &child.data {
            NodeData::Element { name, attrs, .. } => {
                let tag = name.local.as_ref();
                // Skip scripts (executed separately), styles, links
                if tag == "script" || tag == "style" || tag == "link" { continue; }

                js.push_str(&format!("var {}=d.createElement('{}');\n", var_name, escape_js(tag)));
                for attr in attrs.borrow().iter() {
                    let attr_name = attr.name.local.as_ref();
                    let attr_val = attr.value.to_string();
                    js.push_str(&format!("{}.setAttribute('{}','{}');\n",
                        var_name, escape_js(attr_name), escape_js(&attr_val)));
                }
                js.push_str(&format!("{}.appendChild({});\n", parent_var, var_name));
                emit_children(child, &var_name, js, depth + 1);
            }
            NodeData::Text { contents } => {
                let text = contents.borrow().to_string();
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    js.push_str(&format!("{}.appendChild(d.createTextNode('{}'));\n",
                        parent_var, escape_js(trimmed)));
                }
            }
            NodeData::Comment { contents } => {
                let text = contents.to_string();
                js.push_str(&format!("{}.appendChild(d.createComment('{}'));\n",
                    parent_var, escape_js(&text)));
            }
            _ => {}
        }
    }
}

fn escape_js(s: &str) -> String {
    s.replace('\\', "\\\\")
     .replace('\'', "\\'")
     .replace('\n', "\\n")
     .replace('\r', "\\r")
     .replace('\t', "\\t")
}

/// Re-parse an HTML string (exported from NeoRender JS DOM) into an rcdom tree.
/// This produces a Handle compatible with wom::build().
pub fn reparse_html(html: &str) -> Result<RcDom, String> {
    parse_document(RcDom::default(), Default::default())
        .from_utf8()
        .read_from(&mut html.as_bytes())
        .map_err(|e| format!("HTML re-parse error: {e}"))
}
