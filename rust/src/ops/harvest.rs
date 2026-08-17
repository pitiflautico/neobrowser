//! Getting structured data out of a page, including across pages.
//!
//! `extract` and `extract_table` turn a rendered DOM back into data, and `paginate` is here
//! rather than with the navigation verbs because it is only ever used in the same loop:
//! extract this page, advance, extract the next. It reports whether it actually advanced,
//! which matters — a "next" link that is present but disabled is the standard way a
//! scraping loop silently re-reads page one forever.

use crate::cdp::{CdpClient, CdpError};
use crate::page;

use super::{js_lit, str_or};

/// `extract` — links (default) or the outerHTML of all tables.
pub async fn extract(client: &CdpClient, what: &str) -> Result<String, CdpError> {
    if what == "links" {
        Ok(str_or(
            page::js(
                client,
                r#"return JSON.stringify(Array.from(document.querySelectorAll('a[href]')).slice(0,100).map(function(a){
                    return {text: a.textContent.trim().slice(0,80), href: a.href};
                }));"#,
            )
            .await?,
            "[]",
        ))
    } else {
        Ok(str_or(
            page::js(
                client,
                "return Array.from(document.querySelectorAll('table')).map(function(t){ return t.outerHTML; }).join('\\n');",
            )
            .await?,
            "",
        ))
    }
}

/// `extract_table` — parse a table into an array of header→cell objects.
pub async fn extract_table(
    client: &CdpClient,
    selector: &str,
    index: i64,
) -> Result<String, CdpError> {
    let code = format!(
        r#"return (function() {{
            var tables = document.querySelectorAll({sel});
            var table = tables[{idx}];
            if (!table) return JSON.stringify([]);
            var headers = Array.from(table.querySelectorAll('th')).map(function(th){{ return th.textContent.trim(); }});
            if (!headers.length) {{
                var firstRow = table.querySelector('tr');
                if (firstRow) headers = Array.from(firstRow.querySelectorAll('td')).map(function(td){{ return td.textContent.trim(); }});
            }}
            var rows = Array.from(table.querySelectorAll('tr')).slice(headers.length ? 1 : 0);
            var data = rows.map(function(row) {{
                var cells = Array.from(row.querySelectorAll('td')).map(function(td){{ return td.textContent.trim(); }});
                var obj = {{}};
                cells.forEach(function(c, i){{ obj[headers[i] || i] = c; }});
                return obj;
            }});
            return JSON.stringify(data);
        }})()"#,
        sel = js_lit(selector),
        idx = index,
    );
    Ok(str_or(page::js(client, &code).await?, "[]"))
}

/// `paginate` — click a "next" control (given selector or auto-detected), then frame.
pub async fn paginate(client: &CdpClient, selector: Option<&str>) -> Result<String, CdpError> {
    let result = if let Some(sel) = selector {
        let code = format!(
            r#"return (function() {{
                var el = document.querySelector({sel});
                if (!el) return JSON.stringify({{ok: false, error: "selector not found"}});
                el.click();
                return JSON.stringify({{ok: true, method: "custom_selector"}});
            }})()"#,
            sel = js_lit(sel)
        );
        str_or(page::js(client, &code).await?, r#"{"ok": false}"#)
    } else {
        str_or(
            page::js(
                client,
                r#"return (function() {
                    var patterns = ['next','siguiente','→','›','>>','»','more','load more'];
                    var els = Array.from(document.querySelectorAll('a,button,[role=button]'));
                    for (var i=0; i<els.length; i++) {
                        var txt = els[i].textContent.toLowerCase().trim();
                        var aria = (els[i].getAttribute('aria-label')||'').toLowerCase();
                        for (var j=0; j<patterns.length; j++) {
                            if (txt === patterns[j] || aria === patterns[j]) {
                                els[i].click();
                                return JSON.stringify({ok: true, matched: patterns[j]});
                            }
                        }
                    }
                    var rel = document.querySelector('a[rel=next]');
                    if (rel) { rel.click(); return JSON.stringify({ok: true, method: "rel_next"}); }
                    return JSON.stringify({ok: false, error: "no next button found"});
                })()"#,
            )
            .await?,
            r#"{"ok": false}"#,
        )
    };
    page::nudge_frame(client).await;
    Ok(result)
}
