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

/// `perf_trace` — Web Vitals, navigation timing and the slowest resources.
///
/// Returns an interpretation alongside the raw numbers: a table of milliseconds is
/// data, and the point of a debugging tool is to say which number is the problem.
pub async fn perf_trace(client: &CdpClient) -> Result<String, CdpError> {
    let raw = crate::page::js(client, &crate::js::vitals().returning()).await?;
    let mut vitals: Value = match raw {
        Value::String(s) => serde_json::from_str(&s).unwrap_or(json!({})),
        other => other,
    };

    let insights = derive_insights(&vitals);
    vitals["insights"] = json!(insights);
    Ok(crate::trace::redact_value(&vitals).to_string())
}

/// Turn timing numbers into the small set of statements worth acting on.
///
/// Thresholds are Google's published Web Vitals "good" boundaries, so the verdicts
/// match what Lighthouse and DevTools would say rather than inventing a scale.
fn derive_insights(v: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let num = |k: &str| v.get(k).and_then(Value::as_f64);

    if let Some(lcp) = num("largest_contentful_paint") {
        if lcp > 4000.0 {
            out.push(format!(
                "LCP {lcp:.0}ms is poor (>4000ms): the main content is slow to appear"
            ));
        } else if lcp > 2500.0 {
            out.push(format!("LCP {lcp:.0}ms needs improvement (>2500ms)"));
        }
    }
    if let Some(cls) = num("cumulative_layout_shift") {
        if cls > 0.25 {
            out.push(format!(
                "CLS {cls:.3} is poor (>0.25): the layout jumps as it loads"
            ));
        } else if cls > 0.1 {
            out.push(format!("CLS {cls:.3} needs improvement (>0.1)"));
        }
    }
    if let Some(ttfb) = num("ttfb") {
        if ttfb > 800.0 {
            out.push(format!(
                "TTFB {ttfb:.0}ms is slow (>800ms): the delay is server-side, before any \
                 rendering work"
            ));
        }
    }
    // The single slowest resource, when it dominates, is usually the whole story.
    if let Some(slowest) = v
        .get("slowest_resources")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
    {
        if let Some(ms) = slowest.get("ms").and_then(Value::as_f64) {
            if ms > 1000.0 {
                out.push(format!(
                    "slowest resource took {ms:.0}ms: {}",
                    slowest
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("(unknown)")
                ));
            }
        }
    }
    if out.is_empty() {
        out.push("no Web Vitals threshold was exceeded".into());
    }
    out
}

/// `computed_style` — the resolved CSS for one element.
///
/// Answers "why does it look like that", which a DOM dump cannot: the cascade result
/// is what matters and it is not visible in the markup.
pub async fn computed_style(
    client: &CdpClient,
    selector: &str,
    properties: &[String],
) -> Result<String, CdpError> {
    let want = if properties.is_empty() {
        // A useful default rather than all ~340 properties, which would blow the
        // context budget for no benefit.
        vec![
            "display",
            "position",
            "visibility",
            "opacity",
            "z-index",
            "width",
            "height",
            "color",
            "background-color",
            "font-size",
            "font-family",
            "overflow",
            "pointer-events",
        ]
        .into_iter()
        .map(String::from)
        .collect()
    } else {
        properties.to_vec()
    };
    let snippet = crate::js::computed_style()
        .with(
            "SEL",
            &serde_json::to_string(selector).unwrap_or_else(|_| "\"\"".into()),
        )
        .with(
            "PROPS",
            &serde_json::to_string(&want).unwrap_or_else(|_| "[]".into()),
        );
    let raw = crate::page::js(client, &snippet.returning()).await?;
    Ok(match raw {
        Value::String(s) => s,
        other => other.to_string(),
    })
}

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

/// `cpu_profile` — sample the JS main thread for `duration_ms`, then report the
/// heaviest functions.
///
/// Returns self-time totals rather than the raw sample tree: the raw profile is tens of
/// thousands of nodes, which is unusable in a model's context, and "which function
/// burned the time" is the question anyone actually has.
pub async fn cpu_profile(client: &CdpClient, duration_ms: u64) -> Result<String, CdpError> {
    client.send("Profiler.enable", json!({})).await?;
    client.send("Profiler.start", json!({})).await?;
    tokio::time::sleep(std::time::Duration::from_millis(
        duration_ms.clamp(100, 30_000),
    ))
    .await;
    let result = client.send("Profiler.stop", json!({})).await?;
    let _ = client.send("Profiler.disable", json!({})).await;

    let profile = result.get("profile").cloned().unwrap_or(json!({}));
    let nodes = profile
        .get("nodes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let samples = profile
        .get("samples")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    // Sample counts per node id: the profile gives a flat sample list referencing
    // nodes, so self-time is simply how often each node was on top of the stack.
    let mut hits: std::collections::HashMap<i64, usize> = std::collections::HashMap::new();
    for s in &samples {
        if let Some(id) = s.as_i64() {
            *hits.entry(id).or_insert(0) += 1;
        }
    }

    let mut frames: Vec<Value> = nodes
        .iter()
        .filter_map(|n| {
            let id = n.get("id").and_then(Value::as_i64)?;
            let count = *hits.get(&id)?;
            let cf = n.get("callFrame")?;
            let name = cf
                .get("functionName")
                .and_then(Value::as_str)
                .unwrap_or("(anonymous)");
            let url = cf.get("url").and_then(Value::as_str).unwrap_or("");
            Some(json!({
                "function": if name.is_empty() { "(anonymous)" } else { name },
                "url": crate::trace::redact(url),
                "line": cf.get("lineNumber").and_then(Value::as_i64),
                "samples": count,
            }))
        })
        .collect();
    frames.sort_by_key(|f| std::cmp::Reverse(f["samples"].as_u64().unwrap_or(0)));
    frames.truncate(20);

    let total: usize = hits.values().sum();
    Ok(json!({
        "duration_ms": duration_ms,
        "total_samples": total,
        "note": if total == 0 {
            "no samples: the main thread was idle for the whole window, so there is no JS \
             cost to attribute here"
        } else {
            "self-time only: `samples` is how often each function was on top of the stack"
        },
        "hottest": frames,
    })
    .to_string())
}

/// `heap_snapshot` — memory totals plus the DOM node and listener counts.
///
/// Deliberately not a full `.heapsnapshot`: those are hundreds of megabytes and cannot
/// be read by a model at all. The counts below are what actually diagnose a leak —
/// a node count that only ever grows is the signature.
pub async fn heap_stats(client: &CdpClient) -> Result<String, CdpError> {
    let _ = client.send("Performance.enable", json!({})).await;
    let metrics = client.send("Performance.getMetrics", json!({})).await?;
    let mut interesting = serde_json::Map::new();
    if let Some(list) = metrics.get("metrics").and_then(Value::as_array) {
        for m in list {
            let Some(name) = m.get("name").and_then(Value::as_str) else {
                continue;
            };
            if matches!(
                name,
                "JSHeapUsedSize"
                    | "JSHeapTotalSize"
                    | "Nodes"
                    | "JSEventListeners"
                    | "Documents"
                    | "Frames"
                    | "LayoutCount"
                    | "RecalcStyleCount"
            ) {
                interesting.insert(
                    name.to_string(),
                    m.get("value").cloned().unwrap_or(Value::Null),
                );
            }
        }
    }
    Ok(json!({
        "metrics": Value::Object(interesting),
        "how_to_use": "call this twice around a repeated interaction. Nodes or \
                       JSEventListeners that grow every cycle and never come back down is \
                       the signature of a leak",
    })
    .to_string())
}

/// Resolve a minified stack frame to its original file, line and column.
///
/// A minified stack is the normal case in production, and `bundle.js:1:48213` is not a
/// location anyone can act on. The map is fetched from the page (so same-origin maps
/// work) and the VLQ mappings are decoded here — see [`decode_mappings`].
pub async fn resolve_source(
    client: &CdpClient,
    script_url: &str,
    line: u32,
    column: u32,
) -> Result<String, CdpError> {
    // Fetched in the page rather than from Rust: a source map is often same-origin only,
    // and the page already has the cookies and the origin to reach it.
    let js = format!(
        r#"return (async function(){{
  var url = {url};
  var text;
  try {{ text = await (await fetch(url)).text(); }}
  catch (e) {{ return JSON.stringify({{ ok: false, error: 'could not fetch the script: ' + e }}); }}
  var m = /[#@]\s*sourceMappingURL=(\S+)/.exec(text.slice(-4000));
  if (!m) return JSON.stringify({{ ok: false, error: 'the script declares no sourceMappingURL' }});
  var mapUrl = new URL(m[1], url).href;
  var map;
  try {{ map = await (await fetch(mapUrl)).json(); }}
  catch (e) {{ return JSON.stringify({{ ok: false, error: 'could not fetch the source map: ' + e }}); }}
  return JSON.stringify({{
    ok: true,
    map_url: mapUrl,
    sources: map.sources || [],
    source_root: map.sourceRoot || '',
    mappings: map.mappings || '',
    has_content: !!(map.sourcesContent && map.sourcesContent.length),
  }});
}})()"#,
        url = serde_json::to_string(script_url).unwrap_or_else(|_| "\"\"".into()),
    );
    let raw = crate::page::js(client, &js).await?;
    let fetched: Value = match &raw {
        Value::String(t) => serde_json::from_str(t).unwrap_or(Value::Null),
        other => other.clone(),
    };
    if fetched.get("ok") != Some(&Value::Bool(true)) {
        return Ok(fetched.to_string());
    }

    let mappings_str = fetched
        .get("mappings")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let sources: Vec<&str> = fetched
        .get("sources")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let source_root = fetched
        .get("source_root")
        .and_then(Value::as_str)
        .unwrap_or("");

    let mappings = decode_mappings(mappings_str);
    // Stack traces are 1-based; source maps are 0-based. Off by one here means every
    // reported line is wrong by one, which is worse than not reporting at all because it
    // looks right.
    let zero_line = line.saturating_sub(1);
    let hit = lookup_mapping(&mappings, zero_line, column);

    let mut out = json!({
        "ok": true,
        "map_url": crate::trace::redact(
            fetched.get("map_url").and_then(Value::as_str).unwrap_or("")
        ),
        "generated": { "line": line, "column": column },
        "mappings_decoded": mappings.len(),
        "sources_embedded": fetched.get("has_content").cloned().unwrap_or(Value::Bool(false)),
    });

    match hit {
        Some(m) => {
            let file = sources
                .get(m.source_index)
                .copied()
                .unwrap_or("(unknown source)");
            out["original"] = json!({
                "file": if source_root.is_empty() {
                    file.to_string()
                } else {
                    format!("{}/{}", source_root.trim_end_matches('/'), file.trim_start_matches('/'))
                },
                // Back to 1-based for the answer, matching how an editor reports lines.
                "line": m.original_line + 1,
                "column": m.original_column,
            });
        }
        None => {
            out["original"] = Value::Null;
            out["note"] = json!(
                "no mapping covers that position. Stack traces are 1-based and source maps                  0-based, so check the line number came from a real stack frame; a line                  with no mappings has no original position at all"
            );
        }
    }
    Ok(out.to_string())
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

/// Base64 alphabet used by source-map VLQ.
const VLQ_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Decode one Base64-VLQ integer, returning the value and how many characters it used.
///
/// The encoding: each character carries 5 payload bits plus a continuation bit. In the
/// FIRST character the least-significant payload bit is the sign, which is the part
/// everyone gets wrong — a naive implementation reports negative deltas as huge
/// positives and every mapping after the first lands on the wrong line.
fn vlq_decode(input: &[u8]) -> Option<(i64, usize)> {
    let mut result: i64 = 0;
    let mut shift = 0;
    let mut consumed = 0;
    for &c in input {
        let digit = VLQ_ALPHABET.iter().position(|&a| a == c)? as i64;
        consumed += 1;
        let has_continuation = digit & 32 != 0;
        result += (digit & 31) << shift;
        shift += 5;
        if !has_continuation {
            let negative = result & 1 == 1;
            result >>= 1;
            return Some((if negative { -result } else { result }, consumed));
        }
        // A VLQ longer than this is malformed or hostile; refuse rather than shifting
        // past the width of the accumulator.
        if shift > 60 {
            return None;
        }
    }
    None
}

/// One decoded mapping entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mapping {
    pub generated_line: u32,
    pub generated_column: u32,
    pub source_index: usize,
    pub original_line: u32,
    pub original_column: u32,
}

/// Decode a source map's `mappings` string.
///
/// The format is delta-encoded: `;` separates generated lines, `,` separates segments
/// within a line, and every field except the generated column is a delta against the
/// previous segment *across* lines. The generated column resets per line and the others
/// do not — another detail that silently produces plausible-but-wrong results if missed.
pub fn decode_mappings(mappings: &str) -> Vec<Mapping> {
    let mut out = Vec::new();
    let (mut src, mut orig_line, mut orig_col) = (0i64, 0i64, 0i64);

    for (line_no, line) in mappings.split(';').enumerate() {
        // Resets per generated line; the source/original counters deliberately do not.
        let mut gen_col = 0i64;
        for segment in line.split(',') {
            if segment.is_empty() {
                continue;
            }
            let bytes = segment.as_bytes();
            let mut at = 0;
            let field = |at: &mut usize| -> Option<i64> {
                let (v, used) = vlq_decode(&bytes[*at..])?;
                *at += used;
                Some(v)
            };
            let Some(d_gen_col) = field(&mut at) else {
                continue;
            };
            gen_col += d_gen_col;
            // A one-field segment marks generated code with no original position.
            let Some(d_src) = field(&mut at) else {
                continue;
            };
            let Some(d_line) = field(&mut at) else {
                continue;
            };
            let Some(d_col) = field(&mut at) else {
                continue;
            };
            src += d_src;
            orig_line += d_line;
            orig_col += d_col;
            if src < 0 || orig_line < 0 || orig_col < 0 || gen_col < 0 {
                // Deltas that walk off the start mean a corrupt map; skip the segment
                // rather than reporting a nonsensical location.
                continue;
            }
            out.push(Mapping {
                generated_line: line_no as u32,
                generated_column: gen_col as u32,
                source_index: src as usize,
                original_line: orig_line as u32,
                original_column: orig_col as u32,
            });
        }
    }
    out
}

/// Find the mapping covering `line`/`column` in the generated file.
///
/// "Covering" means the last mapping at or before the column on that line, which is how
/// a source map is meant to be queried: mappings mark the *starts* of regions, so an
/// exact-match lookup finds nothing for most positions.
pub fn lookup_mapping(mappings: &[Mapping], line: u32, column: u32) -> Option<&Mapping> {
    mappings
        .iter()
        .filter(|m| m.generated_line == line && m.generated_column <= column)
        .max_by_key(|m| m.generated_column)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insights_flag_poor_web_vitals_against_published_thresholds() {
        let v = json!({
            "largest_contentful_paint": 5000.0,
            "cumulative_layout_shift": 0.4,
            "ttfb": 1200.0,
        });
        let out = derive_insights(&v);
        assert!(out.iter().any(|s| s.contains("LCP") && s.contains("poor")));
        assert!(out.iter().any(|s| s.contains("CLS") && s.contains("poor")));
        assert!(out.iter().any(|s| s.contains("TTFB")));
    }

    #[test]
    fn insights_distinguish_needs_improvement_from_poor() {
        let out = derive_insights(&json!({ "largest_contentful_paint": 3000.0 }));
        assert!(out.iter().any(|s| s.contains("needs improvement")));
        assert!(!out.iter().any(|s| s.contains("poor")));
    }

    /// A healthy page must say so explicitly. An empty list reads as "the tool did
    /// not work" rather than "nothing is wrong".
    #[test]
    fn a_healthy_page_gets_an_explicit_all_clear() {
        let out = derive_insights(&json!({
            "largest_contentful_paint": 1200.0,
            "cumulative_layout_shift": 0.01,
            "ttfb": 200.0,
        }));
        assert_eq!(out.len(), 1);
        assert!(out[0].contains("no Web Vitals threshold was exceeded"));
    }

    #[test]
    fn missing_metrics_do_not_produce_bogus_insights() {
        // A page where the observer buffer had nothing: no metric, no verdict.
        let out = derive_insights(&json!({}));
        assert_eq!(out.len(), 1);
        assert!(out[0].contains("no Web Vitals threshold"));
    }

    #[test]
    fn a_dominant_slow_resource_is_called_out() {
        let out = derive_insights(&json!({
            "slowest_resources": [{ "name": "https://cdn.test/huge.js", "ms": 3200.0 }],
        }));
        assert!(out.iter().any(|s| s.contains("huge.js")));
    }

    fn entry(url: &str) -> crate::capture::NetworkEntry {
        crate::capture::NetworkEntry {
            request_id: "1".into(),
            url: url.into(),
            method: "GET".into(),
            status: Some(200),
            status_text: "OK".into(),
            duration_ms: Some(50.0),
            encoded_data_length: Some(1024.0),
            timestamp: 0.0,
        }
    }

    #[test]
    fn har_export_is_well_formed() {
        let har = to_har(&[entry("https://example.com/a")], "https://example.com/");
        assert_eq!(har["log"]["version"], "1.2");
        assert_eq!(har["log"]["creator"]["name"], "NeoBrowser");
        assert_eq!(har["log"]["entries"][0]["request"]["method"], "GET");
        assert_eq!(har["log"]["entries"][0]["response"]["status"], 200);
        assert_eq!(har["log"]["pages"][0]["id"], "page_1");
    }

    /// A HAR is shared by construction, so a token in a URL must not ride along.
    #[test]
    fn har_export_redacts_urls() {
        let har = to_har(
            &[entry("https://api.test/cb?access_token=LEAKME&page=2")],
            "https://app.test/?session=ALSOLEAK",
        );
        let text = har.to_string();
        assert!(!text.contains("LEAKME"), "{text}");
        assert!(!text.contains("ALSOLEAK"), "{text}");
        // Non-secret parameters survive, or the export loses its diagnostic value.
        assert!(text.contains("page=2"));
    }

    #[test]
    fn a_non_har_document_is_rejected_clearly() {
        assert_eq!(from_har("not json")["ok"], false);
        assert_eq!(from_har(r#"{"hello":1}"#)["ok"], false);
        assert!(from_har(r#"{"hello":1}"#)["error"]
            .as_str()
            .unwrap()
            .contains("not a HAR"));
    }

    #[test]
    fn import_surfaces_failures_and_slow_requests() {
        let har = json!({ "log": { "entries": [
            { "time": 20.0, "request": { "url": "https://a.test/ok" },
              "response": { "status": 200, "bodySize": 100 } },
            { "time": 5000.0, "request": { "url": "https://a.test/slow" },
              "response": { "status": 200, "bodySize": 200 } },
            { "time": 10.0, "request": { "url": "https://a.test/gone" },
              "response": { "status": 404, "bodySize": 0 } },
            // An aborted request: status 0 must count as a failure, not be ignored.
            { "time": 1.0, "request": { "url": "https://a.test/blocked" },
              "response": { "status": 0, "bodySize": 0 } },
        ] } });
        let out = from_har(&har.to_string());
        assert_eq!(out["ok"], true);
        assert_eq!(out["entries"], 4);
        assert_eq!(out["total_body_bytes"], 300);
        assert_eq!(out["failures"].as_array().unwrap().len(), 2);
        assert_eq!(out["slowest"][0]["url"], "https://a.test/slow");
    }

    /// A HAR from someone else's session is full of tokens; importing must not echo
    /// them back into a transcript.
    #[test]
    fn imported_urls_are_redacted() {
        let har = json!({ "log": { "entries": [
            { "time": 1.0, "request": { "url": "https://a.test/?access_token=LEAKME" },
              "response": { "status": 500, "bodySize": 0 } },
        ] } });
        let text = from_har(&har.to_string()).to_string();
        assert!(!text.contains("LEAKME"), "{text}");
    }

    // --- source map VLQ ------------------------------------------------------

    /// The sign bit lives in the first character's LSB. Getting it wrong turns every
    /// negative delta into a huge positive one, so this is the load-bearing case.
    #[test]
    fn vlq_decodes_signed_values() {
        assert_eq!(vlq_decode(b"A").unwrap(), (0, 1));
        assert_eq!(vlq_decode(b"C").unwrap(), (1, 1));
        assert_eq!(vlq_decode(b"D").unwrap(), (-1, 1));
        assert_eq!(vlq_decode(b"E").unwrap(), (2, 1));
        assert_eq!(vlq_decode(b"F").unwrap(), (-2, 1));
        // Multi-character (continuation bit set on all but the last).
        assert_eq!(vlq_decode(b"2H").unwrap(), (123, 2));
        assert_eq!(vlq_decode(b"gB").unwrap(), (16, 2));
    }

    #[test]
    fn vlq_rejects_malformed_input() {
        assert_eq!(vlq_decode(b""), None);
        // Continuation bit set with nothing following.
        assert_eq!(vlq_decode(b"g"), None);
        // Not in the alphabet.
        assert_eq!(vlq_decode(b"!"), None);
        // A run of continuations must not shift past the accumulator.
        assert_eq!(vlq_decode(&[b'g'; 40]), None);
    }

    /// The field order is `[generatedColumn, sourceIndex, originalLine, originalColumn]`,
    /// all delta-encoded. Mixing up the middle two is the classic bug: it reports the
    /// right line number against the wrong file.
    #[test]
    fn mappings_decode_with_correct_field_order() {
        // `ACAA` = [genCol +0, sourceIndex +1, origLine +0, origCol +0].
        let m = decode_mappings("AAAA;ACAA");
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].generated_line, 0);
        assert_eq!(m[0].source_index, 0);
        assert_eq!(m[0].original_line, 0);
        assert_eq!(m[1].generated_line, 1);
        assert_eq!(m[1].source_index, 1, "the second field is the SOURCE index");
        assert_eq!(
            m[1].original_line, 0,
            "the original line did not advance here"
        );
    }

    /// The original-line counter carries across generated lines. If it were reset per
    /// line, every mapping after the first would point at the top of its file.
    #[test]
    fn the_original_line_counter_carries_across_generated_lines() {
        // `AACA` = [genCol +0, sourceIndex +0, origLine +1, origCol +0], twice.
        let m = decode_mappings("AACA;AACA;AACA");
        assert_eq!(m.len(), 3);
        assert_eq!(m[0].original_line, 1);
        assert_eq!(m[1].original_line, 2, "must accumulate, not reset");
        assert_eq!(m[2].original_line, 3);
    }

    #[test]
    fn the_generated_column_resets_per_line_but_others_do_not() {
        // Line 0: columns 0 then 2 (delta 1 -> encoded 'C' is +1 after the >>1).
        let m = decode_mappings("AAAA,CAAA;AAAA");
        assert_eq!(m[0].generated_column, 0);
        assert_eq!(
            m[1].generated_column, 1,
            "second segment advances the column"
        );
        assert_eq!(
            m[2].generated_column, 0,
            "a new generated line must restart the column"
        );
    }

    #[test]
    fn segments_without_an_original_position_are_skipped() {
        // A single-field segment marks generated-only code.
        let m = decode_mappings("A;AAAA");
        assert_eq!(m.len(), 1, "the one-field segment has no original position");
        assert_eq!(m[0].generated_line, 1);
    }

    #[test]
    fn empty_and_corrupt_mappings_do_not_panic() {
        assert!(decode_mappings("").is_empty());
        assert!(decode_mappings(";;;").is_empty());
        assert!(decode_mappings("!!!,???").is_empty());
        // Negative deltas walking off the start are dropped, not reported as garbage.
        assert!(decode_mappings("AAAD").is_empty());
    }

    /// Mappings mark region STARTS, so a lookup must find the last one at or before the
    /// column. An exact-match lookup would fail for almost every real position.
    #[test]
    fn lookup_finds_the_covering_mapping_not_an_exact_match() {
        let mappings = vec![
            Mapping {
                generated_line: 0,
                generated_column: 0,
                source_index: 0,
                original_line: 10,
                original_column: 0,
            },
            Mapping {
                generated_line: 0,
                generated_column: 50,
                source_index: 0,
                original_line: 20,
                original_column: 0,
            },
            Mapping {
                generated_line: 1,
                generated_column: 0,
                source_index: 0,
                original_line: 30,
                original_column: 0,
            },
        ];
        // Column 60 is inside the region that starts at 50.
        assert_eq!(lookup_mapping(&mappings, 0, 60).unwrap().original_line, 20);
        // Column 10 is inside the region that starts at 0.
        assert_eq!(lookup_mapping(&mappings, 0, 10).unwrap().original_line, 10);
        // A different line uses that line's mappings only.
        assert_eq!(lookup_mapping(&mappings, 1, 99).unwrap().original_line, 30);
        // A line with no mappings yields nothing rather than the nearest guess.
        assert!(lookup_mapping(&mappings, 5, 0).is_none());
    }
}
