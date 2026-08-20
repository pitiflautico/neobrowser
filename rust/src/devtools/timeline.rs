//! Why the page is slow, and what it is spending time on.
//!
//! A raw performance trace is thousands of events and useless to a model, so `perf_trace`
//! derives the handful of statements a human would actually make from it — this many long
//! tasks, this much layout thrash. `cpu_profile` and `heap_stats` answer the two follow-up
//! questions: which function, and is memory growing.

use serde_json::{json, Value};

use crate::cdp::{CdpClient, CdpError};

/// `perf_trace` — Web Vitals, navigation timing and the slowest resources.
///
/// Returns an interpretation alongside the raw numbers: a table of milliseconds is
/// data, and the point of a debugging tool is to say which number is the problem.
pub async fn perf_trace(client: &CdpClient) -> Result<String, CdpError> {
    let raw = crate::page::eval_body(client, &crate::js::vitals().returning()).await?;
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
pub(super) fn derive_insights(v: &Value) -> Vec<String> {
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
