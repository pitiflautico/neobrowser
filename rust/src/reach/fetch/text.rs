//! Turning fetched HTML into text worth reading.
//!
//! Deliberately not an HTML parser. The goal is the readable content of a page for a model,
//! and a dependency that builds a full DOM to produce a paragraph of text is a large amount of
//! new attack surface for input that is, by definition, hostile.

//! Server-side fetching: redirect following, credential scoping, and HTML reduction.
//!
//! The credential-scoping rule lives here and is the reason this file exists separately:
//! a cookie or an auth header must not survive a redirect off the origin the caller
//! asked for, including an `https` → `http` downgrade on the same host.

use std::time::Duration;

use serde_json::{json, Map, Value};

use super::get::{guarded_get, read_capped};

/// Strip zero-width and most control characters that hide in scraped text.
pub(in crate::reach) fn clean_scraped(s: &str) -> String {
    s.chars()
        .filter(|c| {
            !matches!(*c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{206F}' | '\u{FEFF}')
                && (!c.is_control() || *c == '\n' || *c == '\t')
        })
        .collect()
}

/// Very small HTML→text: drop script/style blocks and tags, collapse whitespace.
pub(in crate::reach) fn strip_html(input: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            // Skip whole script/style blocks.
            for (tag, end) in [("<script", "</script>"), ("<style", "</style>")] {
                if lower[i..].starts_with(tag) {
                    if let Some(rel) = lower[i..].find(end) {
                        i += rel + end.len();
                    } else {
                        i = bytes.len();
                    }
                    out.push(' ');
                    continue;
                }
            }
            // Skip a normal tag.
            if let Some(rel) = input[i..].find('>') {
                i += rel + 1;
                out.push(' ');
                continue;
            }
            break;
        }
        let ch = input[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    // Collapse whitespace.
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `browse` — server-side fetch of a public URL. JSON passes through; HTML is
/// reduced to text (8000-char cap). Never uses the browser (raw HTTP).
pub async fn browse(url: &str, headers: &Map<String, Value>) -> String {
    let (resp, withheld) = match guarded_get(
        url,
        "Mozilla/5.0 (compatible; neo-browser/rust)",
        Duration::from_secs(15),
        headers,
        None,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => return json!({ "ok": false, "error": e, "url": url }).to_string(),
    };
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = match read_capped(resp, 512 * 1024).await {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(e) => return json!({ "ok": false, "error": e.to_string(), "url": url }).to_string(),
    };
    // Logged as well as returned: the JSON passthrough below hands back the
    // upstream body verbatim, so there is no envelope of ours to carry the notice.
    if !withheld.is_empty() {
        tracing::warn!(
            headers = %withheld.join(", "),
            "withheld caller headers from a cross-origin redirect target"
        );
    }
    if content_type.contains("json") {
        return body;
    }
    let text = clean_scraped(&strip_html(&body));
    let text: String = text.chars().take(8000).collect();
    // Fenced and labelled: `browse` fetches arbitrary third-party HTML, which is the
    // most direct route for a page to try instructing the model.
    let wrapped = crate::untrusted::wrap(url, &text);
    let mut out = json!({
        "url": url,
        "content_type": content_type,
        "trust": wrapped["trust"].clone(),
        "text": wrapped["content"].clone(),
    });
    if let Some(inj) = wrapped.get("injection") {
        out["injection"] = inj.clone();
    }
    if let Some(w) = wrapped.get("warnings").and_then(Value::as_array) {
        out["warnings"] = json!(w.clone());
    }
    if !withheld.is_empty() {
        // Append rather than assign: an injection warning may already be here, and
        // overwriting it would hide the more serious of the two.
        let mut warns = out
            .get("warnings")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        warns.push(json!(format!(
            "redirect left the requested origin; these headers were not forwarded: {}",
            withheld.join(", ")
        )));
        out["warnings"] = Value::Array(warns);
    }
    out.to_string()
}
