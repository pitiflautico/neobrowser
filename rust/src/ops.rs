//! Higher-level page operations built on `page::js` — the JS-blob-backed tools
//! (page_info, analyze, fill, form_fill, submit, find_and_click, dismiss_overlay,
//! extract, extract_table, scroll, wait, paginate). JS blobs are ported verbatim
//! from the Python server; arguments are interpolated with `serde_json::to_string`
//! for the same safe escaping the Python got from `json.dumps`.

use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

use crate::cdp::{CdpClient, CdpError};
use crate::page;

/// Safely encode a Rust string as a JS literal (quotes + escaping).
fn js_lit(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// `js` tool — evaluate arbitrary page JS and return the value (string passthrough).
pub async fn eval_js(client: &CdpClient, code: &str) -> Result<String, CdpError> {
    let v = page::js(client, code).await?;
    Ok(match v {
        Value::String(s) => s,
        other => other.to_string(),
    })
}

const PAGE_INFO_JS: &str = r#"
    var els = document.querySelectorAll('a,button,input,select,textarea,[role=button],[role=link]');
    var forms = document.querySelectorAll('form');
    var overlays = Array.from(document.querySelectorAll('*')).filter(function(e) {
        var s = window.getComputedStyle(e);
        return (s.position === 'fixed' || s.position === 'sticky') &&
               parseInt(s.zIndex) > 100 && e.offsetHeight > 50;
    });
    return JSON.stringify({
        url: location.href, title: document.title,
        interactive: els.length, forms: forms.length,
        has_overlay: overlays.length > 0, overlay_count: overlays.length
    });
"#;

pub async fn page_info(client: &CdpClient) -> Result<String, CdpError> {
    Ok(str_or(page::js(client, PAGE_INFO_JS).await?, "{}"))
}

const ANALYZE_JS: &str = r#"
    var forms = Array.from(document.querySelectorAll('form')).map(function(f, fi) {
        var fields = Array.from(f.querySelectorAll('input,select,textarea')).map(function(el) {
            var label = '';
            if (el.id) { var l = document.querySelector('label[for="'+el.id+'"]'); if(l) label = l.textContent.trim(); }
            if (!label) label = el.placeholder || el.name || el.type || '';
            return {tag: el.tagName.toLowerCase(), type: el.type||'', name: el.name||'', id: el.id||'', label: label, value: el.value||''};
        });
        return {index: fi, action: f.action||'', method: f.method||'get', fields: fields};
    });
    var buttons = Array.from(document.querySelectorAll('button,[role=button],input[type=submit],input[type=button]')).slice(0,20).map(function(b) {
        return {tag: b.tagName.toLowerCase(), text: (b.textContent||b.value||'').trim().slice(0,60), type: b.type||''};
    });
    var overlays = Array.from(document.querySelectorAll('*')).filter(function(e) {
        var s = window.getComputedStyle(e);
        return (s.position==='fixed'||s.position==='sticky') && parseInt(s.zIndex)>100 && e.offsetHeight>50;
    }).slice(0,5).map(function(e){ return {tag: e.tagName.toLowerCase(), id: e.id||'', cls: e.className.toString().slice(0,60)}; });
    var active = document.activeElement ? {tag: document.activeElement.tagName.toLowerCase(), id: document.activeElement.id||''} : null;
    return JSON.stringify({forms: forms, buttons: buttons, overlays: overlays, active_element: active});
"#;

pub async fn analyze(client: &CdpClient) -> Result<String, CdpError> {
    Ok(str_or(page::js(client, ANALYZE_JS).await?, "{}"))
}

/// `fill` — set a field's value using the element's own prototype setter and fire
/// input/change (React/Vue-safe), handling select/checkbox/radio/contenteditable.
pub async fn fill(client: &CdpClient, selector: &str, value: &str) -> Result<String, CdpError> {
    let code = format!(
        r#"return (function() {{
            var sel = {sel}; var v = {val};
            var el = document.querySelector(sel);
            if (!el) return JSON.stringify({{ok: false, error: "selector not found"}});
            var tag = el.tagName.toLowerCase();
            var type = (el.type || '').toLowerCase();
            if (tag === 'select') {{
                el.value = v; el.dispatchEvent(new Event('change', {{bubbles: true}}));
            }} else if (type === 'checkbox' || type === 'radio') {{
                el.checked = (v === 'true' || v === true);
                el.dispatchEvent(new Event('change', {{bubbles: true}}));
            }} else if (el.isContentEditable) {{
                el.focus(); el.textContent = v;
                el.dispatchEvent(new Event('input', {{bubbles: true}}));
                return JSON.stringify({{ok: true, tag: tag, type: 'contenteditable', value: el.textContent}});
            }} else {{
                var proto = tag === 'textarea' ? window.HTMLTextAreaElement.prototype : window.HTMLInputElement.prototype;
                var setter = Object.getOwnPropertyDescriptor(proto, 'value');
                if (setter && setter.set) {{ setter.set.call(el, v); }} else {{ el.value = v; }}
                el.dispatchEvent(new Event('input', {{bubbles: true}}));
                el.dispatchEvent(new Event('change', {{bubbles: true}}));
            }}
            return JSON.stringify({{ok: true, tag: tag, type: type, value: el.value}});
        }})()"#,
        sel = js_lit(selector),
        val = js_lit(value),
    );
    Ok(str_or(
        page::js(client, &code).await?,
        r#"{"ok": false, "error": "js returned null"}"#,
    ))
}

/// `form_fill` — fill multiple fields by fuzzy label/name/placeholder/aria match.
pub async fn form_fill(
    client: &CdpClient,
    fields: &Map<String, Value>,
    form_index: i64,
) -> Result<String, CdpError> {
    let mut results = Map::new();
    for (label, value) in fields {
        let value_str = match value {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        let code = format!(
            r#"return (function() {{
                var forms = document.querySelectorAll('form');
                var form = forms[{idx}] || document;
                var inputs = Array.from(form.querySelectorAll('input,select,textarea'));
                var target = null; var lq = {label}.toLowerCase();
                for (var i=0; i<inputs.length; i++) {{
                    var el = inputs[i];
                    var candidates = [el.name, el.id, el.placeholder, el.getAttribute('aria-label')];
                    var lbl = '';
                    if (el.id) {{ var l = document.querySelector('label[for="'+el.id+'"]'); if(l) lbl = l.textContent; }}
                    candidates.push(lbl);
                    for (var j=0; j<candidates.length; j++) {{
                        if (candidates[j] && candidates[j].toLowerCase().indexOf(lq) !== -1) {{ target = el; break; }}
                    }}
                    if (target) break;
                }}
                if (!target) return JSON.stringify({{ok: false, error: 'field not found: '+{label}}});
                var tag = target.tagName.toLowerCase(); var type = (target.type||'').toLowerCase(); var v = {val};
                if (tag === 'select') {{ target.value = v; target.dispatchEvent(new Event('change', {{bubbles: true}})); }}
                else if (type === 'checkbox' || type === 'radio') {{ target.checked = (v === 'true' || v === true); target.dispatchEvent(new Event('change', {{bubbles: true}})); }}
                else {{
                    var proto = tag === 'textarea' ? window.HTMLTextAreaElement.prototype : window.HTMLInputElement.prototype;
                    var setter = Object.getOwnPropertyDescriptor(proto, 'value');
                    if (setter && setter.set) {{ setter.set.call(target, v); }} else {{ target.value = v; }}
                    target.dispatchEvent(new Event('input', {{bubbles: true}}));
                    target.dispatchEvent(new Event('change', {{bubbles: true}}));
                }}
                return JSON.stringify({{ok: true, field: {label}, value: target.value}});
            }})()"#,
            idx = form_index,
            label = js_lit(label),
            val = js_lit(&value_str),
        );
        let res = page::js(client, &code).await?;
        let parsed: Value = match &res {
            Value::String(s) => serde_json::from_str(s).unwrap_or(json!({ "ok": false })),
            _ => json!({ "ok": false }),
        };
        results.insert(label.clone(), parsed);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Ok(json!({ "filled": results }).to_string())
}

/// `submit` — click a submit control (or given selector) and wait for navigation.
pub async fn submit(
    client: &CdpClient,
    selector: Option<&str>,
    wait_s: f64,
) -> Result<String, CdpError> {
    let url_before = str_or(page::js(client, "return location.href").await?, "");
    let method;
    if let Some(sel) = selector {
        let code = format!(
            "var el = document.querySelector({}); if (el) el.click();",
            js_lit(sel)
        );
        page::js(client, &code).await?;
        method = sel.to_string();
    } else {
        let m = page::js(
            client,
            r#"return (function() {
                var btn = document.querySelector('button[type=submit],input[type=submit]');
                if (btn) { btn.click(); return "button_click"; }
                var btn2 = document.querySelector('[aria-label*="submit" i],[aria-label*="send" i]');
                if (btn2) { btn2.click(); return "aria_button"; }
                var form = document.querySelector('form');
                if (form) { form.submit(); return "form_submit"; }
                return null;
            })()"#,
        )
        .await?;
        method = m.as_str().unwrap_or("").to_string();
        if method.is_empty() {
            return Ok(
                json!({ "ok": false, "error": "no submit button or form found" }).to_string(),
            );
        }
    }

    let t0 = Instant::now();
    let deadline = t0 + Duration::from_secs_f64(wait_s);
    let mut url_after = url_before.clone();
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let ready = str_or(page::js(client, "return document.readyState").await?, "");
        let url_now = str_or(page::js(client, "return location.href").await?, &url_before);
        if url_now != url_before || ready == "complete" {
            url_after = url_now;
            break;
        }
    }
    let waited_ms = t0.elapsed().as_millis();
    Ok(
        json!({ "ok": true, "method": method, "url": url_after, "waited_ms": waited_ms })
            .to_string(),
    )
}

/// `find_and_click` — click the nth clickable whose text/aria-label contains `text`.
pub async fn find_and_click(
    client: &CdpClient,
    text: &str,
    role: &str,
    nth: i64,
) -> Result<String, CdpError> {
    let code = format!(
        r#"return (function() {{
            var role = {role}; var textQ = {textq}; var nth = {nth};
            var sel = role ? '[role=' + role + '],button,a,[role=button],[role=link]' : 'button,a,[role=button],[role=link],input[type=submit]';
            var els = Array.from(document.querySelectorAll(sel));
            var matches = els.filter(function(e) {{
                return e.textContent.toLowerCase().indexOf(textQ) !== -1 ||
                       (e.getAttribute('aria-label')||'').toLowerCase().indexOf(textQ) !== -1;
            }});
            if (matches.length === 0) return JSON.stringify({{ok: false, error: "no match for: " + {textraw}}});
            var target = matches[Math.min(nth, matches.length-1)];
            target.click();
            return JSON.stringify({{ok: true, text: target.textContent.trim().slice(0,60), nth: nth}});
        }})()"#,
        role = js_lit(role),
        textq = js_lit(&text.to_lowercase()),
        textraw = js_lit(text),
        nth = nth,
    );
    Ok(str_or(page::js(client, &code).await?, r#"{"ok": false}"#))
}

const DISMISS_OVERLAY_JS: &str = r#"return (function(force){
    const ACCEPT = ['accept all','accept','agree','i agree','got it','ok','allow all','allow',
        'aceptar','acepto','permitir','entendido','continuar','cerrar'];
    const CLOSE = ['close','dismiss','no thanks','skip','×','✕','✗','x'];
    const click = (el) => { try { el.scrollIntoView(); el.click(); return true; } catch(e){ return false; } };
    const findBtn = (texts, root) => {
        const btns = Array.from((root||document).querySelectorAll(
            'button,a,[role=button],[class*=accept],[class*=agree],[class*=consent],[class*=cookie]'));
        for (const t of texts) {
            const b = btns.find(x => { const s=(x.innerText||'').trim().toLowerCase(); return s===t || s.startsWith(t); });
            if (b) return b;
        }
        return null;
    };
    const overlays = Array.from(document.querySelectorAll('*')).filter(e => {
        const s = getComputedStyle(e);
        return (s.position==='fixed'||s.position==='sticky') && parseInt(s.zIndex||0) > 50
            && e.offsetHeight > 40 && e.offsetWidth > 100;
    });
    if (!overlays.length) return JSON.stringify({dismissed:false, reason:'no overlay detected'});
    for (const o of overlays) { const b = findBtn(ACCEPT, o); if (b && click(b))
        return JSON.stringify({dismissed:true, method:'accept', text:(b.innerText||'').trim().slice(0,30)}); }
    for (const o of overlays) { const b = findBtn(CLOSE, o); if (b && click(b))
        return JSON.stringify({dismissed:true, method:'close', text:(b.innerText||'').trim().slice(0,30)}); }
    if (force) {
        document.dispatchEvent(new KeyboardEvent('keydown', {key:'Escape', bubbles:true}));
        const bd = document.querySelector('[class*=backdrop],[class*=overlay],[class*=mask]');
        if (bd) click(bd);
        return JSON.stringify({dismissed:true, method:'escape_backdrop'});
    }
    return JSON.stringify({dismissed:false, reason:'no dismiss button found, try force=true'});
})(FORCE);"#;

pub async fn dismiss_overlay(client: &CdpClient, force: bool) -> Result<String, CdpError> {
    let code = DISMISS_OVERLAY_JS.replace("FORCE", if force { "true" } else { "false" });
    Ok(str_or(
        page::js(client, &code).await?,
        r#"{"dismissed": false}"#,
    ))
}

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

/// `scroll` — scroll the viewport, then force a frame so load-on-scroll content paints.
pub async fn scroll(client: &CdpClient, direction: &str, amount: i64) -> Result<String, CdpError> {
    match direction {
        "top" => {
            page::js(client, "window.scrollTo(0, 0)").await?;
        }
        "bottom" => {
            page::js(client, "window.scrollTo(0, document.body.scrollHeight)").await?;
        }
        "up" => {
            page::js(client, &format!("window.scrollBy(0, -{amount})")).await?;
        }
        _ => {
            page::js(client, &format!("window.scrollBy(0, {amount})")).await?;
        }
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    page::nudge_frame(client).await; // virtualized lists load-on-scroll via a frame
    let pos = page::js(client, "return window.scrollY").await?;
    let pos = pos.as_f64().unwrap_or(0.0) as i64;
    Ok(json!({ "scrolled": direction, "amount": amount, "scrollY": pos }).to_string())
}

/// `wait` — sleep `ms`, or poll until `selector` appears (whichever `selector` implies).
pub async fn wait(client: &CdpClient, ms: i64, selector: Option<&str>) -> Result<String, CdpError> {
    if let Some(sel) = selector {
        let deadline = Instant::now() + Duration::from_millis(ms.max(0) as u64);
        let mut found = false;
        while Instant::now() < deadline {
            let expr = format!("return document.querySelectorAll({}).length", js_lit(sel));
            let count = page::js(client, &expr).await?.as_i64().unwrap_or(0);
            if count > 0 {
                found = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        Ok(json!({ "found": found, "selector": sel, "waited_ms": ms }).to_string())
    } else {
        tokio::time::sleep(Duration::from_millis(ms.max(0) as u64)).await;
        Ok(format!("Waited {ms}ms"))
    }
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

/// `debug` — install/flush/remove an in-page console interceptor.
pub async fn debug(client: &CdpClient, action: &str) -> Result<String, CdpError> {
    match action {
        "start" => {
            page::js(
                client,
                r#"if (!window.__neo_debug_logs) window.__neo_debug_logs = [];
                window.__neo_debug_orig = {log: console.log, warn: console.warn, error: console.error};
                ['log','warn','error'].forEach(function(l) {
                    console[l] = function() {
                        var msg = Array.from(arguments).map(function(a){ try{return JSON.stringify(a);}catch(e){return String(a);} }).join(' ');
                        window.__neo_debug_logs.push({level: l, msg: msg, t: Date.now()});
                        window.__neo_debug_orig[l].apply(console, arguments);
                    };
                });"#,
            )
            .await?;
            Ok(json!({ "ok": true, "action": "interceptor_installed" }).to_string())
        }
        "stop" => {
            page::js(
                client,
                r#"if (window.__neo_debug_orig) {
                    console.log = window.__neo_debug_orig.log;
                    console.warn = window.__neo_debug_orig.warn;
                    console.error = window.__neo_debug_orig.error;
                    delete window.__neo_debug_orig;
                }
                window.__neo_debug_logs = [];"#,
            )
            .await?;
            Ok(json!({ "ok": true, "action": "interceptor_removed" }).to_string())
        }
        _ => {
            // flush (default)
            Ok(str_or(
                page::js(
                    client,
                    "var logs = window.__neo_debug_logs || []; window.__neo_debug_logs = []; return JSON.stringify(logs);",
                )
                .await?,
                "[]",
            ))
        }
    }
}

/// Return a JSON value's string, or a fallback if it wasn't a non-empty string.
fn str_or(v: Value, fallback: &str) -> String {
    match v {
        Value::String(s) if !s.is_empty() => s,
        Value::Null => fallback.to_string(),
        Value::String(_) => fallback.to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn js_lit_escapes_quotes_and_specials() {
        assert_eq!(js_lit("a\"b"), "\"a\\\"b\"");
        assert_eq!(js_lit("x\ny"), "\"x\\ny\"");
    }

    #[test]
    fn str_or_falls_back_on_null_and_empty() {
        assert_eq!(str_or(Value::Null, "{}"), "{}");
        assert_eq!(str_or(Value::String(String::new()), "[]"), "[]");
        assert_eq!(str_or(Value::String("ok".into()), "x"), "ok");
        assert_eq!(str_or(json!({"a":1}), "x"), "{\"a\":1}");
    }

    #[test]
    fn dismiss_overlay_force_is_substituted() {
        assert!(DISMISS_OVERLAY_JS.contains("(FORCE)"));
        let t = DISMISS_OVERLAY_JS.replace("FORCE", "true");
        assert!(t.contains("(true)"));
        assert!(!t.contains("FORCE"));
    }
}
