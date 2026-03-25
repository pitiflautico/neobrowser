//! `extract` tool — structured data extraction from the current page.

use serde_json::Value;

use crate::state::McpState;
use crate::McpError;

use super::ToolDef;

/// Tool definition for `tools/list`.
pub(crate) fn definition() -> ToolDef {
    ToolDef {
        name: "extract",
        description: "Extract structured data from current page",
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "kind": {
                    "type": "string",
                    "enum": ["wom", "text", "links", "semantic", "tables", "metadata"],
                    "description": "What to extract"
                },
                "max_chars": {
                    "type": "integer",
                    "description": "Max characters for text extraction",
                    "default": 2000
                }
            },
            "required": ["kind"]
        }),
    }
}

/// Execute the `extract` tool.
pub fn call(args: Value, state: &mut McpState) -> Result<Value, McpError> {
    let kind = args
        .get("kind")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::InvalidParams("missing 'kind'".into()))?;

    match kind {
        "wom" => {
            let wom = state.engine.extract()?;
            Ok(serde_json::to_value(wom)?)
        }
        "text" => {
            // Text extraction: serialize WOM summary + node labels.
            let wom = state.engine.extract()?;
            let max = args
                .get("max_chars")
                .and_then(|v| v.as_u64())
                .unwrap_or(2000) as usize;
            let text = wom_to_text(&wom, max);
            Ok(serde_json::json!({ "text": text }))
        }
        "links" => {
            let links = state.engine.extract_links()?;
            let entries: Vec<serde_json::Value> = links
                .into_iter()
                .map(|(text, href)| serde_json::json!({ "text": text, "href": href }))
                .collect();
            Ok(serde_json::json!({ "links": entries, "count": entries.len() }))
        }
        "semantic" => {
            let semantic = state.engine.extract_semantic()?;
            let max = args
                .get("max_chars")
                .and_then(|v| v.as_u64())
                .unwrap_or(50000) as usize;
            let text = if semantic.len() > max {
                semantic[..max].to_string()
            } else {
                semantic
            };
            Ok(serde_json::json!({ "semantic": text }))
        }
        "tables" => {
            let js = r#"(function(){
                var tables=[];
                document.querySelectorAll('table').forEach(function(table,i){
                    var caption=(table.querySelector('caption')||{}).textContent||'';
                    var headers=[].slice.call(table.querySelectorAll('th')).map(function(th){return th.textContent.trim()});
                    var rows=[].slice.call(table.querySelectorAll('tr')).map(function(tr){
                        return [].slice.call(tr.querySelectorAll('td')).map(function(td){return td.textContent.trim()});
                    }).filter(function(row){return row.length>0});
                    tables.push({index:i,caption:caption.trim(),headers:headers,rows:rows,row_count:rows.length,col_count:Math.max(headers.length,rows.length>0?rows[0].length:0)});
                });
                return JSON.stringify({tables:tables,count:tables.length});
            })()"#;
            let result_str = state.engine.eval(js)?;
            let data: Value = serde_json::from_str(&result_str).unwrap_or_else(|_| {
                serde_json::json!({ "tables": [], "count": 0, "raw": result_str })
            });
            Ok(data)
        }
        "metadata" => {
            let js = r#"(function(){
                var m={};
                m.title=document.title||'';
                var desc=document.querySelector('meta[name="description"]');
                m.description=desc?desc.getAttribute('content')||'':'';
                var canon=document.querySelector('link[rel="canonical"]');
                m.canonical=canon?canon.getAttribute('href')||'':'';
                m.language=document.documentElement.lang||'';
                var cs=document.querySelector('meta[charset]');
                m.charset=cs?cs.getAttribute('charset')||'':'';
                if(!m.charset){var cs2=document.querySelector('meta[http-equiv="Content-Type"]');if(cs2){var c=cs2.getAttribute('content')||'';var idx=c.indexOf('charset=');m.charset=idx>=0?c.substring(idx+8).trim():'';}}
                var fav=document.querySelector('link[rel="icon"],link[rel="shortcut icon"]');
                m.favicon=fav?fav.getAttribute('href')||'':'';
                m.og={};
                ['title','description','image','url','type'].forEach(function(p){
                    var el=document.querySelector('meta[property="og:'+p+'"]');
                    m.og[p]=el?el.getAttribute('content')||'':'';
                });
                m.twitter={};
                ['card','title','description','image'].forEach(function(p){
                    var el=document.querySelector('meta[name="twitter:'+p+'"],meta[property="twitter:'+p+'"]');
                    m.twitter[p]=el?el.getAttribute('content')||'':'';
                });
                m.structured_data=[];
                document.querySelectorAll('script[type="application/ld+json"]').forEach(function(s){
                    try{m.structured_data.push(JSON.parse(s.textContent));}catch(e){}
                });
                m.alternate=[];
                document.querySelectorAll('link[rel="alternate"][hreflang]').forEach(function(l){
                    m.alternate.push({hreflang:l.getAttribute('hreflang')||'',href:l.getAttribute('href')||''});
                });
                var robots=document.querySelector('meta[name="robots"]');
                m.robots=robots?robots.getAttribute('content')||'':'';
                return JSON.stringify(m);
            })()"#;
            let result_str = state.engine.eval(js)?;
            let data: Value = serde_json::from_str(&result_str).unwrap_or_else(|_| {
                serde_json::json!({ "error": "failed to parse metadata", "raw": result_str })
            });
            Ok(data)
        }
        other => Err(McpError::InvalidParams(format!("unknown kind: {other}"))),
    }
}

/// Convert WOM to compressed text within char budget.
fn wom_to_text(wom: &neo_extract::WomDocument, max_chars: usize) -> String {
    let mut buf = String::with_capacity(max_chars);
    buf.push_str(&wom.title);
    buf.push('\n');
    buf.push_str(&wom.summary);
    buf.push('\n');

    for node in &wom.nodes {
        if buf.len() >= max_chars {
            break;
        }
        let line = format!("[{}] {} {}\n", node.role, node.label, node.id);
        buf.push_str(&line);
    }

    buf.truncate(max_chars);
    buf
}
