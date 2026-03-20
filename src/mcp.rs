//! MCP Server — JSON-RPC over stdio.
//!
//! Implements the Model Context Protocol for AI agents.
//! 6 core tools: open, observe, act, wait, tabs, session.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{self, BufRead, Write};

use crate::auth;
use crate::engine;
use crate::delta;
use crate::ghost;
use crate::wom;

use html5ever::tendril::TendrilSink;

// ─── Session cache — persists cookies per domain for neorender reuse ───

fn session_cache_dir() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let dir = std::path::PathBuf::from(&home).join(".neobrowser").join("sessions");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn domain_from_url(url: &str) -> String {
    url::Url::parse(url).ok()
        .and_then(|u| u.host_str().map(|s| s.to_string()))
        .unwrap_or_default()
}

fn session_cache_path(domain: &str) -> std::path::PathBuf {
    session_cache_dir().join(format!("{}.json", domain.replace('.', "_")))
}

fn load_session_cache(domain: &str) -> Option<String> {
    let path = session_cache_path(domain);
    if path.exists() {
        eprintln!("[SESSION] Found cached session for {}", domain);
        Some(path.to_string_lossy().to_string())
    } else {
        None
    }
}

/// Save current Chrome session cookies to domain cache
async fn save_session_to_cache(session: &engine::Session, url: &str) -> Result<(), String> {
    let domain = domain_from_url(url);
    if domain.is_empty() { return Ok(()); }

    let path = session_cache_path(&domain);

    // Export cookies via CDP
    let cookies = session.get_all_cookies().await.unwrap_or_default();
    if cookies.is_empty() { return Ok(()); }

    // Also grab localStorage
    let ls_result = session.eval_string(
        "JSON.stringify(Object.fromEntries(Object.entries(localStorage)))"
    ).await.unwrap_or_else(|_| "{}".to_string());
    let local_storage: serde_json::Value = serde_json::from_str(&ls_result).unwrap_or(serde_json::json!({}));

    let state = serde_json::json!({
        "url": url,
        "cookies": cookies,
        "localStorage": local_storage,
        "savedAt": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
    });

    std::fs::write(&path, serde_json::to_string_pretty(&state).unwrap_or_default())
        .map_err(|e| format!("Cache write: {e}"))?;

    eprintln!("[SESSION] Cached {} cookies for {} → {}", cookies.len(), domain, path.display());
    Ok(())
}

// ─── JSON-RPC types ───

#[derive(Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Value,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

// ─── MCP Protocol types ───

#[derive(Serialize)]
struct McpInitResult {
    #[serde(rename = "protocolVersion")]
    protocol_version: String,
    capabilities: McpCapabilities,
    #[serde(rename = "serverInfo")]
    server_info: ServerInfo,
}

#[derive(Serialize)]
struct McpCapabilities {
    tools: ToolsCapability,
}

#[derive(Serialize)]
struct ToolsCapability {}

#[derive(Serialize)]
struct ServerInfo {
    name: String,
    version: String,
}

#[derive(Serialize)]
struct ToolDef {
    name: String,
    description: String,
    #[serde(rename = "inputSchema")]
    input_schema: Value,
}

#[derive(Serialize)]
struct ToolResult {
    content: Vec<ToolContent>,
    #[serde(rename = "isError", skip_serializing_if = "Option::is_none")]
    is_error: Option<bool>,
}

#[derive(Serialize)]
struct ToolContent {
    #[serde(rename = "type")]
    content_type: String,
    text: String,
}

// ─── Server state ───

struct McpState {
    session: Option<engine::Session>,
    ghost: Option<ghost::GhostBrowser>,
    neo_session: Option<crate::neorender::session::NeoSession>,
    wom_revision: u64,
    prev_wom: Option<wom::WomDocument>,
    auth_state: auth::AuthState,
    pending_challenge: Option<auth::AuthChallenge>,
    trace: crate::trace::TraceLog,
    pool: crate::pool::BrowserPool,
}

impl McpState {
    fn new() -> Self {
        auth::ensure_dirs().ok();
        Self {
            session: None,
            ghost: None,
            neo_session: None,
            wom_revision: 0,
            prev_wom: None,
            auth_state: auth::AuthState::Idle,
            pending_challenge: None,
            trace: crate::trace::TraceLog::new(),
            pool: crate::pool::BrowserPool::new(8),
        }
    }

    async fn ensure_session(&mut self) -> Result<&mut engine::Session, String> {
        // Check if existing session is dead (Chrome crashed, WS disconnected)
        if let Some(ref session) = self.session {
            if !session.is_alive() {
                eprintln!("[MCP] Session dead — dropping for recovery");
                let _ = self.session.take();
                self.wom_revision = 0;
                self.prev_wom = None;
            }
        }

        if self.session.is_none() {
            self.launch_session(None).await?;
        }
        Ok(self.session.as_mut().unwrap())
    }

    async fn launch_session(&mut self, force_headed: Option<bool>) -> Result<(), String> {
        let headless = match force_headed {
            Some(true) => false,  // force headed
            Some(false) => true,  // force headless
            None => std::env::var("NEOBROWSER_HEADLESS").unwrap_or_default() == "1",
        };
        // Always use stealth in headless (ChatGPT, Cloudflare detect non-stealth headless)
        let stealth = headless || std::env::var("NEOBROWSER_STEALTH").unwrap_or_default() == "1";

        // Pre-persist cookies if configured
        if let Ok(cookie_paths) = std::env::var("NEOBROWSER_COOKIES") {
            let profile_dir = engine::default_profile_dir();
            for path in cookie_paths.split(',') {
                let path = path.trim();
                if !path.is_empty() {
                    match engine::persist_cookies_to_profile(&profile_dir, path) {
                        Ok(n) => eprintln!("[MCP] Pre-persisted {n} cookies from {path}"),
                        Err(e) => eprintln!("[MCP] Cookie persist warning: {e}"),
                    }
                }
            }
        }

        let mode = if headless { "headless" } else { "headed" };
        eprintln!("[MCP] Launching Chrome ({mode})");

        let session = if stealth && headless {
            eprintln!("[MCP] Stealth mode (pipe CDP, no TCP port)");
            engine::Session::launch_stealth(None, true)
                .await
                .map_err(|e| format!("Failed to launch stealth Chrome: {e}"))?
        } else {
            match engine::Session::connect_running().await {
                Ok(s) => {
                    eprintln!("[MCP] Connected to running Chrome");
                    s
                }
                Err(_) => {
                    engine::Session::launch_ex(None, headless)
                        .await
                        .map_err(|e| format!("Failed to launch Chrome: {e}"))?
                }
            }
        };
        self.session = Some(session);
        Ok(())
    }

    /// Detect Cloudflare challenge and restart headed if needed.
    /// Returns true if it restarted and the caller should retry.
    async fn cloudflare_fallback(&mut self, url: &str) -> bool {
        let is_cloudflare = if let Some(ref session) = self.session {
            match session.eval("document.title").await {
                Ok(title) => title.contains("Un momento") || title.contains("Just a moment"),
                Err(_) => false,
            }
        } else {
            return false;
        };

        if !is_cloudflare {
            return false;
        }

        let is_headless = std::env::var("NEOBROWSER_HEADLESS").unwrap_or_default() == "1";
        if !is_headless {
            return false; // already headed, can't help
        }

        eprintln!("[MCP] Cloudflare detected in headless — restarting headed to pass challenge");

        // Close headless session
        if let Some(mut s) = self.session.take() {
            let _ = s.close().await;
        }
        self.wom_revision = 0;
        self.prev_wom = None;

        // Relaunch headed
        if let Err(e) = self.launch_session(Some(true)).await {
            eprintln!("[MCP] Headed relaunch failed: {e}");
            return false;
        }

        // Navigate again
        if let Some(ref mut session) = self.session {
            let _ = session.goto(url).await;
            // Wait for Cloudflare to pass
            for _ in 0..30 {
                tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
                if let Ok(title) = session.eval("document.title").await {
                    if !title.contains("Un momento") && !title.contains("Just a moment") {
                        eprintln!("[MCP] Cloudflare passed! Continuing headed.");
                        return true;
                    }
                }
            }
        }
        eprintln!("[MCP] Cloudflare still blocking after headed restart");
        false
    }

    fn next_revision(&mut self) -> u64 {
        self.wom_revision += 1;
        self.wom_revision
    }
}

// ─── Tool definitions ───

fn tool_definitions() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "browser_open".into(),
            description: "Open a URL and return WOM (Web Object Model) representation. Use mode='auto' to try light HTTP first and fall back to Chrome for JS-heavy pages.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "URL to open" },
                    "mode": {
                        "type": "string",
                        "enum": ["auto", "light", "neorender", "chrome"],
                        "default": "auto",
                        "description": "auto: Ghost→NeoRender→Chrome | light: HTTP only | neorender: V8 JS execution (no Chrome) | chrome: full browser"
                    },
                    "cookies_file": {
                        "type": "string",
                        "description": "Path to cookies JSON file (Playwright storageState or array)"
                    }
                },
                "required": ["url"]
            }),
        },
        ToolDef {
            name: "browser_observe".into(),
            description: "See the current page as a user would. Returns visible text, interactive elements (inputs, buttons, links), and page info. Default format='see' is fast and human-like. Use 'wom' formats only when you need stable IDs for complex automation.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "format": {
                        "type": "string",
                        "enum": ["see", "compact", "full", "content", "delta"],
                        "default": "see",
                        "description": "see: what a user sees — text + interactive elements (FAST, recommended) | compact: WOM minimal JSON | content: WOM readable text | full: complete WOM JSON | delta: WOM changes since last"
                    },
                    "include_network": {
                        "type": "boolean",
                        "default": false,
                        "description": "Include captured network requests (call browser_session start_capture first)"
                    },
                    "include_console": {
                        "type": "boolean",
                        "default": false,
                        "description": "Include captured console messages"
                    }
                }
            }),
        },
        ToolDef {
            name: "browser_act".into(),
            description: "Execute an action on the page. Target by node_id from WOM or by semantic text match. Returns delta showing what changed.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "kind": {
                        "type": "string",
                        "enum": ["click", "type", "focus", "press", "scroll", "back", "forward", "reload", "eval", "hover", "select", "fill_form", "send_message", "drag", "upload", "clipboard_read", "clipboard_write", "mouse", "highlight", "get_info", "screenshot_annotated", "analyze_forms", "analyze_api", "list_frames", "switch_frame", "auto_frame", "main_frame"],
                        "description": "Action type"
                    },
                    "target": {
                        "type": "string",
                        "description": "Node ID from WOM (e.g. 'btn_003') or text to match (e.g. 'Sign in')"
                    },
                    "text": {
                        "type": "string",
                        "description": "Text to type (for 'type' action) or JS to evaluate (for 'eval' action)"
                    },
                    "key": {
                        "type": "string",
                        "description": "Key to press (for 'press' action): Enter, Tab, Escape, etc."
                    },
                    "direction": {
                        "type": "string",
                        "enum": ["up", "down", "top", "bottom"],
                        "default": "down",
                        "description": "Scroll direction"
                    },
                    "value": {
                        "type": "string",
                        "description": "Value to select (for 'select' action)"
                    },
                    "fields": {
                        "type": "object",
                        "description": "For 'fill_form': map of field_name→value to fill multiple fields at once",
                        "additionalProperties": { "type": "string" }
                    },
                    "return_observation": {
                        "type": "string",
                        "enum": ["none", "see", "compact", "delta"],
                        "default": "see",
                        "description": "What to return after action: see (fast, what user sees), compact/delta (WOM), none"
                    },
                    "from_x": {"type": "number", "description": "Source X coordinate (for drag)"},
                    "from_y": {"type": "number", "description": "Source Y coordinate (for drag)"},
                    "to_x": {"type": "number", "description": "Destination X coordinate (for drag)"},
                    "to_y": {"type": "number", "description": "Destination Y coordinate (for drag)"},
                    "selector": {"type": "string", "description": "CSS selector (for upload, highlight, get_info)"},
                    "files": {"type": "array", "items": {"type": "string"}, "description": "File paths (for upload)"},
                    "button": {"type": "string", "enum": ["left", "right", "middle"], "default": "left", "description": "Mouse button (for mouse)"},
                    "x": {"type": "number", "description": "X coordinate (for mouse)"},
                    "y": {"type": "number", "description": "Y coordinate (for mouse)"},
                    "what": {"type": "string", "description": "What to get: text, html, value, box, styles, count, or attribute name (for get_info)"},
                    "mouse_action": {"type": "string", "enum": ["move", "down", "up", "wheel"], "description": "Mouse action type"},
                    "delta_x": {"type": "number", "description": "Wheel delta X (for mouse wheel)"},
                    "delta_y": {"type": "number", "description": "Wheel delta Y (for mouse wheel)"}
                },
                "required": ["kind"]
            }),
        },
        ToolDef {
            name: "browser_wait".into(),
            description: "Wait for a condition: page load, text to appear/disappear, time delay. Returns observation after condition met.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "seconds": {
                        "type": "number",
                        "description": "Wait N seconds"
                    },
                    "text_present": {
                        "type": "string",
                        "description": "Wait until this text appears on page"
                    },
                    "text_absent": {
                        "type": "string",
                        "description": "Wait until this text disappears"
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "default": 10000,
                        "description": "Max wait time in ms"
                    }
                }
            }),
        },
        ToolDef {
            name: "browser_tabs".into(),
            description: "Manage browser tabs: list all, switch to a tab, close a tab.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "op": {
                        "type": "string",
                        "enum": ["list", "switch", "close"],
                        "description": "Operation to perform"
                    },
                    "index": {
                        "type": "integer",
                        "description": "Tab index (for switch/close)"
                    }
                },
                "required": ["op"]
            }),
        },
        ToolDef {
            name: "browser_auth".into(),
            description: "Authentication & session management. Create profiles with credentials stored in OS keychain. Auto-login with saved sessions, handle 2FA challenges interactively.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "op": {
                        "type": "string",
                        "enum": ["profiles", "add_profile", "delete_profile", "set_credential", "login", "resume_challenge", "check_session", "save_session", "auto_session", "extract_chrome"],
                        "description": "profiles: list all | add_profile: create new | set_credential: store username/password/totp_seed in keychain | login: start auth flow | resume_challenge: provide 2FA code | check_session: verify saved session | save_session: export current cookies+localStorage | auto_session: auto-load saved session | extract_chrome: connect to real Chrome via CDP and extract authenticated session"
                    },
                    "profile_id": { "type": "string", "description": "Profile name (e.g. 'linkedin-work')" },
                    "display_name": { "type": "string" },
                    "domains": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Domains this profile covers (e.g. ['linkedin.com', 'www.linkedin.com'])"
                    },
                    "login_url": { "type": "string", "description": "URL of the login page" },
                    "credential_kind": {
                        "type": "string",
                        "enum": ["username", "password", "totp_seed"],
                        "description": "Which credential to set"
                    },
                    "credential_value": { "type": "string", "description": "The credential value (stored in OS keychain, never logged)" },
                    "challenge_id": { "type": "string", "description": "Challenge ID to resume" },
                    "challenge_response": { "type": "string", "description": "User's response to the challenge (2FA code, etc.)" },
                    "domain": { "type": "string", "description": "Domain for check_session/auto_session" },
                    "totp_enabled": { "type": "boolean", "default": false },
                    "port": { "type": "integer", "default": 9222, "description": "Chrome remote debugging port for extract_chrome (default 9222)" }
                },
                "required": ["op"]
            }),
        },
        ToolDef {
            name: "browser_session".into(),
            description: "Manage session: load cookies, capture network/console, screenshot, or reset browser.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "op": {
                        "type": "string",
                        "enum": ["load_cookies", "screenshot", "reset", "start_capture", "network", "console", "dialogs", "pdf", "device", "geo", "offline", "color_scheme"],
                        "description": "Operation: start_capture enables network+console+dialog interception"
                    },
                    "cookies_file": {
                        "type": "string",
                        "description": "Path to cookies JSON file"
                    },
                    "path": {"type": "string", "description": "File path (for pdf)"},
                    "width": {"type": "integer", "description": "Viewport width (for device)"},
                    "height": {"type": "integer", "description": "Viewport height (for device)"},
                    "scale": {"type": "number", "default": 1.0, "description": "Device scale factor"},
                    "mobile": {"type": "boolean", "default": false, "description": "Mobile mode"},
                    "user_agent": {"type": "string", "description": "Custom user agent (for device)"},
                    "lat": {"type": "number", "description": "Latitude (for geo)"},
                    "lng": {"type": "number", "description": "Longitude (for geo)"},
                    "enabled": {"type": "boolean", "description": "Enable/disable (for offline)"},
                    "scheme": {"type": "string", "enum": ["dark", "light", "no-preference"], "description": "Color scheme"}
                },
                "required": ["op"]
            }),
        },
        ToolDef {
            name: "browser_api".into(),
            description: "Make HTTP requests from inside the browser context, inheriting cookies and session. Much faster than navigating. Use for API calls, data extraction, or any endpoint that returns JSON/HTML/text.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "method": {
                        "type": "string",
                        "enum": ["GET", "POST", "PUT", "DELETE", "PATCH"],
                        "default": "GET",
                        "description": "HTTP method"
                    },
                    "url": {
                        "type": "string",
                        "description": "URL to request (absolute or relative to current page)"
                    },
                    "headers": {
                        "type": "object",
                        "description": "Optional extra headers",
                        "additionalProperties": { "type": "string" }
                    },
                    "body": {
                        "type": "string",
                        "description": "Request body (for POST/PUT/PATCH)"
                    },
                    "content_type": {
                        "type": "string",
                        "default": "application/x-www-form-urlencoded",
                        "description": "Content-Type header for the request body"
                    },
                    "max_length": {
                        "type": "integer",
                        "default": 8000,
                        "description": "Max response text length to return (truncated if longer)"
                    },
                    "extract": {
                        "type": "string",
                        "enum": ["text", "json", "html", "headers"],
                        "default": "text",
                        "description": "What to extract: text (innerText of parsed HTML), json (raw JSON), html (raw HTML), headers (response headers)"
                    }
                },
                "required": ["url"]
            }),
        },
        // ── New tools: state, network, trace, pipeline, pool ──
        ToolDef {
            name: "browser_state".into(),
            description: "Manage browser state: export/import cookies+localStorage+sessionStorage, check session health.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "op": {
                        "type": "string",
                        "enum": ["export", "import", "health"],
                        "description": "export: save all state to JSON | import: restore from JSON | health: check if session is alive"
                    },
                    "data": {
                        "type": "object",
                        "description": "State data for import (from previous export)"
                    },
                    "file": {
                        "type": "string",
                        "description": "File path to save/load state"
                    }
                },
                "required": ["op"]
            }),
        },
        ToolDef {
            name: "browser_network".into(),
            description: "Advanced network intelligence: capture requests+responses via JS monkeypatch or CDP events. CDP mode survives navigation and captures cross-origin iframe requests.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "op": {
                        "type": "string",
                        "enum": ["start", "read", "stop", "body", "har", "intercept", "clear_intercepts"],
                        "description": "start: begin capture | read: get captured data | stop: stop CDP capture | body: get response body by requestId | har: export as HAR | intercept: mock URL responses | clear_intercepts: remove all intercept rules"
                    },
                    "capture_mode": {
                        "type": "string",
                        "enum": ["js", "cdp", "both"],
                        "description": "js: monkeypatch (same-origin only, captures bodies) | cdp: CDP events (all requests incl. cross-origin iframes, survives navigation) | both: run simultaneously. Default: js"
                    },
                    "url_pattern": { "type": "string", "description": "URL pattern for intercept (e.g. '*api*')" },
                    "url_filter": { "type": "string", "description": "URL substring filter for read (CDP mode)" },
                    "response_body": { "type": "string", "description": "Custom response body for intercept" },
                    "status_code": { "type": "integer", "description": "HTTP status for intercept (default: 200)" },
                    "content_type": { "type": "string", "description": "Content-Type for intercept (default: application/json)" },
                    "request_id": { "type": "string", "description": "Request ID for body retrieval (from CDP capture)" },
                    "source": { "type": "string", "enum": ["cdp", "js", "auto"], "description": "Data source for HAR export (default: auto)" },
                    "file": { "type": "string", "description": "File path to save HAR export" }
                },
                "required": ["op"]
            }),
        },
        ToolDef {
            name: "browser_trace".into(),
            description: "Action tracing and observability: record all actions with timing and outcomes, get stats.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "op": {
                        "type": "string",
                        "enum": ["start", "stop", "read", "stats", "clear"],
                        "description": "start: enable tracing | stop: disable | read: get traces | stats: success rates | clear: reset"
                    },
                    "last_n": {
                        "type": "integer",
                        "description": "For read: only return last N traces"
                    }
                },
                "required": ["op"]
            }),
        },
        ToolDef {
            name: "browser_pipeline".into(),
            description: "Run deterministic automation pipelines. Actions: goto, click, type, press, wait, eval, extract, screenshot, send_message (contenteditable+click send), wait_stable (poll until text stops changing), save (write to file). Steps have retry, timeout, assert, store_as, on_fail.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pipeline": {
                        "type": "object",
                        "description": "Pipeline definition: {name, steps: [{action, target, value, timeout_ms, max_retries, assert_text, store_as, on_fail}]}"
                    },
                    "pipeline_json": {
                        "type": "string",
                        "description": "Pipeline as JSON string (alternative to pipeline object)"
                    }
                }
            }),
        },
        ToolDef {
            name: "browser_pool".into(),
            description: "Manage isolated browser contexts for parallel automation. Each context has its own profile and state.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "op": {
                        "type": "string",
                        "enum": ["create", "list", "destroy", "destroy_all"],
                        "description": "create: new isolated context | list: show all | destroy: remove one | destroy_all: clean up"
                    },
                    "id": {
                        "type": "string",
                        "description": "Context ID (for create/destroy)"
                    }
                },
                "required": ["op"]
            }),
        },
        ToolDef {
            name: "browser_learn".into(),
            description: "Workflow mapper: explore a web app step by step, document every interaction, and save a reusable playbook. Use 'start' to begin recording, 'observe' for rich page analysis (forms, Vue state, dropdowns), 'act' to perform actions while recording, 'save' to export playbook, 'replay' to re-execute a saved playbook.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "op": {
                        "type": "string",
                        "enum": ["start", "observe", "act", "save", "stop", "replay"],
                        "description": "start: begin workflow recording | observe: rich page analysis (forms, Vue/React state, errors, dropdowns) | act: perform action + record | save: export playbook to file | stop: end recording | replay: execute saved playbook"
                    },
                    "name": {
                        "type": "string",
                        "description": "For start: workflow name (e.g. 'sesame-add-employee')"
                    },
                    "action": {
                        "type": "string",
                        "description": "For act: action type (click, type, select, navigate, press, scroll, hover, wait)"
                    },
                    "target": {
                        "type": "string",
                        "description": "For act: target text/label/selector"
                    },
                    "value": {
                        "type": "string",
                        "description": "For act: typed text, selected value, key name, wait seconds"
                    },
                    "notes": {
                        "type": "string",
                        "description": "For act: human-readable notes about what this step does"
                    },
                    "path": {
                        "type": "string",
                        "description": "For save/replay: file path for the playbook JSON"
                    }
                },
                "required": ["op"]
            }),
        },
    ]
}

// ─── Target resolution ───

/// Resolve a WOM node_id (e.g. "btn_003") to the actual text/label the browser can find.
/// If the target doesn't look like a WOM ID, pass it through as-is.
fn resolve_target(target: &str, prev_wom: &Option<wom::WomDocument>) -> String {
    if target.is_empty() {
        return String::new();
    }

    // Check if this looks like a WOM node ID: prefix_NNN pattern
    let is_wom_id = target.contains('_')
        && target.split('_').last().map(|s| s.chars().all(|c| c.is_ascii_digit())).unwrap_or(false);

    if !is_wom_id {
        return target.to_string();
    }

    // Try to resolve from previous WOM
    if let Some(ref doc) = prev_wom {
        for node in &doc.nodes {
            if node.id == target {
                // For links/buttons: use the name (visible text)
                if !node.name.is_empty() {
                    return node.name.clone();
                }
                // For fields: use locator aliases or name
                if let Some(ref loc) = node.locator {
                    if !loc.aliases.is_empty() {
                        return loc.aliases[0].clone();
                    }
                }
            }
        }
    }

    // Fallback: pass through as-is
    target.to_string()
}

// ─── Tool execution ───

async fn handle_tool(state: &mut McpState, name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "browser_open" => handle_open(state, args).await,
        "browser_observe" => handle_observe(state, args).await,
        "browser_act" => handle_act(state, args).await,
        "browser_wait" => handle_wait(state, args).await,
        "browser_tabs" => handle_tabs(state, args).await,
        "browser_session" => handle_session(state, args).await,
        "browser_auth" => handle_auth(state, args).await,
        "browser_api" => handle_api(state, args).await,
        "browser_state" => handle_state(state, args).await,
        "browser_network" => handle_network(state, args).await,
        "browser_trace" => handle_trace(state, args).await,
        "browser_pipeline" => handle_pipeline(state, args).await,
        "browser_pool" => handle_pool(state, args).await,
        "browser_learn" => handle_learn(state, args).await,
        _ => Err(format!("Unknown tool: {name}")),
    }
}

async fn handle_open(state: &mut McpState, args: &Value) -> Result<Value, String> {
    let url = args["url"].as_str().ok_or("Missing 'url'")?;
    let mode = args["mode"].as_str().unwrap_or("auto");

    // Check allowed domains
    if let Ok(domains) = std::env::var("NEOBROWSER_ALLOWED_DOMAINS") {
        let allowed: Vec<&str> = domains.split(',').map(|s| s.trim()).collect();
        if let Ok(parsed) = url::Url::parse(url) {
            if let Some(host) = parsed.host_str() {
                let allowed_match = allowed.iter().any(|d| {
                    if d.starts_with('*') {
                        host.ends_with(&d[1..])
                    } else {
                        host == *d
                    }
                });
                if !allowed_match {
                    return Err(format!("Domain '{host}' not in allowed list: {domains}"));
                }
            }
        }
    }

    // ─── NeoRender mode: force V8 JS execution ───
    if mode == "neorender" {
        let domain = domain_from_url(url);

        // ─── NeoSession path: persistent session (preferred) ───
        {
            // Create NeoSession if not exists
            if state.neo_session.is_none() {
                let cookies_file = args["cookies_file"].as_str()
                    .map(|s| s.to_string())
                    .or_else(|| load_session_cache(&domain));
                match crate::neorender::session::NeoSession::new(cookies_file.as_deref()) {
                    Ok(ns) => {
                        eprintln!("[MCP] Created new NeoSession");
                        state.neo_session = Some(ns);
                    }
                    Err(e) => {
                        eprintln!("[MCP] NeoSession creation failed: {e} — falling back to render_page");
                    }
                }
            } else {
                // Existing session — load additional cookies if provided
                if let Some(neo) = state.neo_session.as_mut() {
                    if let Some(cookies_file) = args["cookies_file"].as_str() {
                        neo.cookies_mut().load_file(cookies_file).ok();
                    }
                }
            }

            if let Some(neo) = state.neo_session.as_mut() {
                match neo.goto(url).await {
                    Ok(result) => {
                        // Check for WAF — if detected, drop NeoSession and fall through
                        let is_waf = result.errors.iter().any(|e| e.starts_with("WAF challenge:"));
                        if !is_waf {
                            // Success — build response from WOM (extracted in V8, no html5ever re-parse)
                            let title = result.title.clone();
                            let text = result.text.clone();

                            let (links_count, forms_count, inputs_count, buttons_count) = if let Some(ref wom) = result.wom {
                                (
                                    wom["links"].as_array().map(|a| a.len()).unwrap_or(0),
                                    wom["forms"].as_array().map(|a| a.len()).unwrap_or(0),
                                    wom["inputs"].as_array().map(|a| a.len()).unwrap_or(0),
                                    wom["buttons"].as_array().map(|a| a.len()).unwrap_or(0),
                                )
                            } else {
                                (0, 0, 0, 0)
                            };

                            let mut see = format!(
                                "Page: {}\nURL: {}\nStatus: {}\nType: {}\nEngine: neosession (V8 persistent)\nRender: {}ms, {} scripts\n",
                                title, result.url, result.status, result.page_type, result.render_time_ms, result.scripts_count,
                            );
                            if !text.is_empty() {
                                let t = if text.len() > 8000 { &text[..8000] } else { &text };
                                see.push_str(&format!("\n--- Text ---\n{}\n", t.trim()));
                            }
                            if !result.errors.is_empty() {
                                see.push_str(&format!("\n--- Errors ({}) ---\n", result.errors.len()));
                                for err in &result.errors {
                                    see.push_str(&format!("  {}\n", err));
                                }
                            }
                            if links_count > 0 || forms_count > 0 || inputs_count > 0 || buttons_count > 0 {
                                see.push_str(&format!("\n--- Elements ---\nLinks: {}, Forms: {}, Inputs: {}, Buttons: {}\n",
                                    links_count, forms_count, inputs_count, buttons_count));
                            }

                            // Include headings summary if available
                            if let Some(ref wom) = result.wom {
                                if let Some(headings) = wom["headings"].as_array() {
                                    if !headings.is_empty() {
                                        see.push_str("\n--- Headings ---\n");
                                        for h in headings.iter().take(20) {
                                            let level = h["level"].as_u64().unwrap_or(0);
                                            let htext = h["text"].as_str().unwrap_or("");
                                            see.push_str(&format!("  H{}: {}\n", level, htext));
                                        }
                                    }
                                }
                            }

                            // Include actions summary
                            if !result.actions.is_empty() {
                                see.push_str(&format!("\n--- Actions ({}) ---\n", result.actions.len()));
                                for action in result.actions.iter().take(25) {
                                    see.push_str(&format!("  [{}] {} → {}\n", action.action_type, action.text, action.target));
                                }
                            }

                            // Build actions JSON array
                            let actions_json: Vec<serde_json::Value> = result.actions.iter().take(50).map(|a| {
                                serde_json::json!({
                                    "type": a.action_type,
                                    "text": a.text,
                                    "target": a.target,
                                })
                            }).collect();

                            return Ok(serde_json::json!({
                                "ok": true,
                                "engine": "neosession",
                                "persistent": true,
                                "url": result.url,
                                "status": result.status,
                                "title": title,
                                "page_type": result.page_type,
                                "links": links_count,
                                "forms": forms_count,
                                "inputs": inputs_count,
                                "buttons": buttons_count,
                                "actions": actions_json,
                                "render_ms": result.render_time_ms,
                                "scripts": result.scripts_count,
                                "html_bytes": result.html_len,
                                "errors": result.errors,
                                "page": see,
                            }));
                        }
                        // WAF detected — drop session and fall through to legacy path
                        eprintln!("[MCP] NeoSession hit WAF — falling back to render_page + Chrome recovery");
                        state.neo_session = None;
                    }
                    Err(e) => {
                        eprintln!("[MCP] NeoSession goto failed: {e} — falling back to render_page");
                        state.neo_session = None;
                    }
                }
            }
        }

        // ─── Legacy fallback: render_page (creates/destroys per call) ───
        let ghost = state.ghost.get_or_insert_with(|| ghost::GhostBrowser::new());
        // Load cookies: explicit file > session cache > env var
        if let Some(cookies_file) = args["cookies_file"].as_str() {
            ghost.load_cookies(cookies_file).ok();
        } else if let Some(cached) = load_session_cache(&domain) {
            ghost.load_cookies(&cached).ok();
        }
        if let Ok(cookie_paths) = std::env::var("NEOBROWSER_COOKIES") {
            for path in cookie_paths.split(',') {
                let path = path.trim();
                if !path.is_empty() { ghost.load_cookies(path).ok(); }
            }
        }
        // Load localStorage from state file if present
        let cookies_source = args["cookies_file"].as_str()
            .map(|s| s.to_string())
            .or_else(|| load_session_cache(&domain));
        let local_storage: Option<std::collections::HashMap<String, String>> = cookies_source.as_deref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|data| serde_json::from_str::<serde_json::Value>(&data).ok())
            .and_then(|v| v["localStorage"].as_object().cloned())
            .map(|obj| obj.into_iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_string())))
                .collect());

        let ghost = state.ghost.as_mut().unwrap();
        let result = crate::neorender::render_page(ghost, url, local_storage.as_ref()).await?;

        // WAF detected → try to resolve via Chrome with human assistance
        let is_waf = result.errors.iter().any(|e| e.starts_with("WAF challenge:"));
        if is_waf {
            eprintln!("[NEORENDER] WAF blocked — escalating to Chrome for human resolution");

            // Launch fresh Chrome (headed for human interaction)
            // Kill old session if exists — WAF needs a clean browser
            if state.session.is_some() {
                eprintln!("[SESSION] Closing old Chrome session for WAF resolution");
                state.session = None;
            }
            let session = match engine::Session::launch_stealth(None, false).await {
                Ok(s) => { state.session = Some(s); state.session.as_mut().unwrap() },
                Err(e) => {
                    return Ok(serde_json::json!({
                        "ok": false, "engine": "neorender",
                        "blocked": result.errors.first().unwrap_or(&String::new()),
                        "error": format!("Chrome launch failed: {e}"),
                        "url": url,
                    }));
                }
            };

            // Navigate to the blocked URL
            eprintln!("[SESSION] Navigating Chrome to: {}", url);
            if let Err(e) = session.goto(url).await {
                eprintln!("[SESSION] Chrome goto failed: {e}");
            }

            // Wait for page to stabilize (human resolves captcha/challenge)
            eprintln!("[SESSION] Waiting for human to resolve WAF challenge at: {}", url);
            eprintln!("[SESSION] The page is open in Chrome — resolve the challenge and the browser will capture the session.");

            // Wait up to 60s for the page to change (challenge resolved)
            for _wait in 0..12 {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                // Check if we're past the challenge (URL changed or page has content)
                let current_url = session.eval_string("location.href").await.unwrap_or_default();
                let body_len = session.eval_string("document.body?.innerText?.length || 0").await
                    .unwrap_or_else(|_| "0".to_string())
                    .parse::<usize>().unwrap_or(0);
                eprintln!("[SESSION] Check: url={} body={}B", &current_url[..current_url.len().min(60)], body_len);

                if body_len > 200 && !current_url.contains("challenge") {
                    eprintln!("[SESSION] Challenge appears resolved! Capturing session...");
                    // Save session to cache
                    save_session_to_cache(session, url).await.ok();

                    // Retry neorender with the fresh cookies
                    let ghost = state.ghost.get_or_insert_with(|| ghost::GhostBrowser::new());
                    if let Some(cached) = load_session_cache(&domain) {
                        ghost.load_cookies(&cached).ok();
                    }
                    let ls2: Option<std::collections::HashMap<String, String>> = load_session_cache(&domain)
                        .and_then(|p| std::fs::read_to_string(p).ok())
                        .and_then(|d| serde_json::from_str::<serde_json::Value>(&d).ok())
                        .and_then(|v| v["localStorage"].as_object().cloned())
                        .map(|obj| obj.into_iter()
                            .filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_string())))
                            .collect());
                    let ghost = state.ghost.as_mut().unwrap();
                    let retry = crate::neorender::render_page(ghost, url, ls2.as_ref()).await;
                    if let Ok(retry_result) = retry {
                        if !retry_result.errors.iter().any(|e| e.starts_with("WAF challenge:")) {
                            eprintln!("[SESSION] Neorender retry succeeded after human resolution!");
                            // Use retry_result instead — jump to the rendering path below
                            let dom = html5ever::parse_document(
                                markup5ever_rcdom::RcDom::default(), Default::default()
                            ).from_utf8().read_from(&mut retry_result.html.as_bytes())
                                .map_err(|e| format!("Re-parse error: {e}"))?;

                            let title = extract_title_from_dom(&dom);
                            let text = extract_text_from_dom(&dom);

                            return Ok(serde_json::json!({
                                "ok": true,
                                "engine": "neorender",
                                "human_resolved": true,
                                "url": retry_result.url,
                                "status": retry_result.status,
                                "title": title,
                                "html_bytes": retry_result.html.len(),
                                "page": format!("Page: {}\nURL: {}\nEngine: neorender (human-assisted)\n\n--- Text ---\n{}\n",
                                    title, retry_result.url, &text[..text.len().min(8000)]),
                            }));
                        }
                    }
                    break;
                }
            }

            // If we get here, human didn't resolve or retry failed — return WAF error with instructions
            return Ok(serde_json::json!({
                "ok": false,
                "engine": "neorender",
                "blocked": result.errors.first().unwrap_or(&String::new()),
                "action": "human_resolve",
                "instructions": format!("WAF challenge at {}. Chrome is open — resolve the challenge manually, then call browser_open again.", url),
                "url": url,
            }));
        }

        let dom = html5ever::parse_document(
            markup5ever_rcdom::RcDom::default(), Default::default()
        ).from_utf8().read_from(&mut result.html.as_bytes())
            .map_err(|e| format!("Re-parse error: {e}"))?;

        let title = extract_title_from_dom(&dom);
        let text = extract_text_from_dom(&dom);
        let links_count = count_elements(&dom, "a");
        let forms_count = count_elements(&dom, "form");
        let inputs_count = count_elements(&dom, "input");
        let buttons_count = count_elements(&dom, "button");

        let mut see = format!(
            "Page: {}\nURL: {}\nStatus: {}\nEngine: neorender (V8)\nRender: {}ms, {} scripts\n",
            title, result.url, result.status, result.render_time_ms, result.scripts_count,
        );
        if !text.is_empty() {
            let t = if text.len() > 8000 { &text[..8000] } else { &text };
            see.push_str(&format!("\n--- Text ---\n{}\n", t.trim()));
        }

        if !result.errors.is_empty() {
            see.push_str(&format!("\n--- Errors ({}) ---\n", result.errors.len()));
            for err in &result.errors {
                see.push_str(&format!("  {}\n", err));
            }
        }

        // Elements summary
        if links_count > 0 || forms_count > 0 || inputs_count > 0 || buttons_count > 0 {
            see.push_str(&format!("\n--- Elements ---\nLinks: {}, Forms: {}, Inputs: {}, Buttons: {}\n",
                links_count, forms_count, inputs_count, buttons_count));
        }

        // Debug: show body content (skip head which is just meta tags)
        let body_start = result.html.find("<body").unwrap_or(0);
        let body_preview = &result.html[body_start..];
        let body_preview = if body_preview.len() > 2000 { &body_preview[..2000] } else { body_preview };
        see.push_str(&format!("\n--- Body Preview ---\n{}\n", body_preview));

        return Ok(serde_json::json!({
            "ok": true,
            "engine": "neorender",
            "url": result.url,
            "status": result.status,
            "title": title,
            "links": links_count,
            "forms": forms_count,
            "inputs": inputs_count,
            "buttons": buttons_count,
            "render_ms": result.render_time_ms,
            "scripts": result.scripts_count,
            "html_bytes": result.html.len(),
            "errors": result.errors,
            "page": see,
        }));
    }

    // ─── Ghost mode: pure HTTP, no Chrome ───
    if mode == "light" || mode == "auto" {
        let ghost = state.ghost.get_or_insert_with(|| ghost::GhostBrowser::new());

        // Load cookies if provided
        if let Some(cookies_file) = args["cookies_file"].as_str() {
            ghost.load_cookies(cookies_file).ok();
        }
        // Also load from env
        if let Ok(cookie_paths) = std::env::var("NEOBROWSER_COOKIES") {
            for path in cookie_paths.split(',') {
                let path = path.trim();
                if !path.is_empty() {
                    ghost.load_cookies(path).ok();
                }
            }
        }

        match ghost.goto(url).await {
            Ok(page) => {
                // Check if we got a real page or a Cloudflare challenge
                let is_challenge = page.title.contains("moment")
                    || page.title.contains("momento")
                    || page.title.contains("Attention Required")
                    || (page.status == 403 && page.html_len < 10000);

                if is_challenge && mode == "auto" {
                    eprintln!("[GHOST] Challenge detected, falling back to Chrome");
                    // Fall through to Chrome mode below
                } else if page.is_spa && mode == "auto" {
                    // SPA detected → NeoRender (V8 JS execution)
                    eprintln!("[GHOST] SPA detected — launching NeoRender (V8)...");
                    let ghost = state.ghost.as_mut().unwrap();
                    match crate::neorender::render_page(ghost, url, None).await {
                        Ok(result) if result.errors.iter().any(|e| e.starts_with("WAF challenge:")) => {
                            eprintln!("[NEORENDER] WAF detected, falling back to Chrome");
                            // Fall through to Chrome
                        }
                        Ok(result) => {
                            // Re-parse rendered HTML → extract page info
                            let dom = html5ever::parse_document(
                                markup5ever_rcdom::RcDom::default(), Default::default()
                            ).from_utf8().read_from(&mut result.html.as_bytes())
                                .map_err(|e| format!("Re-parse error: {e}"))?;

                            let title = extract_title_from_dom(&dom);
                            let text = extract_text_from_dom(&dom);
                            let links_count = count_elements(&dom, "a");
                            let forms_count = count_elements(&dom, "form");
                            let inputs_count = count_elements(&dom, "input");
                            let buttons_count = count_elements(&dom, "button");

                            let mut see = format!(
                                "Page: {}\nURL: {}\nStatus: {}\nEngine: neorender (V8)\nRender: {}ms, {} scripts\n",
                                title, result.url, result.status,
                                result.render_time_ms, result.scripts_count,
                            );
                            if !text.is_empty() {
                                let t = if text.len() > 8000 { &text[..8000] } else { &text };
                                see.push_str(&format!("\n--- Text ---\n{}\n", t.trim()));
                            }
                            see.push_str(&format!(
                                "\n--- Elements ---\nLinks: {}, Forms: {}, Inputs: {}, Buttons: {}\n",
                                links_count, forms_count, inputs_count, buttons_count,
                            ));

                            return Ok(serde_json::json!({
                                "ok": true,
                                "engine": "neorender",
                                "url": result.url,
                                "status": result.status,
                                "title": title,
                                "is_spa": true,
                                "links": links_count,
                                "forms": forms_count,
                                "inputs": inputs_count,
                                "buttons": buttons_count,
                                "render_ms": result.render_time_ms,
                                "scripts": result.scripts_count,
                                "html_bytes": result.html.len(),
                                "page": see,
                            }));
                        }
                        Err(e) => {
                            eprintln!("[NEORENDER] Failed ({e}), falling back to Chrome");
                            // Fall through to Chrome
                        }
                    }
                } else {
                    let see = page.to_see(8000);
                    return Ok(serde_json::json!({
                        "ok": true,
                        "engine": "ghost",
                        "url": page.url,
                        "status": page.status,
                        "title": page.title,
                        "is_spa": page.is_spa,
                        "links": page.links.len(),
                        "forms": page.forms.len(),
                        "inputs": page.inputs.len(),
                        "buttons": page.buttons.len(),
                        "apis": page.apis.len(),
                        "html_bytes": page.html_len,
                        "page": see,
                    }));
                }
            }
            Err(e) => {
                if mode == "light" {
                    return Err(format!("Ghost fetch failed: {e}"));
                }
                eprintln!("[GHOST] Failed ({e}), falling back to Chrome");
            }
        }
    }

    // ─── Chrome mode: full browser ───
    let session = state.ensure_session().await?;

    // Load cookies if provided (browser-level, works on about:blank)
    if let Some(cookies_file) = args["cookies_file"].as_str() {
        session.load_cookies(cookies_file).await.map_err(|e| format!("{e}"))?;
    }

    session.goto(url).await.map_err(|e| format!("{e}"))?;
    session.apply_stealth().await.ok();
    session.dismiss_cookie_banners().await.ok();

    // Get WOM
    let rev = state.next_revision();
    let session = state.session.as_mut().unwrap();
    let doc = session.see_wom(rev).await.map_err(|e| format!("{e}"))?;
    let content = wom::format_content(&doc);
    let compact = wom::compact(&doc);

    let result = serde_json::json!({
        "ok": true,
        "engine": "chrome",
        "url": doc.page.url,
        "page_class": doc.page.page_class,
        "revision": doc.session.revision,
        "nodes": doc.nodes.len(),
        "actions": doc.actions.len(),
        "content": content,
        "compact": compact,
    });

    state.prev_wom = Some(doc);
    Ok(result)
}

async fn handle_observe(state: &mut McpState, args: &Value) -> Result<Value, String> {
    let format = args["format"].as_str().unwrap_or("see");
    let include_net = args["include_network"].as_bool().unwrap_or(false);
    let include_con = args["include_console"].as_bool().unwrap_or(false);

    let session = state.ensure_session().await?;
    let session = state.session.as_mut().unwrap();

    // Fast path: "see" mode — JS extraction, no HTML parsing, no WOM
    if format == "see" {
        let page = session.see_page().await.map_err(|e| format!("{e}"))?;

        let mut result = serde_json::json!({ "page": page });

        if include_net {
            result["network"] = serde_json::json!(session.read_network().await.unwrap_or_default());
        }
        if include_con {
            result["console"] = serde_json::json!(session.read_console().await.unwrap_or_default());
        }

        return Ok(result);
    }

    // WOM path: full HTML parsing + structured output
    let rev = state.wom_revision + 1;
    state.wom_revision = rev;

    let doc = session.see_wom(rev).await.map_err(|e| format!("{e}"))?;

    let network = if include_net {
        Some(session.read_network().await.unwrap_or_default())
    } else { None };
    let console = if include_con {
        Some(session.read_console().await.unwrap_or_default())
    } else { None };

    let mut result = match format {
        "content" => {
            let text = wom::format_content(&doc);
            serde_json::json!({
                "revision": doc.session.revision,
                "page_class": doc.page.page_class,
                "content": text,
            })
        }
        "full" => {
            serde_json::to_value(&doc).map_err(|e| format!("{e}"))?
        }
        "delta" => {
            if let Some(ref prev) = state.prev_wom {
                let d = delta::diff(prev, &doc);
                serde_json::json!({
                    "revision": doc.session.revision,
                    "page_class": doc.page.page_class,
                    "delta": d,
                    "compact": wom::compact(&doc),
                })
            } else {
                serde_json::json!({
                    "revision": doc.session.revision,
                    "compact": wom::compact(&doc),
                })
            }
        }
        _ => {
            // compact
            let c = wom::compact(&doc);
            serde_json::to_value(&c).map_err(|e| format!("{e}"))?
        }
    };

    if let Some(net) = network {
        result["network"] = serde_json::json!(net);
    }
    if let Some(con) = console {
        result["console"] = serde_json::json!(con);
    }

    state.prev_wom = Some(doc);
    Ok(result)
}

async fn handle_act(state: &mut McpState, args: &Value) -> Result<Value, String> {
    let act_t0 = std::time::Instant::now();
    let kind = args["kind"].as_str().ok_or("Missing 'kind'")?;
    let raw_target = args["target"].as_str().unwrap_or("");
    let return_obs = args["return_observation"].as_str().unwrap_or("see");

    // Check if target is a WOM ID (e.g. btn_042, lnk_015, fld_003)
    let is_wom_id = raw_target.contains('_')
        && raw_target.split('_').last()
            .map(|s| s.chars().all(|c| c.is_ascii_digit()))
            .unwrap_or(false)
        && matches!(raw_target.split('_').next(), Some("btn" | "lnk" | "fld" | "h" | "sel" | "form" | "img" | "p"));

    // Fallback: resolve WOM ID to text only if we can't use direct DOM targeting
    let target = if is_wom_id {
        raw_target.to_string() // Keep the WOM ID as-is
    } else {
        resolve_target(raw_target, &state.prev_wom)
    };

    // Pre-resolve fill_form fields before borrowing session
    let fill_fields: Option<Vec<(String, String)>> = if kind == "fill_form" {
        let fields_obj = args["fields"].as_object().ok_or("Missing 'fields' object for fill_form")?;
        Some(fields_obj.iter()
            .map(|(k, v)| {
                let field_name = resolve_target(k, &state.prev_wom);
                (field_name, v.as_str().unwrap_or("").to_string())
            })
            .collect())
    } else {
        None
    };

    // ─── NeoSession shortcut: eval/fetch without Chrome ───
    if let Some(neo) = state.neo_session.as_mut() {
        match kind {
            "eval" => {
                let js = args["text"].as_str().ok_or("Missing 'text' for eval")?;
                // Use async eval if JS contains async patterns (fetch, await, Promise, .then)
                let is_async = js.contains("await") || js.contains("fetch(") || js.contains(".then(") || js.contains("async");
                let result = if is_async {
                    neo.eval_async(js, 120_000).await.map_err(|e| format!("{e}"))?
                } else {
                    neo.eval(js).map_err(|e| format!("{e}"))?
                };
                let elapsed = act_t0.elapsed().as_millis() as u64;
                return Ok(serde_json::json!({
                    "ok": true,
                    "engine": "neosession",
                    "outcome": "succeeded",
                    "effect": format!("eval_result: {result}"),
                    "elapsed_ms": elapsed,
                }));
            }
            "click" => {
                use crate::neorender::interact::ClickResult;
                let result = neo.click(&target).await?;
                let elapsed = act_t0.elapsed().as_millis() as u64;
                let (outcome, effect) = match &result {
                    ClickResult::Clicked { tag, text } => {
                        ("succeeded", format!("clicked: {} '{}'", tag, text))
                    }
                    ClickResult::Navigated { url } => {
                        ("navigated", format!("click_navigated: {}", url))
                    }
                    ClickResult::Submitted { url, method } => {
                        ("submitted", format!("click_submitted: {} {}", method, url))
                    }
                };
                return Ok(serde_json::json!({
                    "ok": true,
                    "engine": "neosession",
                    "outcome": outcome,
                    "effect": effect,
                    "elapsed_ms": elapsed,
                }));
            }
            "type" => {
                let text = args["text"].as_str().ok_or("Missing 'text' for type action")?;
                neo.type_text(&target, text)?;
                let elapsed = act_t0.elapsed().as_millis() as u64;
                return Ok(serde_json::json!({
                    "ok": true,
                    "engine": "neosession",
                    "outcome": "succeeded",
                    "effect": format!("typed: {} chars into '{}'", text.len(), target),
                    "elapsed_ms": elapsed,
                }));
            }
            "submit" => {
                use crate::neorender::interact::SubmitResult;
                let form_target = if raw_target.is_empty() { None } else { Some(raw_target) };
                let result = neo.submit(form_target).await?;
                let elapsed = act_t0.elapsed().as_millis() as u64;
                match result {
                    SubmitResult::Submitted { ref url, ref method } => {
                        // Submit navigated — return the new page (like goto)
                        let rendered_html = neo.export_html().unwrap_or_default();
                        let dom = html5ever::parse_document(
                            markup5ever_rcdom::RcDom::default(), Default::default()
                        ).from_utf8().read_from(&mut rendered_html.as_bytes())
                            .map_err(|e| format!("Re-parse: {e}"))?;
                        let title = extract_title_from_dom(&dom);
                        let text = extract_text_from_dom(&dom);
                        let links_count = count_elements(&dom, "a");
                        let buttons_count = count_elements(&dom, "button");
                        let mut see = format!("Page: {}\nURL: {}\nMethod: {}\nEngine: neosession\n", title, url, method);
                        if !text.is_empty() {
                            let t = if text.len() > 8000 { &text[..8000] } else { &text };
                            see.push_str(&format!("\n--- Text ---\n{}\n", t.trim()));
                        }
                        return Ok(serde_json::json!({
                            "ok": true,
                            "engine": "neosession",
                            "outcome": "navigated",
                            "url": url,
                            "title": title,
                            "links": links_count,
                            "buttons": buttons_count,
                            "html_bytes": rendered_html.len(),
                            "page": see,
                            "elapsed_ms": elapsed,
                        }));
                    }
                    SubmitResult::Failed { error } => {
                        return Ok(serde_json::json!({
                            "ok": false,
                            "engine": "neosession",
                            "outcome": "failed",
                            "effect": format!("submit_failed: {}", error),
                            "elapsed_ms": elapsed,
                        }));
                    }
                }
            }
            "select" => {
                let value = args["value"].as_str().ok_or("Missing 'value' for select action")?;
                neo.select(&target, value)?;
                let elapsed = act_t0.elapsed().as_millis() as u64;
                return Ok(serde_json::json!({
                    "ok": true,
                    "engine": "neosession",
                    "outcome": "succeeded",
                    "effect": format!("selected: '{}' in '{}'", value, target),
                    "elapsed_ms": elapsed,
                }));
            }
            "neo_fetch" => {
                let fetch_url = args["url"].as_str().ok_or("Missing 'url' for neo_fetch")?;
                let method = args["method"].as_str().unwrap_or("GET");
                let body = args["body"].as_str();
                let hdrs = args["headers"].as_str();
                let result = neo.fetch(fetch_url, method, body, hdrs).await
                    .map_err(|e| format!("{e}"))?;
                let elapsed = act_t0.elapsed().as_millis() as u64;
                return Ok(serde_json::json!({
                    "ok": true,
                    "engine": "neosession",
                    "outcome": "succeeded",
                    "effect": format!("fetch_result: {result}"),
                    "elapsed_ms": elapsed,
                }));
            }
            "neo_export_html" => {
                let html = neo.export_html().map_err(|e| format!("{e}"))?;
                let elapsed = act_t0.elapsed().as_millis() as u64;
                return Ok(serde_json::json!({
                    "ok": true,
                    "engine": "neosession",
                    "outcome": "succeeded",
                    "effect": format!("html_bytes: {}", html.len()),
                    "html": html,
                    "elapsed_ms": elapsed,
                }));
            }
            "neo_export_text" => {
                let text = neo.export_text().map_err(|e| format!("{e}"))?;
                let elapsed = act_t0.elapsed().as_millis() as u64;
                return Ok(serde_json::json!({
                    "ok": true,
                    "engine": "neosession",
                    "outcome": "succeeded",
                    "effect": text,
                    "elapsed_ms": elapsed,
                }));
            }
            _ => {} // Fall through to Chrome session for other actions
        }
    }

    let session = state.ensure_session().await?;
    let session = state.session.as_mut().unwrap();

    let (outcome, effect) = match kind {
        "click" => {
            // WOM ID → direct DOM click (no text matching)
            // Text → smart text-based click (fallback)
            let found = if is_wom_id {
                session.click_by_wom_id(&target).await.map_err(|e| format!("{e}"))?
            } else {
                session.click(&target).await.map_err(|e| format!("{e}"))?
            };
            if found {
                ("succeeded", format!("clicked: {target}"))
            } else {
                ("not_found", format!("target_not_found: {target} (original: {raw_target})"))
            }
        }
        "type" => {
            let text = args["text"].as_str().ok_or("Missing 'text' for type action")?;
            // If target is a WOM ID, focus by ID first
            if is_wom_id {
                session.focus_by_wom_id(&target).await.map_err(|e| format!("{e}"))?;
            }
            session.type_text(text).await.map_err(|e| format!("{e}"))?;
            ("succeeded", format!("typed: {} chars", text.len()))
        }
        "focus" => {
            let found = if is_wom_id {
                session.focus_by_wom_id(&target).await.map_err(|e| format!("{e}"))?
            } else {
                session.focus(&target).await.map_err(|e| format!("{e}"))?
            };
            if found {
                ("succeeded", format!("focused: {target}"))
            } else {
                ("not_found", format!("focus_not_found: {target}"))
            }
        }
        "press" => {
            let key = args["key"].as_str().unwrap_or("Enter");
            session.press(key).await.map_err(|e| format!("{e}"))?;
            ("succeeded", format!("pressed: {key}"))
        }
        "scroll" => {
            let dir = args["direction"].as_str().unwrap_or("down");
            session.scroll(dir).await.map_err(|e| format!("{e}"))?;
            ("succeeded", format!("scrolled: {dir}"))
        }
        "back" => {
            session.back().await.map_err(|e| format!("{e}"))?;
            ("succeeded", "navigated_back".into())
        }
        "forward" => {
            session.forward().await.map_err(|e| format!("{e}"))?;
            ("succeeded", "navigated_forward".into())
        }
        "reload" => {
            session.reload().await.map_err(|e| format!("{e}"))?;
            ("succeeded", "reloaded".into())
        }
        "eval" => {
            let js = args["text"].as_str().ok_or("Missing 'text' for eval")?;
            let result = session.eval(js).await.map_err(|e| format!("{e}"))?;
            ("succeeded", format!("eval_result: {result}"))
        }
        "hover" => {
            let found = session.hover(&target).await.map_err(|e| format!("{e}"))?;
            if found {
                ("succeeded", format!("hovered: {target}"))
            } else {
                ("not_found", format!("hover_not_found: {target}"))
            }
        }
        "select" => {
            let value = args["value"].as_str().ok_or("Missing 'value' for select action")?;
            let found = session.select_option(&target, value).await.map_err(|e| format!("{e}"))?;
            if found {
                ("succeeded", format!("selected: {value} in {target}"))
            } else {
                ("not_found", format!("select_failed: {target}/{value}"))
            }
        }
        "fill_form" => {
            let fields = fill_fields.unwrap();
            let results = session.fill_form(&fields).await.map_err(|e| format!("{e}"))?;
            let all_ok = results.iter().all(|r| r.starts_with("filled:"));
            let outcome_str = if all_ok { "succeeded" } else { "partial" };
            (outcome_str, format!("fill_form: {}", results.join(", ")))
        }
        // ── New actions (v3) ──
        "click_css" => {
            let selector = args["selector"].as_str().unwrap_or(&target);
            let found = session.click_css(selector).await.map_err(|e| format!("{e}"))?;
            if found {
                ("succeeded", format!("css_clicked: {selector}"))
            } else {
                ("not_found", format!("css_not_found: {selector}"))
            }
        }
        "click_at" => {
            let x = args["x"].as_f64().ok_or("Missing 'x' coordinate")?;
            let y = args["y"].as_f64().ok_or("Missing 'y' coordinate")?;
            session.click_at(x, y).await.map_err(|e| format!("{e}"))?;
            ("succeeded", format!("clicked_at: ({x}, {y})"))
        }
        "type_react" => {
            let selector = args["selector"].as_str().ok_or("Missing 'selector' for type_react")?;
            let value = args["value"].as_str().ok_or("Missing 'value' for type_react")?;
            let ok = session.type_react(selector, value).await.map_err(|e| format!("{e}"))?;
            if ok {
                ("succeeded", format!("react_typed: {value}"))
            } else {
                ("not_found", format!("react_input_not_found: {selector}"))
            }
        }
        "press_combo" => {
            let combo = args["combo"].as_str().ok_or("Missing 'combo' (e.g. 'Ctrl+a')")?;
            session.press_combo(combo).await.map_err(|e| format!("{e}"))?;
            ("succeeded", format!("combo: {combo}"))
        }
        "wait_for" => {
            let selector = args["selector"].as_str().ok_or("Missing 'selector' for wait_for")?;
            let timeout = args["timeout_ms"].as_u64().unwrap_or(10000);
            let found = session.wait_for_selector(selector, timeout).await.map_err(|e| format!("{e}"))?;
            if found {
                ("succeeded", format!("element_found: {selector}"))
            } else {
                ("timeout", format!("wait_timeout: {selector} after {timeout}ms"))
            }
        }
        "scroll_to" => {
            let selector = args["selector"].as_str().ok_or("Missing 'selector' for scroll_to")?;
            let found = session.scroll_to(selector).await.map_err(|e| format!("{e}"))?;
            if found {
                ("succeeded", format!("scrolled_to: {selector}"))
            } else {
                ("not_found", format!("scroll_target_not_found: {selector}"))
            }
        }
        "send_message" => {
            // Universal contenteditable message sender.
            // Works with LinkedIn, Slack, Discord, etc — any site with contenteditable + send button.
            // Uses execCommand('insertText') + InputEvent to activate React/frameworks.
            let text = args["text"].as_str().ok_or("Missing 'text' for send_message")?;
            let input_sel = args["input_selector"].as_str().unwrap_or("div[contenteditable='true']");
            let button_sel = args["button_selector"].as_str().unwrap_or("");
            let result = session.send_message(text, input_sel, button_sel).await.map_err(|e| format!("{e}"))?;
            match result.as_str() {
                "SENT" => ("succeeded", format!("message_sent: {} chars", text.len())),
                other => ("failed", format!("send_failed: {other}")),
            }
        }
        "drag" => {
            let from_x = args["from_x"].as_f64().unwrap_or(0.0);
            let from_y = args["from_y"].as_f64().unwrap_or(0.0);
            let to_x = args["to_x"].as_f64().unwrap_or(0.0);
            let to_y = args["to_y"].as_f64().unwrap_or(0.0);
            session.drag(from_x, from_y, to_x, to_y).await.map_err(|e| format!("{e}"))?;
            ("succeeded", "dragged".to_string())
        }
        "upload" => {
            let selector = args["selector"].as_str().unwrap_or("input[type=file]");
            let files: Vec<String> = args["files"].as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();
            session.upload_file(selector, &files).await.map_err(|e| format!("{e}"))?;
            ("succeeded", format!("uploaded {} files", files.len()))
        }
        "clipboard_read" => {
            let text = session.clipboard_read().await.map_err(|e| format!("{e}"))?;
            ("succeeded", format!("clipboard: {text}"))
        }
        "clipboard_write" => {
            let text = args["text"].as_str().unwrap_or("");
            session.clipboard_write(text).await.map_err(|e| format!("{e}"))?;
            ("succeeded", format!("clipboard written: {} chars", text.len()))
        }
        "mouse" => {
            let action = args["mouse_action"].as_str().unwrap_or("move");
            let x = args["x"].as_f64().unwrap_or(0.0);
            let y = args["y"].as_f64().unwrap_or(0.0);
            let button = args["button"].as_str().unwrap_or("left");
            match action {
                "move" => session.mouse_move(x, y).await.map_err(|e| format!("{e}"))?,
                "down" => session.mouse_down(x, y, button).await.map_err(|e| format!("{e}"))?,
                "up" => session.mouse_up(x, y, button).await.map_err(|e| format!("{e}"))?,
                "wheel" => {
                    let dx = args["delta_x"].as_f64().unwrap_or(0.0);
                    let dy = args["delta_y"].as_f64().unwrap_or(0.0);
                    session.mouse_wheel(x, y, dx, dy).await.map_err(|e| format!("{e}"))?;
                }
                _ => return Err(format!("Unknown mouse action: {action}")),
            };
            ("succeeded", format!("mouse {action} at ({x},{y})"))
        }
        "highlight" => {
            let selector = args["selector"].as_str().or_else(|| args["target"].as_str()).unwrap_or("");
            session.highlight(selector).await.map_err(|e| format!("{e}"))?;
            ("succeeded", "highlighted".to_string())
        }
        "get_info" => {
            let selector = args["selector"].as_str().or_else(|| args["target"].as_str()).unwrap_or("");
            let what = args["what"].as_str().unwrap_or("text");
            let info = session.get_element_info(selector, what).await.map_err(|e| format!("{e}"))?;
            ("succeeded", format!("{what}: {info}"))
        }
        "screenshot_annotated" => {
            let result = session.screenshot_annotated().await.map_err(|e| format!("{e}"))?;
            ("succeeded", result)
        }
        "bounds" => {
            let selector = args["selector"].as_str().ok_or("Missing 'selector' for bounds")?;
            let bounds = session.get_element_bounds(selector).await.map_err(|e| format!("{e}"))?;
            if let Some((x, y, w, h)) = bounds {
                ("succeeded", format!("bounds: x={x},y={y},w={w},h={h}"))
            } else {
                ("not_found", format!("bounds_not_found: {selector}"))
            }
        }
        "drag" => {
            let from_x = args["from_x"].as_f64().unwrap_or(0.0);
            let from_y = args["from_y"].as_f64().unwrap_or(0.0);
            let to_x = args["to_x"].as_f64().unwrap_or(0.0);
            let to_y = args["to_y"].as_f64().unwrap_or(0.0);
            session.drag(from_x, from_y, to_x, to_y).await.map_err(|e| format!("{e}"))?;
            ("succeeded", "dragged".to_string())
        }
        "upload" => {
            let selector = args["selector"].as_str().unwrap_or("input[type=file]");
            let files: Vec<String> = args["files"].as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();
            session.upload_file(selector, &files).await.map_err(|e| format!("{e}"))?;
            ("succeeded", format!("uploaded {} files", files.len()))
        }
        "clipboard_read" => {
            let text = session.clipboard_read().await.map_err(|e| format!("{e}"))?;
            ("succeeded", format!("clipboard: {text}"))
        }
        "clipboard_write" => {
            let text = args["text"].as_str().unwrap_or("");
            session.clipboard_write(text).await.map_err(|e| format!("{e}"))?;
            ("succeeded", format!("clipboard written: {} chars", text.len()))
        }
        "mouse" => {
            let action = args["mouse_action"].as_str().unwrap_or("move");
            let x = args["x"].as_f64().unwrap_or(0.0);
            let y = args["y"].as_f64().unwrap_or(0.0);
            let button = args["button"].as_str().unwrap_or("left");
            match action {
                "move" => session.mouse_move(x, y).await.map_err(|e| format!("{e}"))?,
                "down" => session.mouse_down(x, y, button).await.map_err(|e| format!("{e}"))?,
                "up" => session.mouse_up(x, y, button).await.map_err(|e| format!("{e}"))?,
                "wheel" => {
                    let dx = args["delta_x"].as_f64().unwrap_or(0.0);
                    let dy = args["delta_y"].as_f64().unwrap_or(0.0);
                    session.mouse_wheel(x, y, dx, dy).await.map_err(|e| format!("{e}"))?;
                }
                _ => return Err(format!("Unknown mouse action: {action}")),
            };
            ("succeeded", format!("mouse {action} at ({x},{y})"))
        }
        "highlight" => {
            let selector = args["selector"].as_str().or_else(|| args["target"].as_str()).unwrap_or("");
            session.highlight(selector).await.map_err(|e| format!("{e}"))?;
            ("succeeded", "highlighted".to_string())
        }
        "analyze_forms" => {
            let result = session.analyze_forms().await.map_err(|e| format!("{e}"))?;
            ("succeeded", result)
        }
        "analyze_api" => {
            let result = session.analyze_api_from_js().await.map_err(|e| format!("{e}"))?;
            ("succeeded", result)
        }
        "list_frames" => {
            let frames = session.list_frames().await.map_err(|e| format!("{e}"))?;
            let count = frames.as_array().map(|a| a.len()).unwrap_or(0);
            let fi = session.active_frame_info();
            let summary = format!("frames: {} total, active: {} ({})", count, fi["frame_id"].as_str().unwrap_or("main"), fi["mode"].as_str().unwrap_or("main"));
            ("succeeded", format!("{}\n{}", summary, serde_json::to_string_pretty(&frames).unwrap_or_default()))
        }
        "switch_frame" => {
            let ft = args["target"].as_str().unwrap_or("");
            if ft.is_empty() { return Err("Missing 'target' for switch_frame (index or URL pattern)".into()); }
            let frames = session.list_frames().await.map_err(|e| format!("{e}"))?;
            let arr = frames.as_array().ok_or("No frames")?;
            if let Ok(idx) = ft.parse::<usize>() {
                if idx >= arr.len() { return Err(format!("Frame index {} out of range (have {})", idx, arr.len())); }
                let fid = arr[idx]["id"].as_str().ok_or("No frame id")?;
                let msg = session.switch_frame(fid).await.map_err(|e| format!("{e}"))?;
                ("succeeded", msg)
            } else {
                let pat = ft.to_lowercase();
                let found = arr.iter().find(|f| f["url"].as_str().map(|u| u.to_lowercase().contains(&pat)).unwrap_or(false) || f["name"].as_str().map(|n| n.to_lowercase().contains(&pat)).unwrap_or(false));
                match found {
                    Some(frame) => { let fid = frame["id"].as_str().ok_or("No frame id")?; let msg = session.switch_frame(fid).await.map_err(|e| format!("{e}"))?; ("succeeded", msg) }
                    None => ("not_found", format!("No frame matching '{}' found", ft))
                }
            }
        }
        "auto_frame" => {
            let text = args["target"].as_str().or_else(|| args["text"].as_str()).unwrap_or("");
            if text.is_empty() { return Err("Missing text hint for auto_frame".into()); }
            let msg = session.auto_switch_frame(text).await.map_err(|e| format!("{e}"))?;
            ("succeeded", msg)
        }
        "main_frame" => {
            let msg = session.switch_to_main_frame().await.map_err(|e| format!("{e}"))?;
            ("succeeded", msg)
        }
        _ => return Err(format!("Unknown action kind: {kind}")),
    };

    let ok = outcome == "succeeded";

    // Return observation if requested
    let mut result = serde_json::json!({
        "ok": ok,
        "outcome": outcome,
        "effect": effect,
        "resolved_target": target,
        "recoverable": !ok,
    });

    if return_obs != "none" {
        let session = state.session.as_mut().unwrap();

        match return_obs {
            "see" => {
                // Fast path: JS-only extraction, no WOM
                let page = session.see_page().await.map_err(|e| format!("{e}"))?;
                result["page"] = serde_json::json!(page);
            }
            "delta" | "compact" => {
                let rev = state.wom_revision + 1;
                state.wom_revision = rev;
                let doc = session.see_wom(rev).await.map_err(|e| format!("{e}"))?;

                if return_obs == "delta" {
                    if let Some(ref prev) = state.prev_wom {
                        let d = delta::diff(prev, &doc);
                        result["delta"] = serde_json::to_value(&d).unwrap_or_default();
                    }
                    result["compact"] = serde_json::to_value(&wom::compact(&doc)).unwrap_or_default();
                } else {
                    result["compact"] = serde_json::to_value(&wom::compact(&doc)).unwrap_or_default();
                }
                result["revision"] = serde_json::json!(doc.session.revision);
                state.prev_wom = Some(doc);
            }
            _ => {}
        }
    }

    // Record trace if enabled
    if state.trace.is_enabled() {
        let url = state.session.as_ref().map(|s| s.last_url.clone()).unwrap_or_default();
        state.trace.record(
            kind,
            &raw_target,
            outcome,
            &effect,
            act_t0.elapsed().as_millis() as u64,
            &url,
            if ok { None } else { Some(effect.clone()) },
        );
    }

    Ok(result)
}

async fn handle_wait(state: &mut McpState, args: &Value) -> Result<Value, String> {
    let timeout_ms = args["timeout_ms"].as_u64().unwrap_or(10000);

    // Simple time wait
    if let Some(secs) = args["seconds"].as_f64() {
        let session = state.ensure_session().await?;
        let session = state.session.as_ref().unwrap();
        session.wait(secs).await;
        return Ok(serde_json::json!({
            "ok": true,
            "reason": format!("waited {secs}s"),
        }));
    }

    // Text-based wait (poll)
    let text_present = args["text_present"].as_str();
    let text_absent = args["text_absent"].as_str();

    if text_present.is_none() && text_absent.is_none() {
        // Just wait for a network idle equivalent
        let session = state.ensure_session().await?;
        let session = state.session.as_ref().unwrap();
        session.wait(1.0).await;
        return Ok(serde_json::json!({
            "ok": true,
            "reason": "waited 1s (no condition specified)",
        }));
    }

    let start = std::time::Instant::now();
    let deadline = std::time::Duration::from_millis(timeout_ms);

    loop {
        if start.elapsed() > deadline {
            return Ok(serde_json::json!({
                "ok": false,
                "matched": false,
                "reason": "timeout",
            }));
        }

        let rev = state.wom_revision + 1;
        state.wom_revision = rev;
        let session = state.session.as_mut().unwrap();
        let doc = session.see_wom(rev).await.map_err(|e| format!("{e}"))?;

        // Serialize all text for matching
        let all_text = doc.nodes.iter()
            .map(|n| n.name.as_str())
            .chain(doc.content.headings.iter().map(|h| h.text.as_str()))
            .chain(doc.content.paragraphs.iter().map(|p| p.text.as_str()))
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();

        if let Some(target) = text_present {
            if all_text.contains(&target.to_lowercase()) {
                let compact = wom::compact(&doc);
                state.prev_wom = Some(doc);
                return Ok(serde_json::json!({
                    "ok": true,
                    "matched": true,
                    "reason": format!("text found: {target}"),
                    "compact": compact,
                }));
            }
        }

        if let Some(target) = text_absent {
            if !all_text.contains(&target.to_lowercase()) {
                let compact = wom::compact(&doc);
                state.prev_wom = Some(doc);
                return Ok(serde_json::json!({
                    "ok": true,
                    "matched": true,
                    "reason": format!("text absent: {target}"),
                    "compact": compact,
                }));
            }
        }

        state.prev_wom = Some(doc);
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}

async fn handle_tabs(state: &mut McpState, args: &Value) -> Result<Value, String> {
    let op = args["op"].as_str().ok_or("Missing 'op'")?;
    let session = state.ensure_session().await?;
    let session = state.session.as_mut().unwrap();

    match op {
        "list" => {
            let tabs = session.pages().await.map_err(|e| format!("{e}"))?;
            Ok(serde_json::json!({
                "tabs": tabs,
            }))
        }
        "switch" => {
            let idx = args["index"].as_u64().ok_or("Missing 'index'")? as usize;
            session.switch_tab(idx).await.map_err(|e| format!("{e}"))?;
            Ok(serde_json::json!({
                "ok": true,
                "switched_to": idx,
            }))
        }
        _ => Err(format!("Unknown tab op: {op}")),
    }
}

async fn handle_session(state: &mut McpState, args: &Value) -> Result<Value, String> {
    let op = args["op"].as_str().ok_or("Missing 'op'")?;

    match op {
        "load_cookies" => {
            let file = args["cookies_file"].as_str().ok_or("Missing 'cookies_file'")?;
            let session = state.ensure_session().await?;
            let session = state.session.as_mut().unwrap();
            let count = session.load_cookies(file).await.map_err(|e| format!("{e}"))?;
            Ok(serde_json::json!({
                "ok": true,
                "cookies_loaded": count,
            }))
        }
        "screenshot" => {
            let session = state.ensure_session().await?;
            let session = state.session.as_mut().unwrap();
            let data = session.screenshot().await.map_err(|e| format!("{e}"))?;
            let path = "/tmp/neo_screenshot.jpg";
            std::fs::write(path, &data).map_err(|e| format!("{e}"))?;
            Ok(serde_json::json!({
                "ok": true,
                "path": path,
                "size_kb": data.len() / 1024,
            }))
        }
        "reset" => {
            if let Some(session) = state.session.take() {
                session.close().await.ok();
            }
            state.wom_revision = 0;
            state.prev_wom = None;
            Ok(serde_json::json!({
                "ok": true,
                "effect": "session_reset",
            }))
        }
        "start_capture" => {
            let session = state.ensure_session().await?;
            let session = state.session.as_mut().unwrap();
            session.start_network_capture().await.map_err(|e| format!("{e}"))?;
            session.start_console_capture().await.map_err(|e| format!("{e}"))?;
            session.setup_dialog_handler().await.map_err(|e| format!("{e}"))?;
            Ok(serde_json::json!({
                "ok": true,
                "effect": "network+console+dialog capture enabled",
            }))
        }
        "network" => {
            let session = state.ensure_session().await?;
            let session = state.session.as_mut().unwrap();
            let reqs = session.read_network().await.map_err(|e| format!("{e}"))?;
            Ok(serde_json::json!({
                "ok": true,
                "requests": reqs,
                "count": reqs.len(),
            }))
        }
        "console" => {
            let session = state.ensure_session().await?;
            let session = state.session.as_mut().unwrap();
            let msgs = session.read_console().await.map_err(|e| format!("{e}"))?;
            Ok(serde_json::json!({
                "ok": true,
                "messages": msgs,
                "count": msgs.len(),
            }))
        }
        "dialogs" => {
            let session = state.ensure_session().await?;
            let session = state.session.as_mut().unwrap();
            let dlgs = session.get_dialogs().await.map_err(|e| format!("{e}"))?;
            Ok(serde_json::json!({
                "ok": true,
                "dialogs": dlgs,
                "count": dlgs.len(),
            }))
        }
        "pdf" => {
            let session = state.ensure_session().await?;
            let session = state.session.as_mut().unwrap();
            let path = args["path"].as_str();
            let result = session.pdf(path).await.map_err(|e| format!("{e}"))?;
            Ok(serde_json::json!({"ok": true, "result": result}))
        }
        "device" => {
            let session = state.ensure_session().await?;
            let session = state.session.as_mut().unwrap();
            let width = args["width"].as_u64().unwrap_or(1440) as u32;
            let height = args["height"].as_u64().unwrap_or(900) as u32;
            let scale = args["scale"].as_f64().unwrap_or(1.0);
            let mobile = args["mobile"].as_bool().unwrap_or(false);
            let ua = args["user_agent"].as_str();
            session.set_device(width, height, scale, mobile, ua).await.map_err(|e| format!("{e}"))?;
            Ok(serde_json::json!({"ok": true, "device": format!("{width}x{height} @{scale}x mobile={mobile}")}))
        }
        "geo" => {
            let session = state.ensure_session().await?;
            let session = state.session.as_mut().unwrap();
            let lat = args["lat"].as_f64().ok_or("Missing lat")?;
            let lng = args["lng"].as_f64().ok_or("Missing lng")?;
            session.set_geolocation(lat, lng, None).await.map_err(|e| format!("{e}"))?;
            Ok(serde_json::json!({"ok": true, "geolocation": format!("{lat}, {lng}")}))
        }
        "offline" => {
            let session = state.ensure_session().await?;
            let session = state.session.as_mut().unwrap();
            let enabled = args["enabled"].as_bool().unwrap_or(true);
            session.set_offline(enabled).await.map_err(|e| format!("{e}"))?;
            Ok(serde_json::json!({"ok": true, "offline": enabled}))
        }
        "color_scheme" => {
            let session = state.ensure_session().await?;
            let session = state.session.as_mut().unwrap();
            let scheme = args["scheme"].as_str().unwrap_or("dark");
            session.set_color_scheme(scheme).await.map_err(|e| format!("{e}"))?;
            Ok(serde_json::json!({"ok": true, "color_scheme": scheme}))
        }
        _ => Err(format!("Unknown session op: {op}")),
    }
}

// ─── Auth handler ───

async fn handle_auth(state: &mut McpState, args: &Value) -> Result<Value, String> {
    let op = args["op"].as_str().ok_or("Missing 'op'")?;

    match op {
        "profiles" => {
            let profiles = auth::list_profiles()?;
            let summary: Vec<Value> = profiles
                .iter()
                .map(|p| {
                    serde_json::json!({
                        "profile_id": p.profile_id,
                        "display_name": p.display_name,
                        "domains": p.domains,
                        "totp_enabled": p.totp_enabled,
                        "has_username": auth::SecretStore::get(&p.profile_id, "username").ok().flatten().is_some(),
                        "has_password": auth::SecretStore::get(&p.profile_id, "password").ok().flatten().is_some(),
                    })
                })
                .collect();
            Ok(serde_json::json!({
                "ok": true,
                "profiles": summary,
                "count": summary.len(),
            }))
        }

        "add_profile" => {
            let profile_id = args["profile_id"].as_str().ok_or("Missing 'profile_id'")?;
            let display_name = args["display_name"].as_str().unwrap_or(profile_id);
            let domains: Vec<String> = args["domains"]
                .as_array()
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();
            let login_url = args["login_url"].as_str().map(String::from);
            let totp_enabled = args["totp_enabled"].as_bool().unwrap_or(false);

            if domains.is_empty() {
                return Err("At least one domain required".into());
            }

            let now = chrono::Utc::now();
            let profile = auth::AuthProfile {
                profile_id: profile_id.to_string(),
                display_name: display_name.to_string(),
                domains,
                login_url,
                username_field: None,
                password_field: None,
                totp_enabled,
                preferred_backend: auth::SessionBackend::ManagedCookies,
                created_at: now,
                updated_at: now,
            };
            auth::save_profile(&profile)?;
            Ok(serde_json::json!({
                "ok": true,
                "effect": format!("Profile '{}' created", profile_id),
                "profile": profile,
            }))
        }

        "delete_profile" => {
            let profile_id = args["profile_id"].as_str().ok_or("Missing 'profile_id'")?;
            auth::delete_profile(profile_id)?;
            Ok(serde_json::json!({
                "ok": true,
                "effect": format!("Profile '{}' deleted (including credentials and sessions)", profile_id),
            }))
        }

        "set_credential" => {
            let profile_id = args["profile_id"].as_str().ok_or("Missing 'profile_id'")?;
            let kind = args["credential_kind"].as_str().ok_or("Missing 'credential_kind'")?;
            let value = args["credential_value"].as_str().ok_or("Missing 'credential_value'")?;

            // Verify profile exists
            auth::load_profile(profile_id)?
                .ok_or_else(|| format!("Profile '{}' not found", profile_id))?;

            // Validate kind
            if !["username", "password", "totp_seed"].contains(&kind) {
                return Err(format!("Invalid credential_kind: {kind}"));
            }

            auth::SecretStore::set(profile_id, kind, value)?;
            // SECURITY: Never log the actual value
            Ok(serde_json::json!({
                "ok": true,
                "effect": format!("{kind} stored in OS keychain for profile '{profile_id}'"),
                "note": "Credential stored securely — value is NOT logged or returned",
            }))
        }

        "login" => {
            let profile_id = args["profile_id"].as_str().ok_or("Missing 'profile_id'")?;
            let profile = auth::load_profile(profile_id)?
                .ok_or_else(|| format!("Profile '{}' not found", profile_id))?;

            // Get login URL
            let login_url = profile
                .login_url
                .as_deref()
                .ok_or("Profile has no login_url configured")?;

            // Try to load saved session first
            let domain = profile.domains.first().ok_or("Profile has no domains")?;
            if let Some(saved) = auth::load_session(profile_id, domain)? {
                if saved.health.status == auth::HealthStatus::Valid {
                    // Inject saved cookies
                    let session = state.ensure_session().await?;
                    let session = state.session.as_mut().unwrap();
                    let cookies_json = serde_json::to_string(&saved.cookies).map_err(|e| format!("{e}"))?;
                    let tmp_path = "/tmp/neo_auth_cookies.json";
                    std::fs::write(tmp_path, &cookies_json).map_err(|e| format!("{e}"))?;
                    session.load_cookies(tmp_path).await.map_err(|e| format!("{e}"))?;
                    session.goto(login_url).await.map_err(|e| format!("{e}"))?;

                    // Check if we're still logged in
                    let rev = state.next_revision();
                    let session = state.session.as_mut().unwrap();
                    let doc = session.see_wom(rev).await.map_err(|e| format!("{e}"))?;
                    if doc.page.page_class != "login" {
                        state.auth_state = auth::AuthState::Authenticated;
                        state.prev_wom = Some(doc);
                        return Ok(serde_json::json!({
                            "ok": true,
                            "status": "authenticated",
                            "method": "saved_session",
                            "message": format!("Logged in to {} using saved session", domain),
                        }));
                    }
                    // Session expired — fall through to fresh login
                }
            }

            // Fresh login — navigate and fill credentials
            let session = state.ensure_session().await?;
            let session = state.session.as_mut().unwrap();
            session.goto(login_url).await.map_err(|e| format!("{e}"))?;

            // Get credentials from keychain
            let username = auth::SecretStore::get(profile_id, "username")?;
            let password = auth::SecretStore::get(profile_id, "password")?;

            if username.is_none() || password.is_none() {
                state.auth_state = auth::AuthState::Failed("Missing credentials in keychain".into());
                return Ok(serde_json::json!({
                    "ok": false,
                    "status": "missing_credentials",
                    "message": "Set credentials first with set_credential op",
                    "has_username": username.is_some(),
                    "has_password": password.is_some(),
                }));
            }

            // Auto-fill username and password
            state.auth_state = auth::AuthState::FillingCredentials;
            let session = state.session.as_mut().unwrap();

            // Type username — look for common selectors
            let username_val = username.unwrap();
            let password_val = password.unwrap();

            // Use fill_form for both fields
            let fields = vec![
                ("email".to_string(), username_val.clone()),
                ("username".to_string(), username_val.clone()),
                ("login".to_string(), username_val),
                ("password".to_string(), password_val),
            ];
            // Try fill_form, ignore individual field failures
            let _ = session.fill_form(&fields).await;

            // Submit
            state.auth_state = auth::AuthState::SubmittingLogin;
            let session = state.session.as_mut().unwrap();
            session.press("Enter").await.map_err(|e| format!("{e}"))?;

            // Wait for navigation
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;

            // Observe result
            let rev = state.next_revision();
            let session = state.session.as_mut().unwrap();
            let doc = session.see_wom(rev).await.map_err(|e| format!("{e}"))?;
            let page_class = doc.page.page_class.clone();

            // Check if TOTP is needed
            if profile.totp_enabled {
                if let Ok(code) = auth::generate_totp(profile_id) {
                    // Auto-fill TOTP
                    state.auth_state = auth::AuthState::FillingTotp;
                    let session = state.session.as_mut().unwrap();
                    // Type code into whatever field is focused
                    session.type_text(&code).await.map_err(|e| format!("{e}"))?;
                    session.press("Enter").await.map_err(|e| format!("{e}"))?;
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

                    let rev = state.next_revision();
                    let session = state.session.as_mut().unwrap();
                    let doc = session.see_wom(rev).await.map_err(|e| format!("{e}"))?;
                    state.prev_wom = Some(doc.clone());

                    if doc.page.page_class != "login" {
                        state.auth_state = auth::AuthState::Authenticated;
                        return Ok(serde_json::json!({
                            "ok": true,
                            "status": "authenticated",
                            "method": "credentials+totp",
                            "page_class": doc.page.page_class,
                        }));
                    }
                }
            }

            // Check if we need 2FA (page still looks like a verification/login challenge)
            let has_auth_intent = doc.goal_surface.primary_intents
                .iter()
                .any(|i| i.intent == "authenticate");
            let needs_challenge = page_class == "login"
                || page_class == "form"
                || has_auth_intent;

            state.prev_wom = Some(doc);

            if needs_challenge && !profile.totp_enabled {
                // Create a challenge for the user
                let challenge = auth::create_challenge(
                    profile_id,
                    &profile.domains[0],
                    auth::ChallengeType::Unknown,
                    "The site is asking for additional verification. Please provide the code from your SMS, email, or authenticator app.",
                    None,
                );
                let challenge_id = challenge.challenge_id.clone();
                state.auth_state = auth::AuthState::AwaitingChallenge(challenge.clone());
                state.pending_challenge = Some(challenge);

                return Ok(serde_json::json!({
                    "ok": true,
                    "status": "requires_user_input",
                    "challenge_id": challenge_id,
                    "challenge_type": "unknown",
                    "message": "The site is asking for additional verification. Please ask the user for their 2FA code, then call browser_auth with op='resume_challenge'.",
                    "page_class": page_class,
                }));
            }

            if page_class != "login" {
                state.auth_state = auth::AuthState::Authenticated;
                Ok(serde_json::json!({
                    "ok": true,
                    "status": "authenticated",
                    "method": "credentials",
                    "page_class": page_class,
                    "hint": "Call save_session to persist this login for future use",
                }))
            } else {
                state.auth_state = auth::AuthState::Failed("Login page still showing after submit".into());
                Ok(serde_json::json!({
                    "ok": false,
                    "status": "login_failed",
                    "page_class": page_class,
                    "message": "Still on login page after credential submission. Check credentials or handle manually.",
                }))
            }
        }

        "resume_challenge" => {
            let challenge_id = args["challenge_id"].as_str().ok_or("Missing 'challenge_id'")?;
            let response = args["challenge_response"].as_str().ok_or("Missing 'challenge_response'")?;

            // Verify we have a matching pending challenge
            let challenge = state
                .pending_challenge
                .as_ref()
                .ok_or("No pending challenge")?;
            if challenge.challenge_id != challenge_id {
                return Err(format!(
                    "Challenge ID mismatch: expected {}, got {challenge_id}",
                    challenge.challenge_id
                ));
            }
            let profile_id = challenge.profile_id.clone();

            // Type the response into the browser
            let session = state.session.as_mut().ok_or("No browser session")?;
            session.type_text(response).await.map_err(|e| format!("{e}"))?;
            session.press("Enter").await.map_err(|e| format!("{e}"))?;

            // Wait for navigation
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;

            // Check result
            let rev = state.next_revision();
            let session = state.session.as_mut().unwrap();
            let doc = session.see_wom(rev).await.map_err(|e| format!("{e}"))?;
            let page_class = doc.page.page_class.clone();
            state.prev_wom = Some(doc);
            state.pending_challenge = None;

            if page_class != "login" {
                state.auth_state = auth::AuthState::Authenticated;
                Ok(serde_json::json!({
                    "ok": true,
                    "status": "authenticated",
                    "method": "credentials+challenge",
                    "page_class": page_class,
                    "hint": "Call save_session to persist this login",
                }))
            } else {
                state.auth_state = auth::AuthState::Failed("Still on login after challenge".into());
                Ok(serde_json::json!({
                    "ok": false,
                    "status": "challenge_failed",
                    "page_class": page_class,
                    "message": "Still on login/verification page. The code may have been wrong or expired.",
                }))
            }
        }

        "check_session" => {
            let profile_id = args["profile_id"].as_str().ok_or("Missing 'profile_id'")?;
            let domain = args["domain"].as_str().ok_or("Missing 'domain'")?;

            match auth::load_session(profile_id, domain)? {
                Some(session) => {
                    let age_hours = (chrono::Utc::now() - session.updated_at).num_hours();
                    Ok(serde_json::json!({
                        "ok": true,
                        "exists": true,
                        "health": session.health.status,
                        "age_hours": age_hours,
                        "cookies_count": session.cookies.len(),
                        "last_verified": session.last_verified_at,
                    }))
                }
                None => Ok(serde_json::json!({
                    "ok": true,
                    "exists": false,
                    "message": format!("No saved session for {profile_id}/{domain}"),
                })),
            }
        }

        "save_session" => {
            let profile_id = args["profile_id"].as_str().ok_or("Missing 'profile_id'")?;
            let domain = args["domain"].as_str().ok_or("Missing 'domain'")?;

            let session = state.session.as_mut().ok_or("No browser session")?;

            // Export cookies
            let cookies = session
                .get_all_cookies()
                .await
                .unwrap_or_else(|_| vec![]);

            // Export localStorage
            let local_storage = session
                .get_local_storage()
                .await
                .unwrap_or_default();

            let ls_count = local_storage.len();
            let stored = auth::create_session_from_cookies(
                profile_id,
                domain,
                cookies,
                local_storage,
                vec![],
            );
            auth::save_session(&stored)?;

            Ok(serde_json::json!({
                "ok": true,
                "effect": format!("Session saved for {profile_id}/{domain}"),
                "cookies_count": stored.cookies.len(),
                "local_storage_count": ls_count,
                "session_id": stored.session_id,
                "path": format!("~/.neobrowser/sessions/{profile_id}/{domain}.json"),
            }))
        }

        "auto_session" => {
            let domain = args["domain"].as_str().ok_or("Missing 'domain'")?;

            // Find a profile for this domain
            let profile = auth::find_profile_for_domain(domain)?;
            if profile.is_none() {
                return Ok(serde_json::json!({
                    "ok": true,
                    "has_profile": false,
                    "has_session": false,
                    "message": format!("No auth profile found for domain '{domain}'"),
                }));
            }
            let profile = profile.unwrap();

            // Try to load saved session
            let saved = auth::load_session(&profile.profile_id, domain)?;
            if saved.is_none() {
                return Ok(serde_json::json!({
                    "ok": true,
                    "has_profile": true,
                    "profile_id": profile.profile_id,
                    "has_session": false,
                    "message": format!("Profile '{}' exists but no saved session. Use login op.", profile.profile_id),
                }));
            }
            let saved = saved.unwrap();

            // Navigate to domain first (cookies/localStorage need a page context)
            let target_url = profile
                .login_url
                .clone()
                .unwrap_or_else(|| format!("https://{domain}/"));

            let session = state.ensure_session().await?;
            let session = state.session.as_mut().unwrap();
            session.goto(&target_url).await.map_err(|e| format!("navigate: {e}"))?;
            // Wait for SPA to render
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;

            // Inject cookies
            if !saved.cookies.is_empty() {
                let cookies_json = serde_json::to_string(&saved.cookies).map_err(|e| format!("{e}"))?;
                let tmp_path = "/tmp/neo_auth_cookies.json";
                std::fs::write(tmp_path, &cookies_json).map_err(|e| format!("{e}"))?;
                let session = state.session.as_mut().unwrap();
                session.load_cookies(tmp_path).await.ok();
            }

            // Inject localStorage (critical for JWT-based auth like Mercadona)
            let ls_count = saved.local_storage.len();
            if !saved.local_storage.is_empty() {
                let session = state.session.as_mut().unwrap();
                session
                    .set_local_storage(&saved.local_storage)
                    .await
                    .map_err(|e| format!("localStorage inject: {e}"))?;
            }

            // Reload to apply injected session
            let session = state.session.as_mut().unwrap();
            session.reload().await.map_err(|e| format!("reload: {e}"))?;
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;

            // Verify auth state via WOM
            let rev = state.next_revision();
            let session = state.session.as_mut().unwrap();
            let doc = session.see_wom(rev).await.map_err(|e| format!("{e}"))?;
            let page_class = doc.page.page_class.clone();
            let authenticated = page_class != "login";
            state.prev_wom = Some(doc);

            let age_hours = (chrono::Utc::now() - saved.updated_at).num_hours();

            Ok(serde_json::json!({
                "ok": true,
                "has_profile": true,
                "has_session": true,
                "authenticated": authenticated,
                "profile_id": profile.profile_id,
                "cookies_injected": saved.cookies.len(),
                "local_storage_injected": ls_count,
                "session_age_hours": age_hours,
                "health": saved.health.status,
                "page_class": page_class,
                "message": if authenticated { "Session restored — authenticated!" } else { "Session injected but page still shows login." },
            }))
        }

        "extract_chrome" => {
            // Connect to user's real Chrome via CDP, navigate to domain, extract session.
            // Requires Chrome running with --remote-debugging-port or DevToolsActivePort.
            let profile_id = args["profile_id"].as_str().ok_or("Missing 'profile_id'")?;
            let domain = args["domain"].as_str().ok_or("Missing 'domain'")?;
            let port = args["port"].as_u64().unwrap_or(9222) as u16;

            // Connect to real Chrome
            let mut real_chrome = engine::Session::connect_port(port)
                .await
                .map_err(|e| format!("Cannot connect to Chrome on port {port}: {e}. Launch Chrome with --remote-debugging-port={port}"))?;

            // Navigate to domain
            let url = format!("https://{domain}/");
            real_chrome
                .goto(&url)
                .await
                .map_err(|e| format!("navigate: {e}"))?;

            // Wait for page to settle
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;

            // Extract cookies + localStorage
            let cookies = real_chrome.get_all_cookies().await.unwrap_or_default();
            let local_storage = real_chrome.get_local_storage().await.unwrap_or_default();

            // Check page title for auth confirmation
            let title = real_chrome
                .eval("document.title")
                .await
                .unwrap_or_default();

            // Check for user indicators
            let user_indicator = real_chrome
                .eval(r#"(()=>{const el=document.querySelector('[class*=user],[class*=account],[class*=profile],[class*=greeting]');return el?el.textContent.trim().substring(0,80):''})()"#)
                .await
                .unwrap_or_default();

            let ls_count = local_storage.len();
            let cookie_count = cookies.len();

            // Save session
            let stored = auth::create_session_from_cookies(
                profile_id,
                domain,
                cookies,
                local_storage,
                vec![],
            );
            auth::save_session(&stored)?;

            // Close the CDP connection (don't close user's Chrome)
            // real_chrome goes out of scope without closing

            Ok(serde_json::json!({
                "ok": true,
                "effect": format!("Session extracted from real Chrome for {profile_id}/{domain}"),
                "cookies_count": cookie_count,
                "local_storage_count": ls_count,
                "page_title": title,
                "user_indicator": user_indicator,
                "session_id": stored.session_id,
                "authenticated": !user_indicator.is_empty(),
                "path": format!("~/.neobrowser/sessions/{profile_id}/{domain}.json"),
            }))
        }

        _ => Err(format!("Unknown auth op: {op}")),
    }
}

// ─── browser_api handler ───

async fn handle_api(state: &mut McpState, args: &Value) -> Result<Value, String> {
    let session = state.ensure_session().await?;
    let url = args["url"].as_str().ok_or("Missing 'url'")?;
    let method = args["method"].as_str().unwrap_or("GET");
    let body = args["body"].as_str().unwrap_or("");
    let content_type = args["content_type"].as_str().unwrap_or("application/x-www-form-urlencoded");
    let max_length = args["max_length"].as_u64().unwrap_or(8000) as usize;
    let extract = args["extract"].as_str().unwrap_or("text");

    // Build extra headers JS
    let headers_js = if let Some(headers) = args["headers"].as_object() {
        headers
            .iter()
            .map(|(k, v)| {
                format!(
                    "x.setRequestHeader({},{});",
                    serde_json::to_string(k).unwrap_or_default(),
                    serde_json::to_string(v.as_str().unwrap_or("")).unwrap_or_default()
                )
            })
            .collect::<Vec<_>>()
            .join("")
    } else {
        String::new()
    };

    // Build body JS
    let body_js = if body.is_empty() {
        "x.send();".to_string()
    } else {
        format!(
            "x.setRequestHeader('Content-Type',{ct});x.send({body});",
            ct = serde_json::to_string(content_type).unwrap_or_default(),
            body = serde_json::to_string(body).unwrap_or_default()
        )
    };

    // Extract mode determines how we parse the response
    let extract_js = match extract {
        "json" => "try{JSON.parse(x.responseText)}catch(e){x.responseText}".to_string(),
        "html" => format!("x.responseText.substring(0,{})", max_length),
        "headers" => "x.getAllResponseHeaders()".to_string(),
        _ => {
            // "text" — parse HTML and get innerText
            format!(
                "(function(){{var d=document.createElement('div');d.innerHTML=x.responseText;return d.innerText.substring(0,{});}})()",
                max_length
            )
        }
    };

    let js = format!(
        r#"(function(){{
            var x = new XMLHttpRequest();
            x.open({method},{url},false);
            {headers}
            {body}
            return JSON.stringify({{
                status: x.status,
                statusText: x.statusText,
                data: {extract}
            }});
        }})()"#,
        method = serde_json::to_string(method).unwrap_or_default(),
        url = serde_json::to_string(url).unwrap_or_default(),
        headers = headers_js,
        body = body_js,
        extract = extract_js,
    );

    let result = session.eval_string(&js).await.map_err(|e| format!("{e}"))?;

    // Parse the JSON result
    let parsed: Value = serde_json::from_str(&result).unwrap_or_else(|_| {
        serde_json::json!({
            "status": 0,
            "error": "Failed to parse XHR response",
            "raw": result.chars().take(500).collect::<String>()
        })
    });

    Ok(serde_json::json!({
        "ok": parsed["status"].as_u64().unwrap_or(0) >= 200 && parsed["status"].as_u64().unwrap_or(0) < 400,
        "status": parsed["status"],
        "data": parsed["data"],
    }))
}

// ─── browser_state handler ───

async fn handle_state(state: &mut McpState, args: &Value) -> Result<Value, String> {
    let op = args["op"].as_str().ok_or("Missing 'op'")?;
    let session = state.ensure_session().await?;
    let session = state.session.as_mut().unwrap();

    match op {
        "export" => {
            let data = session.export_state().await.map_err(|e| format!("{e}"))?;

            // Optionally save to file
            if let Some(file) = args["file"].as_str() {
                let json = serde_json::to_string_pretty(&data).map_err(|e| format!("{e}"))?;
                std::fs::write(file, json).map_err(|e| format!("{e}"))?;
                return Ok(serde_json::json!({
                    "ok": true,
                    "effect": format!("State exported to {file}"),
                    "cookies": data["cookies"].as_array().map(|a| a.len()).unwrap_or(0),
                    "localStorage": data["localStorage"].as_object().map(|o| o.len()).unwrap_or(0),
                    "sessionStorage": data["sessionStorage"].as_object().map(|o| o.len()).unwrap_or(0),
                }));
            }

            Ok(data)
        }
        "import" => {
            let data = if let Some(file) = args["file"].as_str() {
                let content = std::fs::read_to_string(file).map_err(|e| format!("{e}"))?;
                serde_json::from_str::<Value>(&content).map_err(|e| format!("{e}"))?
            } else if let Some(data) = args.get("data") {
                data.clone()
            } else {
                return Err("Need 'file' or 'data' for import".into());
            };

            let result = session.import_state(&data).await.map_err(|e| format!("{e}"))?;
            Ok(serde_json::json!({
                "ok": true,
                "effect": format!("State imported: {result}"),
            }))
        }
        "health" => {
            let health = session.check_session_health().await.map_err(|e| format!("{e}"))?;
            Ok(health)
        }
        _ => Err(format!("Unknown state op: {op}")),
    }
}

// ─── browser_network handler ───

async fn handle_network(state: &mut McpState, args: &Value) -> Result<Value, String> {
    let op = args["op"].as_str().ok_or("Missing 'op'")?;
    let session = state.ensure_session().await?;
    let session = state.session.as_mut().unwrap();
    let capture_mode = args["capture_mode"].as_str().unwrap_or("js");

    match op {
        "start" => {
            match capture_mode {
                "cdp" => {
                    session.start_cdp_network_capture().await.map_err(|e| format!("{e}"))?;
                    Ok(serde_json::json!({
                        "ok": true,
                        "capture_mode": "cdp",
                        "effect": "CDP network capture started (survives navigation, captures cross-origin iframes)",
                    }))
                }
                "both" => {
                    session.start_cdp_network_capture().await.map_err(|e| format!("{e}"))?;
                    session.start_js_network_capture().await.map_err(|e| format!("{e}"))?;
                    Ok(serde_json::json!({
                        "ok": true,
                        "capture_mode": "both",
                        "effect": "Both CDP and JS network capture started",
                    }))
                }
                _ => {
                    // "js" or default — backward compatible
                    session.start_full_network_capture().await.map_err(|e| format!("{e}"))?;
                    Ok(serde_json::json!({
                        "ok": true,
                        "capture_mode": "js",
                        "effect": "JS network capture started (headers + bodies via monkeypatch)",
                    }))
                }
            }
        }
        "read" => {
            let url_filter = args["url_filter"].as_str();
            match capture_mode {
                "cdp" => {
                    let data = session.read_cdp_network(url_filter).await.map_err(|e| format!("{e}"))?;
                    Ok(serde_json::json!({
                        "ok": true,
                        "capture_mode": "cdp",
                        "requests": data,
                        "count": data.len(),
                    }))
                }
                "both" => {
                    let cdp_data = session.read_cdp_network(url_filter).await.map_err(|e| format!("{e}"))?;
                    let js_data = session.read_js_network().await.map_err(|e| format!("{e}"))?;
                    Ok(serde_json::json!({
                        "ok": true,
                        "capture_mode": "both",
                        "cdp_requests": cdp_data,
                        "cdp_count": cdp_data.len(),
                        "js_requests": js_data,
                        "js_count": js_data.len(),
                    }))
                }
                _ => {
                    let data = session.read_full_network().await.map_err(|e| format!("{e}"))?;
                    Ok(serde_json::json!({
                        "ok": true,
                        "capture_mode": "js",
                        "requests": data,
                        "count": data.len(),
                    }))
                }
            }
        }
        "stop" => {
            match capture_mode {
                "cdp" | "both" => {
                    session.stop_cdp_network_capture().await.map_err(|e| format!("{e}"))?;
                    Ok(serde_json::json!({
                        "ok": true,
                        "effect": "CDP network capture stopped",
                    }))
                }
                _ => {
                    Ok(serde_json::json!({
                        "ok": true,
                        "effect": "JS capture has no explicit stop (wiped on navigation)",
                    }))
                }
            }
        }
        "body" => {
            let request_id = args["request_id"].as_str().ok_or("Missing 'request_id'")?;
            let body = session.get_response_body(request_id).await.map_err(|e| format!("{e}"))?;
            Ok(serde_json::json!({
                "ok": true,
                "requestId": request_id,
                "body": body,
            }))
        }
        "har" => {
            let source = args["source"].as_str();
            let har = session.export_har(source).await.map_err(|e| format!("{e}"))?;
            if let Some(file) = args["file"].as_str() {
                let json = serde_json::to_string_pretty(&har).map_err(|e| format!("{e}"))?;
                std::fs::write(file, json).map_err(|e| format!("{e}"))?;
                return Ok(serde_json::json!({
                    "ok": true,
                    "effect": format!("HAR exported to {file}"),
                }));
            }
            Ok(har)
        }
        "intercept" => {
            let pattern = args["url_pattern"].as_str().ok_or("Missing 'url_pattern'")?;
            let body = args["response_body"].as_str().unwrap_or("{}");
            let status = args["status_code"].as_u64().map(|s| s as u16);
            let content_type = args["content_type"].as_str();
            session.intercept_requests(pattern, body, status, content_type)
                .await.map_err(|e| format!("{e}"))?;
            Ok(serde_json::json!({
                "ok": true,
                "effect": format!("Intercepting requests matching: {pattern}"),
            }))
        }
        "clear_intercepts" => {
            session.clear_intercepts().await.map_err(|e| format!("{e}"))?;
            Ok(serde_json::json!({
                "ok": true,
                "effect": "All intercept rules cleared",
            }))
        }
        _ => Err(format!("Unknown network op: {op}")),
    }
}

// ─── browser_trace handler ───

async fn handle_trace(state: &mut McpState, args: &Value) -> Result<Value, String> {
    let op = args["op"].as_str().ok_or("Missing 'op'")?;

    match op {
        "start" => {
            state.trace.enable();
            Ok(serde_json::json!({"ok": true, "effect": "Tracing enabled"}))
        }
        "stop" => {
            state.trace.disable();
            Ok(serde_json::json!({"ok": true, "effect": "Tracing disabled"}))
        }
        "read" => {
            let last_n = args["last_n"].as_u64().map(|n| n as usize);
            let traces = state.trace.read(last_n);
            let json: Vec<Value> = traces.iter().map(|t| serde_json::to_value(t).unwrap()).collect();
            Ok(serde_json::json!({"traces": json, "count": json.len()}))
        }
        "stats" => {
            let stats = state.trace.stats();
            Ok(serde_json::to_value(stats).unwrap_or(serde_json::json!({})))
        }
        "clear" => {
            state.trace.clear();
            Ok(serde_json::json!({"ok": true, "effect": "Traces cleared"}))
        }
        _ => Err(format!("Unknown trace op: {op}")),
    }
}

// ─── browser_pipeline handler ───

async fn handle_pipeline(state: &mut McpState, args: &Value) -> Result<Value, String> {
    use crate::runner::{Pipeline, PipelineResult, StepResult, OnFail};

    let pipeline = if let Some(json_str) = args["pipeline_json"].as_str() {
        Pipeline::from_json(json_str)?
    } else if let Some(obj) = args.get("pipeline") {
        let json_str = serde_json::to_string(obj).map_err(|e| format!("{e}"))?;
        Pipeline::from_json(&json_str)?
    } else {
        return Err("Need 'pipeline' or 'pipeline_json'".into());
    };

    state.ensure_session().await?;

    let t0 = std::time::Instant::now();
    let mut results = Vec::new();
    let mut variables = pipeline.variables.clone();
    let mut aborted = false;

    for (i, step) in pipeline.steps.iter().enumerate() {
        let step_t0 = std::time::Instant::now();
        let mut retries_used = 0;
        let mut step_outcome = "failed".to_string();
        let mut step_detail = String::new();

        for attempt in 0..=step.max_retries {
            if attempt > 0 {
                retries_used = attempt;
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }

            let session = state.session.as_mut().unwrap();
            let result = match step.action.as_str() {
                "goto" => {
                    let url = substitute_vars(&step.target, &variables);
                    match session.goto(&url).await {
                        Ok(_) => Ok("navigated".to_string()),
                        Err(e) => Err(format!("{e}")),
                    }
                }
                "click" => {
                    let target = substitute_vars(&step.target, &variables);
                    match session.click_reliable(&target).await {
                        Ok((true, strategy)) => Ok(format!("clicked via {strategy}")),
                        Ok((false, _)) => Err("target not found".into()),
                        Err(e) => Err(format!("{e}")),
                    }
                }
                "type" => {
                    let text = substitute_vars(&step.value, &variables);
                    match session.type_text(&text).await {
                        Ok(_) => Ok(format!("typed {} chars", text.len())),
                        Err(e) => Err(format!("{e}")),
                    }
                }
                "press" => {
                    let key = if step.value.is_empty() { "Enter" } else { &step.value };
                    match session.press(key).await {
                        Ok(_) => Ok(format!("pressed {key}")),
                        Err(e) => Err(format!("{e}")),
                    }
                }
                "wait" => {
                    let ms = step.timeout_ms;
                    tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
                    Ok(format!("waited {ms}ms"))
                }
                "eval" => {
                    let js = substitute_vars(&step.value, &variables);
                    match session.eval(&js).await {
                        Ok(result) => {
                            if let Some(var_name) = &step.store_as {
                                variables.insert(var_name.clone(), result.clone());
                            }
                            Ok(result)
                        }
                        Err(e) => Err(format!("{e}")),
                    }
                }
                "screenshot" => {
                    match session.screenshot_base64().await {
                        Ok(b64) => {
                            if let Some(var_name) = &step.store_as {
                                variables.insert(var_name.clone(), format!("{}B", b64.len()));
                            }
                            Ok(format!("screenshot: {}KB", b64.len() / 1024))
                        }
                        Err(e) => Err(format!("{e}")),
                    }
                }
                "extract" => {
                    let js = if step.value.is_empty() {
                        "document.body.innerText.substring(0, 4000)".to_string()
                    } else {
                        substitute_vars(&step.value, &variables)
                    };
                    match session.eval(&js).await {
                        Ok(result) => {
                            if let Some(var_name) = &step.store_as {
                                variables.insert(var_name.clone(), result.clone());
                            }
                            Ok(result)
                        }
                        Err(e) => Err(format!("{e}")),
                    }
                }
                "send_message" => {
                    // target = input CSS selector, value = text to send
                    // Optional: step.assert_text can hold button selector (default: auto-detect)
                    let input_sel = substitute_vars(&step.target, &variables);
                    let text = substitute_vars(&step.value, &variables);
                    let btn_sel = step.assert_text.as_deref().unwrap_or("");
                    match session.send_message(&text, &input_sel, btn_sel).await {
                        Ok(r) if r == "SENT" => Ok(format!("sent {} chars", text.len())),
                        Ok(r) => Err(format!("send_message: {r}")),
                        Err(e) => Err(format!("{e}")),
                    }
                }
                "wait_stable" => {
                    // Wait until page text stops changing (streaming done).
                    // target = optional JS selector for content area
                    // timeout_ms = max wait (default 30000)
                    let js = if step.target.is_empty() {
                        "document.body.innerText".to_string()
                    } else {
                        let sel = substitute_vars(&step.target, &variables);
                        format!("(document.querySelector('{sel}')||document.body).innerText")
                    };
                    let timeout = if step.timeout_ms > 0 { step.timeout_ms } else { 30000 };
                    let mut prev = String::new();
                    let mut stable_count = 0u32;
                    let start = std::time::Instant::now();
                    loop {
                        if start.elapsed().as_millis() as u64 > timeout {
                            // Store whatever we have
                            if let Some(var_name) = &step.store_as {
                                variables.insert(var_name.clone(), prev.clone());
                            }
                            break Err("timeout waiting for stable content".into());
                        }
                        match session.eval(&js).await {
                            Ok(current) => {
                                if current == prev && !current.is_empty() {
                                    stable_count += 1;
                                    if stable_count >= 3 {
                                        if let Some(var_name) = &step.store_as {
                                            variables.insert(var_name.clone(), current.clone());
                                        }
                                        break Ok(format!("stable after {}ms", start.elapsed().as_millis()));
                                    }
                                } else {
                                    stable_count = 0;
                                    prev = current;
                                }
                            }
                            Err(_) => { stable_count = 0; }
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
                    }
                }
                "save" => {
                    // Save variable or page text to file.
                    // target = file path, value = variable name (or empty for page text)
                    let path = substitute_vars(&step.target, &variables);
                    let content = if step.value.is_empty() {
                        session.eval("document.body.innerText").await
                            .unwrap_or_default()
                    } else {
                        let var_name = substitute_vars(&step.value, &variables);
                        variables.get(&var_name).cloned().unwrap_or_default()
                    };
                    match std::fs::write(&path, &content) {
                        Ok(_) => Ok(format!("saved {} bytes to {path}", content.len())),
                        Err(e) => Err(format!("save failed: {e}")),
                    }
                }
                _ => Err(format!("Unknown step action: {}", step.action)),
            };

            match result {
                Ok(detail) => {
                    // Check assertion if present
                    if let Some(ref expected) = step.assert_text {
                        let page = session.see_page().await.unwrap_or_default();
                        if page.contains(expected) {
                            step_outcome = "ok".into();
                            step_detail = detail;
                            break;
                        } else {
                            step_detail = format!("assertion failed: '{}' not found", expected);
                            continue;
                        }
                    }
                    step_outcome = "ok".into();
                    step_detail = detail;
                    break;
                }
                Err(e) => {
                    step_detail = e;
                }
            }
        }

        // Record trace if enabled
        let session = state.session.as_mut().unwrap();
        if state.trace.is_enabled() {
            let url = session.last_url.clone();
            state.trace.record(
                &step.action,
                &step.target,
                &step_outcome,
                &step_detail,
                step_t0.elapsed().as_millis() as u64,
                &url,
                if step_outcome != "ok" { Some(step_detail.clone()) } else { None },
            );
        }

        results.push(StepResult {
            step_index: i,
            action: step.action.clone(),
            outcome: step_outcome.clone(),
            detail: step_detail,
            duration_ms: step_t0.elapsed().as_millis() as u64,
            retries_used,
        });

        if step_outcome != "ok" {
            match step.on_fail {
                OnFail::Abort => { aborted = true; break; }
                OnFail::Skip => continue,
                OnFail::Continue => continue,
            }
        }
    }

    let pr = PipelineResult {
        name: pipeline.name,
        status: if aborted { "aborted".into() } else { "completed".into() },
        steps_completed: results.iter().filter(|r| r.outcome == "ok").count(),
        steps_total: pipeline.steps.len(),
        total_ms: t0.elapsed().as_millis() as u64,
        results,
        variables,
    };

    serde_json::to_value(pr).map_err(|e| format!("{e}"))
}

fn substitute_vars(template: &str, vars: &std::collections::HashMap<String, String>) -> String {
    let mut result = template.to_string();
    for (k, v) in vars {
        result = result.replace(&format!("{{{{{}}}}}", k), v);
    }
    result
}

// ─── browser_learn handler (workflow mapper) ───

async fn handle_learn(state: &mut McpState, args: &Value) -> Result<Value, String> {
    let op = args["op"].as_str().ok_or("Missing 'op'")?;

    match op {
        "start" => {
            let name = args["name"].as_str().unwrap_or("unnamed");
            let session = state.ensure_session().await?;
            let session = state.session.as_mut().unwrap();

            if session.active_workflow.is_some() {
                return Err("A workflow is already being recorded. Use 'stop' or 'save' first.".into());
            }

            session.workflow_start(name);
            Ok(serde_json::json!({
                "ok": true,
                "effect": format!("Started workflow recording: '{name}'"),
                "hint": "Use op='observe' to analyze the page, op='act' to perform actions, op='save' to export the playbook.",
            }))
        }
        "observe" => {
            let _session = state.ensure_session().await?;
            let session = state.session.as_mut().unwrap();

            let result = session.workflow_observe().await.map_err(|e| format!("{e}"))?;
            Ok(result)
        }
        "act" => {
            let action = args["action"].as_str().ok_or("Missing 'action' (click, type, select, navigate, press, scroll, hover, wait)")?;
            let target = args["target"].as_str().unwrap_or("");
            let value = args["value"].as_str().unwrap_or("");
            let notes = args["notes"].as_str().unwrap_or("");

            let _session = state.ensure_session().await?;
            let session = state.session.as_mut().unwrap();

            let result = session.workflow_act(action, target, value, notes).await.map_err(|e| format!("{e}"))?;
            Ok(result)
        }
        "save" => {
            let path = args["path"].as_str().ok_or("Missing 'path' — where to save the playbook JSON")?;
            let session = state.session.as_ref().ok_or("No active session")?;

            let msg = session.workflow_save(path).map_err(|e| format!("{e}"))?;
            Ok(serde_json::json!({
                "ok": true,
                "effect": msg,
            }))
        }
        "stop" => {
            let session = state.session.as_mut().ok_or("No active session")?;
            let summary = session.workflow_stop().map_err(|e| format!("{e}"))?;
            Ok(serde_json::json!({
                "ok": true,
                "summary": summary,
            }))
        }
        "replay" => {
            let path = args["path"].as_str().ok_or("Missing 'path' — path to the playbook JSON")?;
            let _session = state.ensure_session().await?;
            let session = state.session.as_mut().unwrap();

            let result = session.workflow_replay(path).await.map_err(|e| format!("{e}"))?;
            Ok(result)
        }
        _ => Err(format!("Unknown learn op: {op}. Use: start, observe, act, save, stop, replay")),
    }
}

// ─── browser_pool handler ───

async fn handle_pool(state: &mut McpState, args: &Value) -> Result<Value, String> {
    let op = args["op"].as_str().ok_or("Missing 'op'")?;

    match op {
        "create" => {
            let id = args["id"].as_str().map(|s| s.to_string());
            let ctx_id = state.pool.create_context(id)?;
            Ok(serde_json::json!({
                "ok": true,
                "context_id": ctx_id,
                "effect": format!("Created isolated context: {ctx_id}"),
            }))
        }
        "list" => {
            let contexts = state.pool.list();
            let json: Vec<Value> = contexts.iter().map(|c| serde_json::to_value(c).unwrap()).collect();
            Ok(serde_json::json!({
                "contexts": json,
                "count": json.len(),
            }))
        }
        "destroy" => {
            let id = args["id"].as_str().ok_or("Missing 'id'")?;
            state.pool.destroy(id)?;
            Ok(serde_json::json!({
                "ok": true,
                "effect": format!("Destroyed context: {id}"),
            }))
        }
        "destroy_all" => {
            state.pool.destroy_all();
            Ok(serde_json::json!({
                "ok": true,
                "effect": "All contexts destroyed",
            }))
        }
        _ => Err(format!("Unknown pool op: {op}")),
    }
}

// ─── MCP Server loop ───

pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = McpState::new();
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    eprintln!("[MCP] NeoBrowser MCP server started");

    for line_result in stdin.lock().lines() {
        let line = match line_result {
            Ok(l) => l,
            Err(_) => break,
        };

        if line.trim().is_empty() {
            continue;
        }

        let request: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let err_response = JsonRpcResponse {
                    jsonrpc: "2.0".into(),
                    id: Value::Null,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32700,
                        message: format!("Parse error: {e}"),
                    }),
                };
                let out = serde_json::to_string(&err_response)?;
                writeln!(stdout, "{out}")?;
                stdout.flush()?;
                continue;
            }
        };

        let response = match request.method.as_str() {
            "initialize" => {
                let init = McpInitResult {
                    protocol_version: "2024-11-05".into(),
                    capabilities: McpCapabilities {
                        tools: ToolsCapability {},
                    },
                    server_info: ServerInfo {
                        name: "neobrowser".into(),
                        version: "0.3.0".into(),
                    },
                };
                JsonRpcResponse {
                    jsonrpc: "2.0".into(),
                    id: request.id,
                    result: Some(serde_json::to_value(init)?),
                    error: None,
                }
            }
            "notifications/initialized" => {
                // Acknowledgment, no response needed
                continue;
            }
            "tools/list" => {
                let tools = tool_definitions();
                JsonRpcResponse {
                    jsonrpc: "2.0".into(),
                    id: request.id,
                    result: Some(serde_json::json!({ "tools": tools })),
                    error: None,
                }
            }
            "tools/call" => {
                let tool_name = request.params["name"].as_str().unwrap_or("");
                let tool_args = &request.params["arguments"];

                match handle_tool(&mut state, tool_name, tool_args).await {
                    Ok(result) => {
                        let text = serde_json::to_string(&result).unwrap_or_default();
                        let tool_result = ToolResult {
                            content: vec![ToolContent {
                                content_type: "text".into(),
                                text,
                            }],
                            is_error: None,
                        };
                        JsonRpcResponse {
                            jsonrpc: "2.0".into(),
                            id: request.id,
                            result: Some(serde_json::to_value(tool_result)?),
                            error: None,
                        }
                    }
                    Err(e) => {
                        let tool_result = ToolResult {
                            content: vec![ToolContent {
                                content_type: "text".into(),
                                text: format!("Error: {e}"),
                            }],
                            is_error: Some(true),
                        };
                        JsonRpcResponse {
                            jsonrpc: "2.0".into(),
                            id: request.id,
                            result: Some(serde_json::to_value(tool_result)?),
                            error: None,
                        }
                    }
                }
            }
            _ => {
                JsonRpcResponse {
                    jsonrpc: "2.0".into(),
                    id: request.id,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32601,
                        message: format!("Method not found: {}", request.method),
                    }),
                }
            }
        };

        let out = serde_json::to_string(&response)?;
        writeln!(stdout, "{out}")?;
        stdout.flush()?;
    }

    // Cleanup
    if let Some(session) = state.session.take() {
        session.close().await.ok();
    }

    eprintln!("[MCP] Server stopped");
    Ok(())
}

// ─── NeoRender helpers ───

fn extract_title_from_dom(dom: &markup5ever_rcdom::RcDom) -> String {
    fn find(node: &markup5ever_rcdom::Handle) -> Option<String> {
        if let markup5ever_rcdom::NodeData::Element { name, .. } = &node.data {
            if name.local.as_ref() == "title" {
                let text: String = node.children.borrow().iter()
                    .filter_map(|c| match &c.data {
                        markup5ever_rcdom::NodeData::Text { contents } => Some(contents.borrow().to_string()),
                        _ => None,
                    }).collect();
                if !text.trim().is_empty() { return Some(text.trim().to_string()); }
            }
        }
        for child in node.children.borrow().iter() {
            if let Some(t) = find(child) { return Some(t); }
        }
        None
    }
    find(&dom.document).unwrap_or_default()
}

fn extract_text_from_dom(dom: &markup5ever_rcdom::RcDom) -> String {
    fn collect(node: &markup5ever_rcdom::Handle, depth: usize) -> String {
        if depth > 200 { return String::new(); }
        let mut text = String::new();
        if let markup5ever_rcdom::NodeData::Element { name, .. } = &node.data {
            let tag = name.local.as_ref();
            if matches!(tag, "script" | "style" | "noscript" | "svg" | "head") { return String::new(); }
        }
        if let markup5ever_rcdom::NodeData::Text { contents } = &node.data {
            let t = contents.borrow().to_string();
            let t = t.trim();
            if !t.is_empty() { text.push_str(t); text.push(' '); }
        }
        for child in node.children.borrow().iter() {
            text.push_str(&collect(child, depth + 1));
        }
        text
    }
    collect(&dom.document, 0)
}

fn count_elements(dom: &markup5ever_rcdom::RcDom, tag_name: &str) -> usize {
    fn count(node: &markup5ever_rcdom::Handle, tag: &str) -> usize {
        let mut n = 0;
        if let markup5ever_rcdom::NodeData::Element { name, .. } = &node.data {
            if name.local.as_ref() == tag { n += 1; }
        }
        for child in node.children.borrow().iter() { n += count(child, tag); }
        n
    }
    count(&dom.document, tag_name)
}
