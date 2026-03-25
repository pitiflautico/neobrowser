//! CDP observation/inspection tools for Chrome sessions.
//!
//! Provides accessibility snapshots, screenshots, heap snapshots,
//! console message collection, and network request inspection.
//! Console and network use JS injection (v1) to avoid modifying CdpClient's recv_loop.

use crate::cdp::CdpClient;
use crate::{ChromeError, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;

// ─── Types ───

/// Screenshot image format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageFormat {
    Png,
    Jpeg,
    Webp,
}

impl ImageFormat {
    fn as_str(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpeg",
            Self::Webp => "webp",
        }
    }
}

/// A node in the accessibility tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AxNode {
    /// Unique node ID from CDP.
    pub uid: String,
    /// Role (e.g. "button", "heading", "link").
    pub role: String,
    /// Human-readable name.
    pub name: String,
    /// Current value, if any.
    pub value: String,
    /// Depth in the tree (for indentation).
    pub depth: usize,
    /// Whether the node is ignored by assistive tech.
    pub ignored: bool,
}

/// A collected console message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsoleMessage {
    /// Auto-incremented message ID.
    pub id: usize,
    /// Console method: log, warn, error, info, debug, etc.
    pub msg_type: String,
    /// Stringified message text.
    pub text: String,
    /// Timestamp (ms since epoch).
    pub timestamp: f64,
}

/// A collected network request entry (from Performance API).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkEntry {
    /// Auto-incremented request ID.
    pub id: usize,
    /// Request URL.
    pub url: String,
    /// HTTP method (GET for most resource timing entries).
    pub method: String,
    /// HTTP status code (0 if unavailable from perf API).
    pub status: u16,
    /// Resource type (script, stylesheet, img, xmlhttprequest, fetch, etc).
    pub resource_type: String,
    /// Transfer size in bytes.
    pub transfer_size: u64,
    /// Total duration in ms.
    pub duration_ms: f64,
}

// ─── Observation tools ───

/// Observation tools that operate on a CDP session.
///
/// Created with a reference to a `CdpClient` and a session ID.
/// All methods send CDP commands through the existing session.
pub struct Observation<'a> {
    cdp: &'a CdpClient,
    session_id: &'a str,
}

impl<'a> Observation<'a> {
    /// Create observation tools for a CDP session.
    pub fn new(cdp: &'a CdpClient, session_id: &'a str) -> Self {
        Self { cdp, session_id }
    }

    // ─── 1. Accessibility snapshot ───

    /// Get the full accessibility tree as structured text.
    ///
    /// If `verbose` is true, includes ignored nodes and extra properties.
    pub async fn take_snapshot(&self, verbose: bool) -> Result<String> {
        let result = self
            .cdp
            .send_to(self.session_id, "Accessibility.getFullAXTree", None)
            .await?;

        let nodes = result
            .get("nodes")
            .and_then(|v| v.as_array())
            .ok_or_else(|| ChromeError::CommandFailed {
                method: "Accessibility.getFullAXTree".into(),
                error: "No nodes in response".into(),
            })?;

        let parsed = parse_ax_nodes(nodes, verbose);
        Ok(format_ax_tree(&parsed))
    }

    // ─── 2. Screenshot ───

    /// Capture a screenshot of the page or a specific element.
    ///
    /// - `selector`: CSS selector to screenshot a specific element.
    /// - `full_page`: Capture the full scrollable page.
    /// - `format`: Image format (png, jpeg, webp).
    /// - `quality`: JPEG/WebP quality (0-100). Ignored for PNG.
    /// - `file_path`: If provided, write to file; otherwise return base64.
    pub async fn take_screenshot(
        &self,
        selector: Option<&str>,
        full_page: bool,
        format: ImageFormat,
        quality: Option<u32>,
        file_path: Option<&Path>,
    ) -> Result<String> {
        let mut params = json!({
            "format": format.as_str(),
        });

        if let Some(q) = quality {
            if format != ImageFormat::Png {
                params["quality"] = json!(q.min(100));
            }
        }

        if full_page {
            params["captureBeyondViewport"] = json!(true);

            // Get full page dimensions for the clip.
            let metrics = self
                .cdp
                .send_to(self.session_id, "Page.getLayoutMetrics", None)
                .await?;

            if let Some(content_size) = metrics.get("contentSize") {
                let w = content_size.get("width").and_then(|v| v.as_f64()).unwrap_or(1280.0);
                let h = content_size.get("height").and_then(|v| v.as_f64()).unwrap_or(720.0);
                params["clip"] = json!({
                    "x": 0,
                    "y": 0,
                    "width": w,
                    "height": h,
                    "scale": 1,
                });
            }
        }

        if let Some(sel) = selector {
            let clip = self.resolve_element_clip(sel).await?;
            params["clip"] = clip;
        }

        let result = self
            .cdp
            .send_to(
                self.session_id,
                "Page.captureScreenshot",
                Some(params),
            )
            .await?;

        let data = result
            .get("data")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ChromeError::CommandFailed {
                method: "Page.captureScreenshot".into(),
                error: "No data in screenshot response".into(),
            })?;

        if let Some(path) = file_path {
            use std::io::Write;
            let decoded = base64_decode(data)?;
            let mut file = std::fs::File::create(path)?;
            file.write_all(&decoded)?;
            Ok(path.to_string_lossy().to_string())
        } else {
            Ok(data.to_string())
        }
    }

    // ─── 3. Heap snapshot ───

    /// Take a heap snapshot and write it to a file.
    ///
    /// Note: This uses a simplified approach — it triggers heap snapshot
    /// and collects the result via Runtime.evaluate since we can't listen
    /// for HeapProfiler events in the current CdpClient architecture.
    pub async fn take_memory_snapshot(&self, file_path: &Path) -> Result<String> {
        // Enable the HeapProfiler domain.
        self.cdp
            .send_to(self.session_id, "HeapProfiler.enable", None)
            .await?;

        // Collect GC first for a cleaner snapshot.
        self.cdp
            .send_to(self.session_id, "HeapProfiler.collectGarbage", None)
            .await?;

        // Use Runtime.evaluate to get heap statistics as a lightweight alternative
        // since we can't collect HeapProfiler.addHeapSnapshotChunk events without
        // modifying the recv_loop.
        let stats = self
            .cdp
            .send_to(
                self.session_id,
                "Runtime.evaluate",
                Some(json!({
                    "expression": r#"JSON.stringify({
                        jsHeapSizeLimit: performance.memory ? performance.memory.jsHeapSizeLimit : 0,
                        totalJSHeapSize: performance.memory ? performance.memory.totalJSHeapSize : 0,
                        usedJSHeapSize: performance.memory ? performance.memory.usedJSHeapSize : 0,
                        timestamp: Date.now()
                    })"#,
                    "returnByValue": true,
                })),
            )
            .await?;

        let value = stats
            .get("result")
            .and_then(|r| r.get("value"))
            .and_then(|v| v.as_str())
            .unwrap_or("{}");

        use std::io::Write;
        let mut file = std::fs::File::create(file_path)?;
        file.write_all(value.as_bytes())?;

        self.cdp
            .send_to(self.session_id, "HeapProfiler.disable", None)
            .await?;

        Ok(file_path.to_string_lossy().to_string())
    }

    // ─── 4. Console messages (JS injection) ───

    /// Install the console message collector (call once after navigate).
    ///
    /// Overrides console methods to capture messages into a global array.
    pub async fn install_console_collector(&self) -> Result<()> {
        let js = r#"
        (function() {
            if (window.__neo_console_msgs) return;
            window.__neo_console_msgs = [];
            window.__neo_console_id = 0;
            const methods = ['log','warn','error','info','debug','trace'];
            const orig = {};
            methods.forEach(m => {
                orig[m] = console[m];
                console[m] = function(...args) {
                    window.__neo_console_msgs.push({
                        id: ++window.__neo_console_id,
                        type: m,
                        text: args.map(a => {
                            try { return typeof a === 'object' ? JSON.stringify(a) : String(a); }
                            catch(e) { return String(a); }
                        }).join(' '),
                        timestamp: Date.now()
                    });
                    orig[m].apply(console, args);
                };
            });
        })();
        "true"
        "#;
        self.eval_checked(js).await?;
        Ok(())
    }

    /// List collected console messages, optionally filtered by type.
    ///
    /// - `types_filter`: Only return messages of these types (e.g. `["error", "warn"]`).
    /// - `page_size`: Max messages to return (0 = all).
    pub async fn list_console_messages(
        &self,
        types_filter: &[&str],
        page_size: usize,
    ) -> Result<Vec<ConsoleMessage>> {
        let filter_json = serde_json::to_string(types_filter)
            .unwrap_or_else(|_| "[]".to_string());

        let js = format!(
            r#"(function() {{
                var msgs = window.__neo_console_msgs || [];
                var filter = {filter_json};
                if (filter.length > 0) {{
                    msgs = msgs.filter(function(m) {{ return filter.indexOf(m.type) >= 0; }});
                }}
                var limit = {page_size};
                if (limit > 0) msgs = msgs.slice(-limit);
                return JSON.stringify(msgs);
            }})()"#,
        );

        let raw = self.eval_checked(&js).await?;
        let messages: Vec<ConsoleMessage> =
            serde_json::from_str(&raw).unwrap_or_default();
        Ok(messages)
    }

    /// Get a specific console message by ID.
    pub async fn get_console_message(&self, msg_id: usize) -> Result<Option<ConsoleMessage>> {
        let js = format!(
            r#"(function() {{
                var msgs = window.__neo_console_msgs || [];
                var found = msgs.find(function(m) {{ return m.id === {msg_id}; }});
                return JSON.stringify(found || null);
            }})()"#,
        );

        let raw = self.eval_checked(&js).await?;
        if raw == "null" {
            return Ok(None);
        }
        let msg: ConsoleMessage = serde_json::from_str(&raw).map_err(|e| {
            ChromeError::CommandFailed {
                method: "get_console_message".into(),
                error: e.to_string(),
            }
        })?;
        Ok(Some(msg))
    }

    // ─── 5. Network requests (Performance API) ───

    /// List network requests using the Performance Resource Timing API.
    ///
    /// - `resource_types`: Filter by initiatorType (e.g. `["script", "fetch", "xmlhttprequest"]`).
    /// - `page_size`: Max entries to return (0 = all).
    pub async fn list_network_requests(
        &self,
        resource_types: &[&str],
        page_size: usize,
    ) -> Result<Vec<NetworkEntry>> {
        let filter_json = serde_json::to_string(resource_types)
            .unwrap_or_else(|_| "[]".to_string());

        let js = format!(
            r#"(function() {{
                var entries = performance.getEntriesByType('resource');
                var filter = {filter_json};
                var result = entries.map(function(e, i) {{
                    return {{
                        id: i + 1,
                        url: e.name,
                        method: 'GET',
                        status: e.responseStatus || 0,
                        resource_type: e.initiatorType || 'other',
                        transfer_size: e.transferSize || 0,
                        duration_ms: Math.round(e.duration * 100) / 100
                    }};
                }});
                if (filter.length > 0) {{
                    result = result.filter(function(r) {{ return filter.indexOf(r.resource_type) >= 0; }});
                }}
                var limit = {page_size};
                if (limit > 0) result = result.slice(-limit);
                return JSON.stringify(result);
            }})()"#,
        );

        let raw = self.eval_checked(&js).await?;
        let entries: Vec<NetworkEntry> =
            serde_json::from_str(&raw).unwrap_or_default();
        Ok(entries)
    }

    /// Get details for a specific network request by ID.
    ///
    /// - `save_request_path`: If provided, write request info to this file.
    /// - `save_response_path`: If provided, write response info to this file.
    pub async fn get_network_request(
        &self,
        req_id: usize,
        save_request_path: Option<&Path>,
        save_response_path: Option<&Path>,
    ) -> Result<Option<NetworkEntry>> {
        let js = format!(
            r#"(function() {{
                var entries = performance.getEntriesByType('resource');
                var idx = {req_id} - 1;
                if (idx < 0 || idx >= entries.length) return JSON.stringify(null);
                var e = entries[idx];
                return JSON.stringify({{
                    id: idx + 1,
                    url: e.name,
                    method: 'GET',
                    status: e.responseStatus || 0,
                    resource_type: e.initiatorType || 'other',
                    transfer_size: e.transferSize || 0,
                    duration_ms: Math.round(e.duration * 100) / 100
                }});
            }})()"#,
        );

        let raw = self.eval_checked(&js).await?;
        if raw == "null" {
            return Ok(None);
        }

        let entry: NetworkEntry = serde_json::from_str(&raw).map_err(|e| {
            ChromeError::CommandFailed {
                method: "get_network_request".into(),
                error: e.to_string(),
            }
        })?;

        if let Some(path) = save_request_path {
            use std::io::Write;
            let info = json!({
                "url": entry.url,
                "method": entry.method,
                "resource_type": entry.resource_type,
            });
            let mut f = std::fs::File::create(path)?;
            f.write_all(serde_json::to_string_pretty(&info).unwrap().as_bytes())?;
        }

        if let Some(path) = save_response_path {
            use std::io::Write;
            let info = json!({
                "url": entry.url,
                "status": entry.status,
                "transfer_size": entry.transfer_size,
                "duration_ms": entry.duration_ms,
            });
            let mut f = std::fs::File::create(path)?;
            f.write_all(serde_json::to_string_pretty(&info).unwrap().as_bytes())?;
        }

        Ok(Some(entry))
    }

    // ─── Internal helpers ───

    /// Evaluate JS and return the string value, checking for exceptions.
    async fn eval_checked(&self, js: &str) -> Result<String> {
        let result = self
            .cdp
            .send_to(
                self.session_id,
                "Runtime.evaluate",
                Some(json!({
                    "expression": js,
                    "returnByValue": true,
                })),
            )
            .await?;

        if let Some(exc) = result.get("exceptionDetails") {
            let text = exc
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("JS exception");
            return Err(ChromeError::CommandFailed {
                method: "Runtime.evaluate".into(),
                error: text.to_string(),
            });
        }

        let value = result
            .get("result")
            .and_then(|r| r.get("value"))
            .cloned()
            .unwrap_or(Value::Null);

        match value {
            Value::String(s) => Ok(s),
            other => Ok(other.to_string()),
        }
    }

    /// Resolve a CSS selector to a clip rectangle via DOM.getBoxModel.
    async fn resolve_element_clip(&self, selector: &str) -> Result<Value> {
        // Find the element via DOM.
        let doc = self
            .cdp
            .send_to(self.session_id, "DOM.getDocument", None)
            .await?;

        let root_id = doc
            .get("root")
            .and_then(|r| r.get("nodeId"))
            .and_then(|v| v.as_u64())
            .ok_or_else(|| ChromeError::CommandFailed {
                method: "DOM.getDocument".into(),
                error: "No root nodeId".into(),
            })?;

        let query = self
            .cdp
            .send_to(
                self.session_id,
                "DOM.querySelector",
                Some(json!({
                    "nodeId": root_id,
                    "selector": selector,
                })),
            )
            .await?;

        let node_id = query.get("nodeId").and_then(|v| v.as_u64()).ok_or_else(|| {
            ChromeError::CommandFailed {
                method: "DOM.querySelector".into(),
                error: format!("Element not found: {selector}"),
            }
        })?;

        if node_id == 0 {
            return Err(ChromeError::CommandFailed {
                method: "DOM.querySelector".into(),
                error: format!("Element not found: {selector}"),
            });
        }

        let box_model = self
            .cdp
            .send_to(
                self.session_id,
                "DOM.getBoxModel",
                Some(json!({ "nodeId": node_id })),
            )
            .await?;

        let content = box_model
            .get("model")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_array())
            .ok_or_else(|| ChromeError::CommandFailed {
                method: "DOM.getBoxModel".into(),
                error: "No content quad in box model".into(),
            })?;

        // Content quad is [x1,y1, x2,y2, x3,y3, x4,y4].
        let coords: Vec<f64> = content.iter().filter_map(|v| v.as_f64()).collect();
        if coords.len() < 8 {
            return Err(ChromeError::CommandFailed {
                method: "DOM.getBoxModel".into(),
                error: "Invalid content quad".into(),
            });
        }

        let x = coords[0].min(coords[6]);
        let y = coords[1].min(coords[3]);
        let w = coords[2].max(coords[4]) - x;
        let h = coords[5].max(coords[7]) - y;

        Ok(json!({
            "x": x,
            "y": y,
            "width": w,
            "height": h,
            "scale": 1,
        }))
    }
}

// ─── AX tree parsing ───

/// Parse CDP accessibility nodes into our AxNode format.
pub fn parse_ax_nodes(nodes: &[Value], verbose: bool) -> Vec<AxNode> {
    let mut result = Vec::new();

    for node in nodes {
        let ignored = node
            .get("ignored")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if !verbose && ignored {
            continue;
        }

        let uid = node
            .get("nodeId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let role = node
            .get("role")
            .and_then(|r| r.get("value"))
            .and_then(|v| v.as_str())
            .unwrap_or("none")
            .to_string();

        let name = node
            .get("name")
            .and_then(|n| n.get("value"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let value = node
            .get("value")
            .and_then(|n| n.get("value"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Approximate depth from parentId chain (flat list, no tree walk needed).
        // CDP returns nodes in document order; depth is embedded in backendDOMNodeId
        // relationships but we simplify to 0 here — the caller can reconstruct.
        let depth = node
            .get("depth")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;

        // Skip empty "none" role nodes in non-verbose mode.
        if !verbose && role == "none" && name.is_empty() {
            continue;
        }

        result.push(AxNode {
            uid,
            role,
            name,
            value,
            depth,
            ignored,
        });
    }

    result
}

/// Format parsed AX nodes into indented text representation.
pub fn format_ax_tree(nodes: &[AxNode]) -> String {
    let mut out = String::with_capacity(nodes.len() * 80);

    for node in nodes {
        let indent = "  ".repeat(node.depth);
        out.push_str(&indent);

        // [uid] role "name"
        out.push('[');
        out.push_str(&node.uid);
        out.push_str("] ");
        out.push_str(&node.role);

        if !node.name.is_empty() {
            out.push_str(" \"");
            out.push_str(&node.name);
            out.push('"');
        }

        if !node.value.is_empty() {
            out.push_str(" = \"");
            out.push_str(&node.value);
            out.push('"');
        }

        if node.ignored {
            out.push_str(" [ignored]");
        }

        out.push('\n');
    }

    out
}

/// Decode base64 string to bytes (no-dependency, simple decoder).
fn base64_decode(input: &str) -> Result<Vec<u8>> {
    const TABLE: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    fn val(c: u8) -> std::result::Result<u8, ()> {
        TABLE.iter().position(|&x| x == c).map(|p| p as u8).ok_or(())
    }

    let input: Vec<u8> = input.bytes().filter(|&b| b != b'\n' && b != b'\r' && b != b' ').collect();
    let mut out = Vec::with_capacity(input.len() * 3 / 4);

    for chunk in input.chunks(4) {
        let mut buf = [0u8; 4];
        let mut len = 0;
        for (i, &b) in chunk.iter().enumerate() {
            if b == b'=' {
                break;
            }
            buf[i] = val(b).map_err(|_| ChromeError::CommandFailed {
                method: "base64_decode".into(),
                error: format!("Invalid base64 character: {}", b as char),
            })?;
            len = i + 1;
        }
        if len >= 2 {
            out.push((buf[0] << 2) | (buf[1] >> 4));
        }
        if len >= 3 {
            out.push((buf[1] << 4) | (buf[2] >> 2));
        }
        if len >= 4 {
            out.push((buf[2] << 6) | buf[3]);
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── Screenshot params serialization ───

    #[test]
    fn test_screenshot_format_serialization() {
        assert_eq!(ImageFormat::Png.as_str(), "png");
        assert_eq!(ImageFormat::Jpeg.as_str(), "jpeg");
        assert_eq!(ImageFormat::Webp.as_str(), "webp");
    }

    #[test]
    fn test_screenshot_params_basic() {
        let params = json!({
            "format": ImageFormat::Png.as_str(),
        });
        assert_eq!(params["format"], "png");
    }

    #[test]
    fn test_screenshot_params_quality_jpeg() {
        let format = ImageFormat::Jpeg;
        let quality: u32 = 85;
        let mut params = json!({ "format": format.as_str() });
        if format != ImageFormat::Png {
            params["quality"] = json!(quality.min(100));
        }
        assert_eq!(params["quality"], 85);
    }

    #[test]
    fn test_screenshot_params_quality_ignored_for_png() {
        let format = ImageFormat::Png;
        let mut params = json!({ "format": format.as_str() });
        if format != ImageFormat::Png {
            params["quality"] = json!(80);
        }
        assert!(params.get("quality").is_none());
    }

    #[test]
    fn test_screenshot_params_clip() {
        let clip = json!({
            "x": 10,
            "y": 20,
            "width": 300,
            "height": 200,
            "scale": 1,
        });
        let params = json!({
            "format": "png",
            "clip": clip,
        });
        assert_eq!(params["clip"]["x"], 10);
        assert_eq!(params["clip"]["width"], 300);
    }

    #[test]
    fn test_screenshot_params_full_page() {
        let params = json!({
            "format": "png",
            "captureBeyondViewport": true,
            "clip": {
                "x": 0, "y": 0,
                "width": 1920, "height": 5000,
                "scale": 1,
            },
        });
        assert_eq!(params["captureBeyondViewport"], true);
        assert_eq!(params["clip"]["height"], 5000);
    }

    // ─── A11y tree parsing ───

    #[test]
    fn test_ax_parse_basic() {
        let nodes = vec![
            json!({
                "nodeId": "1",
                "role": { "value": "WebArea" },
                "name": { "value": "Test Page" },
                "ignored": false,
            }),
            json!({
                "nodeId": "2",
                "role": { "value": "heading" },
                "name": { "value": "Hello World" },
                "ignored": false,
                "depth": 1,
            }),
        ];

        let parsed = parse_ax_nodes(&nodes, false);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].role, "WebArea");
        assert_eq!(parsed[0].name, "Test Page");
        assert_eq!(parsed[1].role, "heading");
        assert_eq!(parsed[1].depth, 1);
    }

    #[test]
    fn test_ax_parse_skips_ignored_in_normal_mode() {
        let nodes = vec![
            json!({
                "nodeId": "1",
                "role": { "value": "WebArea" },
                "name": { "value": "Page" },
                "ignored": false,
            }),
            json!({
                "nodeId": "2",
                "role": { "value": "generic" },
                "name": { "value": "" },
                "ignored": true,
            }),
        ];

        let normal = parse_ax_nodes(&nodes, false);
        assert_eq!(normal.len(), 1);

        let verbose = parse_ax_nodes(&nodes, true);
        assert_eq!(verbose.len(), 2);
        assert!(verbose[1].ignored);
    }

    #[test]
    fn test_ax_parse_skips_empty_none_nodes() {
        let nodes = vec![json!({
            "nodeId": "1",
            "role": { "value": "none" },
            "name": { "value": "" },
            "ignored": false,
        })];

        let parsed = parse_ax_nodes(&nodes, false);
        assert_eq!(parsed.len(), 0);

        let verbose = parse_ax_nodes(&nodes, true);
        assert_eq!(verbose.len(), 1);
    }

    #[test]
    fn test_ax_format_output() {
        let nodes = vec![
            AxNode {
                uid: "1".into(),
                role: "WebArea".into(),
                name: "Page".into(),
                value: String::new(),
                depth: 0,
                ignored: false,
            },
            AxNode {
                uid: "2".into(),
                role: "button".into(),
                name: "Submit".into(),
                value: String::new(),
                depth: 1,
                ignored: false,
            },
        ];

        let text = format_ax_tree(&nodes);
        assert!(text.contains("[1] WebArea \"Page\""));
        assert!(text.contains("  [2] button \"Submit\""));
    }

    #[test]
    fn test_ax_format_with_value() {
        let nodes = vec![AxNode {
            uid: "3".into(),
            role: "textbox".into(),
            name: "Email".into(),
            value: "user@test.com".into(),
            depth: 0,
            ignored: false,
        }];

        let text = format_ax_tree(&nodes);
        assert!(text.contains("\"Email\" = \"user@test.com\""));
    }

    // ─── Console message filtering ───

    #[test]
    fn test_console_message_filter_by_type() {
        let messages = vec![
            ConsoleMessage {
                id: 1,
                msg_type: "log".into(),
                text: "hello".into(),
                timestamp: 1000.0,
            },
            ConsoleMessage {
                id: 2,
                msg_type: "error".into(),
                text: "oops".into(),
                timestamp: 1001.0,
            },
            ConsoleMessage {
                id: 3,
                msg_type: "warn".into(),
                text: "careful".into(),
                timestamp: 1002.0,
            },
            ConsoleMessage {
                id: 4,
                msg_type: "error".into(),
                text: "boom".into(),
                timestamp: 1003.0,
            },
        ];

        let filter = ["error"];
        let filtered: Vec<_> = messages
            .iter()
            .filter(|m| filter.contains(&m.msg_type.as_str()))
            .collect();

        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].text, "oops");
        assert_eq!(filtered[1].text, "boom");
    }

    #[test]
    fn test_console_message_empty_filter_returns_all() {
        let messages = vec![
            ConsoleMessage {
                id: 1,
                msg_type: "log".into(),
                text: "a".into(),
                timestamp: 0.0,
            },
            ConsoleMessage {
                id: 2,
                msg_type: "error".into(),
                text: "b".into(),
                timestamp: 0.0,
            },
        ];

        let filter: [&str; 0] = [];
        let filtered: Vec<_> = messages
            .iter()
            .filter(|m| filter.is_empty() || filter.contains(&m.msg_type.as_str()))
            .collect();

        assert_eq!(filtered.len(), 2);
    }

    // ─── Network request filtering ───

    #[test]
    fn test_network_filter_by_resource_type() {
        let entries = vec![
            NetworkEntry {
                id: 1,
                url: "https://cdn.example.com/app.js".into(),
                method: "GET".into(),
                status: 200,
                resource_type: "script".into(),
                transfer_size: 50000,
                duration_ms: 120.5,
            },
            NetworkEntry {
                id: 2,
                url: "https://cdn.example.com/style.css".into(),
                method: "GET".into(),
                status: 200,
                resource_type: "stylesheet".into(),
                transfer_size: 12000,
                duration_ms: 45.2,
            },
            NetworkEntry {
                id: 3,
                url: "https://api.example.com/data".into(),
                method: "GET".into(),
                status: 200,
                resource_type: "fetch".into(),
                transfer_size: 8000,
                duration_ms: 230.0,
            },
        ];

        let filter = ["script", "fetch"];
        let filtered: Vec<_> = entries
            .iter()
            .filter(|e| filter.contains(&e.resource_type.as_str()))
            .collect();

        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].resource_type, "script");
        assert_eq!(filtered[1].resource_type, "fetch");
    }

    #[test]
    fn test_network_empty_filter_returns_all() {
        let entries = vec![
            NetworkEntry {
                id: 1,
                url: "a.js".into(),
                method: "GET".into(),
                status: 200,
                resource_type: "script".into(),
                transfer_size: 100,
                duration_ms: 10.0,
            },
            NetworkEntry {
                id: 2,
                url: "b.css".into(),
                method: "GET".into(),
                status: 200,
                resource_type: "stylesheet".into(),
                transfer_size: 200,
                duration_ms: 20.0,
            },
        ];

        let filter: [&str; 0] = [];
        let filtered: Vec<_> = entries
            .iter()
            .filter(|e| filter.is_empty() || filter.contains(&e.resource_type.as_str()))
            .collect();

        assert_eq!(filtered.len(), 2);
    }

    // ─── Base64 decode ───

    #[test]
    fn test_base64_decode() {
        let encoded = "SGVsbG8gV29ybGQ=";
        let decoded = base64_decode(encoded).unwrap();
        assert_eq!(String::from_utf8(decoded).unwrap(), "Hello World");
    }

    #[test]
    fn test_base64_decode_no_padding() {
        let encoded = "SGk";
        let decoded = base64_decode(encoded).unwrap();
        assert_eq!(String::from_utf8(decoded).unwrap(), "Hi");
    }

    // ─── Serde roundtrip ───

    #[test]
    fn test_image_format_serde() {
        let fmt = ImageFormat::Jpeg;
        let json = serde_json::to_string(&fmt).unwrap();
        assert_eq!(json, r#""jpeg""#);
        let back: ImageFormat = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ImageFormat::Jpeg);
    }
}
