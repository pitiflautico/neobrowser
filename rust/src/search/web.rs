//! Web search across providers, merged and deduplicated.
//!
//! More than one provider is queried because they disagree, and the disagreement is the
//! point: a query that returns nothing on one engine often returns the answer on another.
//! Consent dialogs are dismissed first, since a Google result page behind an unaccepted
//! consent banner extracts as an empty list — which is indistinguishable from no results.

//! Browser-driven search: text (Google → DuckDuckGo fallback), images, videos.
//!
//! Ported from the Python `_search_google`/`_search_duckduckgo` and
//! `google_search.py`. Search runs through the real stealth browser because a raw
//! HTTP fetch to Google/DDG gets bot-blocked. The Google image/video extraction
//! blobs are Google-DOM-specific and ported verbatim; they (like the Python
//! originals) may need selector updates when Google changes its markup.

use serde_json::{json, Value};

use crate::cdp::CdpClient;
use crate::page;

/// Percent-encode a query the `quote_plus` way (space → '+').
pub(super) fn quote_plus(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub(super) fn google_url(query: &str, udm: u8) -> String {
    format!(
        "https://www.google.com/search?q={}&udm={}&num=30",
        quote_plus(query),
        udm
    )
}

/// Loaded from `js/dismiss_consent.js`. See [`crate::js`] for why the snippets live
/// in real JavaScript files rather than Rust string literals.
fn dismiss_consent_js() -> &'static str {
    include_str!("../../js/dismiss_consent.js")
}

pub(super) async fn dismiss_consent(client: &CdpClient) {
    let _ = page::eval_body(client, dismiss_consent_js()).await;
}

/// Loaded from `js/search_google_text.js`. See [`crate::js`] for why the snippets live
/// in real JavaScript files rather than Rust string literals.
fn google_text_js() -> &'static str {
    include_str!("../../js/search_google_text.js")
}

/// Loaded from `js/search_ddg_text.js`. See [`crate::js`] for why the snippets live
/// in real JavaScript files rather than Rust string literals.
fn ddg_text_js() -> &'static str {
    include_str!("../../js/search_ddg_text.js")
}

pub(super) async fn js_array(client: &CdpClient, code: &str) -> Vec<Value> {
    match page::eval_body(client, code).await {
        Ok(Value::String(s)) => serde_json::from_str(&s).unwrap_or_default(),
        Ok(Value::Array(a)) => a,
        _ => Vec::new(),
    }
}

/// One search source: a URL to open plus JS that returns an array of results.
pub(super) struct Provider {
    pub(super) name: &'static str,
    pub(super) url: String,
    pub(super) extract_js: String,
    pub(super) consent: bool,
}

/// Run providers in order: navigate, dismiss consent, skip if the page is walled
/// (bot wall / captcha / etc.), extract, and merge deduped results until `count`.
///
/// This is the general answer to "we'll hit this on many more sites": no single
/// source is a hard dependency — a walled or empty provider is transparently
/// skipped and the next one fills in. Returns (results, per-engine trace).
pub(super) async fn merge_providers(
    client: &CdpClient,
    providers: Vec<Provider>,
    key: impl Fn(&Value) -> Option<String>,
    count: usize,
) -> (Vec<Value>, Vec<Value>) {
    let mut out: Vec<Value> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut trace: Vec<Value> = Vec::new();
    for p in providers {
        if out.len() >= count {
            break;
        }
        if page::navigate(client, &p.url, 3.0).await.is_err() {
            trace.push(json!({ "engine": p.name, "error": "navigate failed" }));
            continue;
        }
        if p.consent {
            dismiss_consent(client).await;
        }
        if let Some(w) = crate::walls::detect(client).await {
            trace.push(json!({ "engine": p.name, "walled": w.as_str() }));
            continue;
        }
        let before = out.len();
        for item in js_array(client, &p.extract_js).await {
            if let Some(k) = key(&item) {
                if seen.insert(k) {
                    out.push(item);
                    if out.len() >= count {
                        break;
                    }
                }
            }
        }
        trace.push(json!({ "engine": p.name, "got": out.len() - before }));
    }
    (out, trace)
}

pub(super) fn by_field(field: &'static str) -> impl Fn(&Value) -> Option<String> {
    move |v: &Value| {
        v.get(field)
            .and_then(|x| x.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from)
    }
}

/// `search` — merges DuckDuckGo + Google (skips whichever is walled).
pub async fn search(client: &CdpClient, query: &str, limit: usize) -> String {
    let providers = vec![
        Provider {
            name: "duckduckgo",
            url: format!("https://html.duckduckgo.com/html/?q={}", quote_plus(query)),
            extract_js: ddg_text_js().replace("LIMIT", &limit.to_string()),
            consent: false,
        },
        Provider {
            name: "google",
            url: format!(
                "https://www.google.com/search?q={}&hl=en&num=20",
                quote_plus(query)
            ),
            extract_js: google_text_js().replace("LIMIT", &limit.to_string()),
            consent: true,
        },
    ];
    let (mut results, engines) = merge_providers(client, providers, by_field("url"), limit).await;
    results.truncate(limit);
    json!({ "query": query, "results": results, "engines": engines }).to_string()
}
