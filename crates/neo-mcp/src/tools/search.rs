//! `search` tool — AI-optimized web search via DuckDuckGo.
//!
//! Fetches DDG HTML search results, parses them with html5ever, and
//! optionally deep-fetches top N result pages for content extraction.

use std::collections::HashMap;
use std::time::Instant;

use html5ever::tendril::TendrilSink;
use markup5ever_rcdom::{Handle, NodeData, RcDom};
use serde_json::Value;

use neo_http::{HttpClient, HttpRequest, RequestContext, RequestKind, RquestClient};

use crate::state::McpState;
use crate::McpError;

use super::ToolDef;

/// Tool definition for `tools/list`.
pub(crate) fn definition() -> ToolDef {
    ToolDef {
        name: "search",
        description: "Search the web using DuckDuckGo. Returns titles, URLs, snippets. \
                       Use deep=true to also fetch and extract text from top results.",
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query"
                },
                "num": {
                    "type": "integer",
                    "description": "Max results to return (default 10, max 30)",
                    "default": 10
                },
                "region": {
                    "type": "string",
                    "description": "Region code for DuckDuckGo (e.g. 'es-es', 'us-en')"
                },
                "deep": {
                    "type": "boolean",
                    "description": "Fetch top result pages and extract text content",
                    "default": false
                },
                "deep_num": {
                    "type": "integer",
                    "description": "How many pages to deep-fetch (default 3, max 5)",
                    "default": 3
                },
                "deep_chars": {
                    "type": "integer",
                    "description": "Max chars to extract per deep page (default 800, max 2000)",
                    "default": 800
                }
            },
            "required": ["query"]
        }),
    }
}

/// Execute the `search` tool.
///
/// This tool operates independently from the BrowserEngine — it creates
/// its own HTTP client and parses HTML directly with html5ever.
pub fn call(args: Value, _state: &mut McpState) -> Result<Value, McpError> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::InvalidParams("missing 'query'".into()))?;

    let num = args
        .get("num")
        .and_then(|v| v.as_u64())
        .unwrap_or(10)
        .min(30) as usize;

    let region = args
        .get("region")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let deep = args
        .get("deep")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let deep_num = args
        .get("deep_num")
        .and_then(|v| v.as_u64())
        .unwrap_or(3)
        .min(5) as usize;

    let deep_chars = args
        .get("deep_chars")
        .and_then(|v| v.as_u64())
        .unwrap_or(800)
        .min(2000) as usize;

    let start = Instant::now();

    // Build DuckDuckGo HTML search URL
    let encoded_query = query.replace(' ', "+");
    let url = if region.is_empty() {
        format!("https://html.duckduckgo.com/html/?q={encoded_query}")
    } else {
        format!("https://html.duckduckgo.com/html/?q={encoded_query}&kl={region}")
    };

    // Fetch with Chrome TLS
    let client = RquestClient::default_client()
        .map_err(|e| McpError::InvalidParams(format!("http client init: {e}")))?;

    let html = fetch_html(&client, &url)?;
    if html.is_empty() {
        return Ok(serde_json::json!({
            "ok": false,
            "error": "Empty response from DuckDuckGo",
            "query": query
        }));
    }

    // Parse HTML with html5ever
    let dom = parse_html(&html)?;

    // Extract search results from DDG HTML structure
    let mut results: Vec<Value> = Vec::new();
    extract_ddg_results(&dom.document, &mut results);
    results.truncate(num);

    // Deep mode: fetch top N pages in parallel and extract text
    if deep && !results.is_empty() {
        let urls_to_fetch: Vec<String> = results
            .iter()
            .take(deep_num)
            .filter_map(|r| r["url"].as_str().map(|s| s.to_string()))
            .collect();

        let fetched = deep_fetch(&urls_to_fetch, deep_chars);

        for (page_url, content) in fetched {
            if !content.is_empty() {
                for r in results.iter_mut() {
                    if r["url"].as_str() == Some(&page_url) {
                        if let Some(obj) = r.as_object_mut() {
                            obj.insert("content".to_string(), Value::String(content));
                        }
                        break;
                    }
                }
            }
        }
    }

    let elapsed_ms = start.elapsed().as_millis();

    Ok(serde_json::json!({
        "ok": true,
        "query": query,
        "results": results,
        "count": results.len(),
        "engine": "duckduckgo",
        "deep": deep,
        "elapsed_ms": elapsed_ms,
    }))
}

// ─── HTTP helpers ───

/// Fetch a URL and return the response body as a string.
fn fetch_html(client: &RquestClient, url: &str) -> Result<String, McpError> {
    let req = HttpRequest {
        method: "GET".into(),
        url: url.into(),
        headers: HashMap::from([(
            "Accept".into(),
            "text/html,application/xhtml+xml,*/*".into(),
        )]),
        body: None,
        context: RequestContext {
            kind: RequestKind::Navigation,
            initiator: "search".into(),
            referrer: None,
            frame_id: None,
            top_level_url: None,
        },
        timeout_ms: 10_000,
    };

    let resp = client
        .request(&req)
        .map_err(|e| McpError::InvalidParams(format!("fetch failed: {e}")))?;

    Ok(resp.body)
}

/// Parse HTML string into an RcDom.
fn parse_html(html: &str) -> Result<RcDom, McpError> {
    html5ever::parse_document(RcDom::default(), html5ever::ParseOpts::default())
        .from_utf8()
        .read_from(&mut html.as_bytes())
        .map_err(|e| McpError::InvalidParams(format!("HTML parse error: {e}")))
}

// ─── DDG result extraction ───

/// Walk the DDG HTML DOM and extract search results.
///
/// DDG HTML structure:
///   div.result.web-result > div.links_main.result__body >
///     h2.result__title > a.result__a [href, text=title]
///     a.result__snippet [text=snippet]
fn extract_ddg_results(node: &Handle, results: &mut Vec<Value>) {
    let (tag, class) = tag_and_class(node);

    // Found a result container
    if tag == "div" && class.contains("result") && class.contains("web-result") {
        let mut title = String::new();
        let mut href = String::new();
        let mut snippet = String::new();

        extract_ddg_result_fields(node, &mut title, &mut href, &mut snippet, 0);

        // Clean up href: DDG wraps in redirect URLs
        let clean_url = clean_ddg_url(&href);
        let title = title.trim().to_string();
        let snippet = snippet.trim().to_string();

        if !title.is_empty() && !clean_url.is_empty() {
            results.push(serde_json::json!({
                "title": title,
                "url": clean_url,
                "snippet": snippet,
            }));
        }
    }

    // Recurse into children
    for child in node.children.borrow().iter() {
        extract_ddg_results(child, results);
    }
}

/// Extract title, href, and snippet from a DDG result container.
fn extract_ddg_result_fields(
    node: &Handle,
    title: &mut String,
    href: &mut String,
    snippet: &mut String,
    depth: usize,
) {
    if depth > 15 {
        return;
    }

    let (tag, class) = tag_and_class(node);

    // Title link: a.result__a
    if tag == "a" && class.contains("result__a") {
        *title = collect_text(node).trim().to_string();
        let h = get_attr(node, "href");
        if !h.is_empty() {
            *href = h;
        }
    }

    // Snippet: a.result__snippet
    if tag == "a" && class.contains("result__snippet") {
        *snippet = collect_text(node).trim().to_string();
    }

    for child in node.children.borrow().iter() {
        extract_ddg_result_fields(child, title, href, snippet, depth + 1);
    }
}

/// Clean DDG redirect URL, extracting the actual target URL.
///
/// DDG wraps results in `//duckduckgo.com/l/?uddg=<encoded_url>&...`.
fn clean_ddg_url(href: &str) -> String {
    if let Some(pos) = href.find("uddg=") {
        let encoded = &href[pos + 5..];
        let end = encoded.find('&').unwrap_or(encoded.len());
        percent_decode(&encoded[..end])
    } else {
        href.to_string()
    }
}

// ─── DOM helpers (lightweight, no dependency on neo-dom internals) ───

/// Get tag name and class attribute from a node.
fn tag_and_class(handle: &Handle) -> (String, String) {
    match &handle.data {
        NodeData::Element {
            ref name,
            ref attrs,
            ..
        } => {
            let tag = name.local.to_string();
            let class = attrs
                .borrow()
                .iter()
                .find(|a| a.name.local.as_ref() == "class")
                .map(|a| a.value.to_string())
                .unwrap_or_default();
            (tag, class)
        }
        _ => (String::new(), String::new()),
    }
}

/// Get an attribute value from a node.
fn get_attr(handle: &Handle, name: &str) -> String {
    match &handle.data {
        NodeData::Element { ref attrs, .. } => attrs
            .borrow()
            .iter()
            .find(|a| a.name.local.as_ref() == name)
            .map(|a| a.value.to_string())
            .unwrap_or_default(),
        _ => String::new(),
    }
}

/// Collect all text content from a DOM subtree.
fn collect_text(handle: &Handle) -> String {
    let mut buf = String::new();
    collect_text_recursive(handle, &mut buf);
    buf
}

fn collect_text_recursive(handle: &Handle, buf: &mut String) {
    match &handle.data {
        NodeData::Text { contents } => {
            buf.push_str(&contents.borrow());
        }
        _ => {
            for child in handle.children.borrow().iter() {
                collect_text_recursive(child, buf);
            }
        }
    }
}

// ─── Deep fetch ───

/// Fetch multiple URLs in parallel using thread::spawn and extract text.
///
/// Returns a vec of (url, extracted_text) pairs.
fn deep_fetch(urls: &[String], max_chars: usize) -> Vec<(String, String)> {
    let handles: Vec<_> = urls
        .iter()
        .map(|url| {
            let url = url.clone();
            let mc = max_chars;
            std::thread::spawn(move || {
                let client = match RquestClient::default_client() {
                    Ok(c) => c,
                    Err(_) => return (url, String::new()),
                };
                let html = match fetch_page(&client, &url) {
                    Ok(h) => h,
                    Err(_) => return (url, String::new()),
                };
                if html.is_empty() {
                    return (url, String::new());
                }
                let dom = match parse_html(&html) {
                    Ok(d) => d,
                    Err(_) => return (url, String::new()),
                };
                let text = collect_text(&dom.document);
                let clean: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
                let truncated = if clean.len() > mc {
                    let boundary = floor_char_boundary(&clean, mc);
                    format!("{}…", &clean[..boundary])
                } else {
                    clean
                };
                (url, truncated)
            })
        })
        .collect();

    handles
        .into_iter()
        .filter_map(|h| h.join().ok())
        .collect()
}

/// Fetch a page and return its body. Separate from fetch_html to avoid
/// the McpError coupling — deep-fetch failures are silently ignored.
fn fetch_page(client: &RquestClient, url: &str) -> Result<String, neo_http::HttpError> {
    let req = HttpRequest {
        method: "GET".into(),
        url: url.into(),
        headers: HashMap::from([(
            "Accept".into(),
            "text/html,application/xhtml+xml,*/*".into(),
        )]),
        body: None,
        context: RequestContext {
            kind: RequestKind::Navigation,
            initiator: "search-deep".into(),
            referrer: None,
            frame_id: None,
            top_level_url: None,
        },
        timeout_ms: 10_000,
    };
    let resp = client.request(&req)?;
    Ok(resp.body)
}

/// Find the largest char boundary <= `max` in a string.
fn floor_char_boundary(s: &str, max: usize) -> usize {
    if max >= s.len() {
        return s.len();
    }
    let mut i = max;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Percent-decode a URL-encoded string.
fn percent_decode(input: &str) -> String {
    let mut result = Vec::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (
                hex_val(bytes[i + 1]),
                hex_val(bytes[i + 2]),
            ) {
                result.push(hi << 4 | lo);
                i += 3;
                continue;
            }
        } else if bytes[i] == b'+' {
            result.push(b' ');
            i += 1;
            continue;
        }
        result.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(result).unwrap_or_else(|_| input.to_string())
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    /// Mock DDG HTML page for unit testing.
    const MOCK_DDG_HTML: &str = r#"<!DOCTYPE html>
<html>
<head><title>DuckDuckGo</title></head>
<body>
<div id="links">
  <div class="result results_links results_links_deep web-result">
    <div class="links_main links_deep result__body">
      <h2 class="result__title">
        <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fwww.rust-lang.org%2F&amp;rut=abc">
          Rust Programming Language
        </a>
      </h2>
      <a class="result__snippet" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fwww.rust-lang.org%2F">
        A language empowering everyone to build reliable and efficient software.
      </a>
    </div>
  </div>
  <div class="result results_links results_links_deep web-result">
    <div class="links_main links_deep result__body">
      <h2 class="result__title">
        <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fdoc.rust-lang.org%2Fbook%2F&amp;rut=def">
          The Rust Book
        </a>
      </h2>
      <a class="result__snippet" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fdoc.rust-lang.org%2Fbook%2F">
        The Rust Programming Language book, an introductory guide.
      </a>
    </div>
  </div>
  <div class="result results_links results_links_deep web-result">
    <div class="links_main links_deep result__body">
      <h2 class="result__title">
        <a rel="nofollow" class="result__a" href="https://plain-url.example.com/page">
          Plain URL Result
        </a>
      </h2>
      <a class="result__snippet" href="https://plain-url.example.com/page">
        This result has no DDG redirect wrapper.
      </a>
    </div>
  </div>
</div>
</body>
</html>"#;

    #[test]
    fn test_parse_ddg_results() {
        let dom = parse_html(MOCK_DDG_HTML).expect("parse failed");
        let mut results = Vec::new();
        extract_ddg_results(&dom.document, &mut results);

        assert_eq!(results.len(), 3, "should find 3 results");

        // First result
        assert_eq!(
            results[0]["title"].as_str().unwrap().trim(),
            "Rust Programming Language"
        );
        assert_eq!(
            results[0]["url"].as_str().unwrap(),
            "https://www.rust-lang.org/"
        );
        assert!(results[0]["snippet"]
            .as_str()
            .unwrap()
            .contains("reliable and efficient"));

        // Second result
        assert_eq!(
            results[1]["title"].as_str().unwrap().trim(),
            "The Rust Book"
        );
        assert_eq!(
            results[1]["url"].as_str().unwrap(),
            "https://doc.rust-lang.org/book/"
        );
    }

    #[test]
    fn test_clean_ddg_url_with_redirect() {
        let href = "//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fpath&rut=xyz";
        assert_eq!(clean_ddg_url(href), "https://example.com/path");
    }

    #[test]
    fn test_clean_ddg_url_plain() {
        let href = "https://example.com/direct";
        assert_eq!(clean_ddg_url(href), "https://example.com/direct");
    }

    #[test]
    fn test_percent_decode() {
        assert_eq!(
            percent_decode("https%3A%2F%2Fexample.com%2Fpath%3Fq%3Dhello+world"),
            "https://example.com/path?q=hello world"
        );
    }

    #[test]
    fn test_truncate_num() {
        let dom = parse_html(MOCK_DDG_HTML).expect("parse failed");
        let mut results = Vec::new();
        extract_ddg_results(&dom.document, &mut results);
        results.truncate(1);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_plain_url_not_redirected() {
        let dom = parse_html(MOCK_DDG_HTML).expect("parse failed");
        let mut results = Vec::new();
        extract_ddg_results(&dom.document, &mut results);

        assert_eq!(
            results[2]["url"].as_str().unwrap(),
            "https://plain-url.example.com/page"
        );
    }

    #[test]
    fn test_deep_mode_structure() {
        // Verify the JSON shape when deep content is added
        let mut result = serde_json::json!({
            "title": "Test",
            "url": "https://example.com",
            "snippet": "A test result"
        });

        if let Some(obj) = result.as_object_mut() {
            obj.insert(
                "content".to_string(),
                Value::String("Deep fetched content here".into()),
            );
        }

        assert!(result["content"].as_str().is_some());
        assert_eq!(result["content"].as_str().unwrap(), "Deep fetched content here");
    }

    #[test]
    fn test_empty_html_no_results() {
        let dom = parse_html("<html><body></body></html>").expect("parse failed");
        let mut results = Vec::new();
        extract_ddg_results(&dom.document, &mut results);
        assert!(results.is_empty());
    }

    #[test]
    fn test_floor_char_boundary() {
        assert_eq!(floor_char_boundary("hello", 3), 3);
        assert_eq!(floor_char_boundary("hello", 100), 5);
        // Multi-byte: "é" is 2 bytes in UTF-8, "café" = [c,a,f,0xC3,0xA9]
        let s = "café";
        assert_eq!(floor_char_boundary(s, 4), 3); // byte 4 is mid-char, backs up to 3
        assert_eq!(floor_char_boundary(s, 3), 3); // lands on 'é' start
        assert_eq!(floor_char_boundary(s, 5), 5); // full string
    }
}
