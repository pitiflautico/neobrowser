//! Mapping a minified location back to the source someone wrote.
//!
//! A stack frame in `main.4f2a.js` at line 1 column 84213 is not actionable. Resolving it
//! needs a Base64-VLQ decoder for the source map's `mappings` field, which is implemented
//! here rather than pulled in as a dependency: it is about eighty lines of well-specified
//! bit manipulation, and it runs on data fetched from whatever page is being debugged —
//! which is exactly the kind of input where a small, readable, fuzz-tested decoder beats a
//! transitive dependency tree.

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

/// Base64 alphabet used by source-map VLQ.
const VLQ_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Decode one Base64-VLQ integer, returning the value and how many characters it used.
///
/// The encoding: each character carries 5 payload bits plus a continuation bit. In the
/// FIRST character the least-significant payload bit is the sign, which is the part
/// everyone gets wrong — a naive implementation reports negative deltas as huge
/// positives and every mapping after the first lands on the wrong line.
pub(super) fn vlq_decode(input: &[u8]) -> Option<(i64, usize)> {
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
