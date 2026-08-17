//! What actually went over the wire, in a format other tools already read.
//!
//! HAR is here rather than a bespoke JSON shape because the whole point of capturing a
//! network log is to open it somewhere else — Chrome DevTools, a waterfall viewer, a
//! colleague's machine. `from_har` completes the round trip, so a captured session can be
//! replayed and diffed rather than only looked at.

//! Deep debugging: performance traces with Web Vitals, network waterfalls with
//! bounded response bodies, computed styles, and HAR export.
//!
//! This is the gap against Chrome DevTools MCP. The existing `console_logs` /
//! `network_log` tools answer "what happened"; this module answers "why is it slow"
//! and "what exactly did that request return", which is what a developer debugging
//! their own app actually needs.
//!
//! Everything here is read-only and passes through [`crate::trace::redact`] before it
//! leaves, because a network waterfall is made of headers and query strings — i.e. of
//! session tokens.

use serde_json::{json, Value};

use crate::cdp::{CdpClient, CdpError};

/// `response_body` — the body of a captured response, capped.
///
/// Capped because a response body is unbounded and this goes into a model's context;
/// truncation is reported so a partial body is never mistaken for the whole one.
pub async fn response_body(
    client: &CdpClient,
    request_id: &str,
    max_chars: usize,
) -> Result<String, CdpError> {
    let result = client
        .send(
            "Network.getResponseBody",
            json!({ "requestId": request_id }),
        )
        .await?;
    let body = result
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let base64 = result
        .get("base64Encoded")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let truncated = body.chars().count() > max_chars;
    let shown: String = body.chars().take(max_chars).collect();
    Ok(json!({
        "request_id": request_id,
        "base64_encoded": base64,
        "truncated": truncated,
        "body": crate::trace::redact(&shown),
    })
    .to_string())
}

/// Build a HAR 1.2 document from captured network entries.
///
/// HAR because it is the interchange format every other tool already reads — DevTools,
/// Charles, Fiddler — so an export is useful outside NeoBrowser instead of being a
/// bespoke shape someone has to write a parser for.
pub fn to_har(entries: &[crate::capture::NetworkEntry], page_url: &str) -> Value {
    let har_entries: Vec<Value> = entries
        .iter()
        .map(|e| {
            json!({
                "startedDateTime": "1970-01-01T00:00:00.000Z",
                "time": e.duration_ms.unwrap_or(0.0),
                "request": {
                    "method": e.method,
                    // Redacted: a HAR is routinely attached to a bug report, and a
                    // query-string token in one is a live credential.
                    "url": crate::trace::redact(&e.url),
                    "httpVersion": "HTTP/1.1",
                    "cookies": [],
                    "headers": [],
                    "queryString": [],
                    "headersSize": -1,
                    "bodySize": -1,
                },
                "response": {
                    "status": e.status.unwrap_or(0),
                    "statusText": e.status_text,
                    "httpVersion": "HTTP/1.1",
                    "cookies": [],
                    "headers": [],
                    "content": {
                        "size": e.encoded_data_length.unwrap_or(0.0) as i64,
                        "mimeType": "",
                    },
                    "redirectURL": "",
                    "headersSize": -1,
                    "bodySize": e.encoded_data_length.unwrap_or(0.0) as i64,
                },
                "cache": {},
                "timings": { "send": 0, "wait": e.duration_ms.unwrap_or(0.0), "receive": 0 },
            })
        })
        .collect();

    json!({
        "log": {
            "version": "1.2",
            "creator": { "name": "NeoBrowser", "version": env!("CARGO_PKG_VERSION") },
            "pages": [{
                "startedDateTime": "1970-01-01T00:00:00.000Z",
                "id": "page_1",
                "title": crate::trace::redact(page_url),
                "pageTimings": {},
            }],
            "entries": har_entries,
        }
    })
}

/// Parse a HAR document and summarise it.
///
/// Import exists so a HAR captured elsewhere — by DevTools, by a colleague, in a bug
/// report — can be read here. Summarised rather than echoed: a HAR is megabytes and the
/// useful part is the failures and the slow requests.
pub fn from_har(text: &str) -> Value {
    let Ok(har) = serde_json::from_str::<Value>(text) else {
        return json!({ "ok": false, "error": "not valid JSON" });
    };
    let Some(entries) = har
        .get("log")
        .and_then(|l| l.get("entries"))
        .and_then(Value::as_array)
    else {
        return json!({ "ok": false, "error": "no log.entries array: this is not a HAR document" });
    };

    let mut failures = Vec::new();
    let mut slowest: Vec<(f64, String, i64)> = Vec::new();
    let mut total_bytes = 0i64;

    for e in entries {
        let url = e
            .get("request")
            .and_then(|r| r.get("url"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let status = e
            .get("response")
            .and_then(|r| r.get("status"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let time = e.get("time").and_then(Value::as_f64).unwrap_or(0.0);
        total_bytes += e
            .get("response")
            .and_then(|r| r.get("bodySize"))
            .and_then(Value::as_i64)
            .unwrap_or(0)
            .max(0);
        // A 0 status is a request that never completed — blocked, aborted, offline —
        // which is as much a failure as a 500 and is easy to overlook.
        if status >= 400 || status == 0 {
            failures.push(json!({
                "url": crate::trace::redact(url),
                "status": status,
                "time_ms": time.round(),
            }));
        }
        slowest.push((time, crate::trace::redact(url), status));
    }
    slowest.sort_by(|a, b| b.0.total_cmp(&a.0));
    slowest.truncate(10);

    json!({
        "ok": true,
        "entries": entries.len(),
        "total_body_bytes": total_bytes,
        "failures": failures,
        "slowest": slowest
            .into_iter()
            .map(|(t, u, s)| json!({ "time_ms": t.round(), "url": u, "status": s }))
            .collect::<Vec<_>>(),
    })
}
