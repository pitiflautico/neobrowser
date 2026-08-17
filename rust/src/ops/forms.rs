//! Filling fields and submitting forms.
//!
//! Filling a single field is easy; filling a form is not, and the difference is why
//! `form_fill` exists separately. A real form re-renders between fields — a country select
//! reveals a state select, a validation message shifts the layout — so each field is
//! located freshly at the moment it is filled rather than from one upfront snapshot.
//! `submit` then has the harder job: deciding whether submitting actually did anything,
//! since a form that silently fails validation looks exactly like one that succeeded.

use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

use crate::cdp::{CdpClient, CdpError};
use crate::page;

use super::{bounded_secs_f64, js_lit, str_or};

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
    let deadline = t0 + Duration::from_secs_f64(bounded_secs_f64(wait_s));
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
