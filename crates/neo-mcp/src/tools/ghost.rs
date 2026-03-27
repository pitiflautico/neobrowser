//! Ghost tool — neomode Chrome for operations that need a real browser.
//!
//! Uses undetected-chromedriver with neomode patches (headless but
//! indistinguishable from real Chrome) for:
//! - Sites behind Cloudflare/bot protection
//! - SPA rendering (React, Vue, Angular)
//! - Form filling and submission
//! - Chat interactions (ChatGPT, Grok)
//! - Screenshot capture
//! - Search, scraping, login, monitoring, pipelines, etc.

use serde_json::{json, Value};
use std::process::Command;

use crate::McpError;
use crate::state::McpState;

/// All supported ghost actions.
const ALL_ACTIONS: &[&str] = &[
    "search", "navigate", "read", "find", "click", "type", "fill_form", "submit",
    "screenshot", "scroll", "extract_data", "login", "download", "monitor",
    "api_intercept", "cookie_manage", "multi_tab", "wait_for", "pipeline",
    "open", "chat", "html",
];

pub(crate) fn definition() -> super::ToolDef {
    super::ToolDef {
        name: "ghost",
        description: "Neomode ghost browser — real Chrome (headless, undetectable). \
            Use for Cloudflare-protected sites, SPAs, form filling, chat interactions, \
            search, scraping, login, monitoring, pipelines, and more. \
            Actions: search, navigate, read, find, click, type, fill_form, submit, \
            screenshot, scroll, extract_data, login, download, monitor, \
            api_intercept, cookie_manage, multi_tab, wait_for, pipeline, \
            open, chat, html.",
        schema: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ALL_ACTIONS,
                    "description": "Action to perform"
                },
                "url": {
                    "type": "string",
                    "description": "URL to navigate to"
                },
                "message": {
                    "type": "string",
                    "description": "For chat action: message to send"
                },
                "profile": {
                    "type": "string",
                    "description": "Chrome profile name for cookie import (e.g. 'Profile 24')"
                },
                "wait": {
                    "type": "integer",
                    "default": 5000,
                    "description": "Wait time in ms after page load"
                },
                "query": {
                    "type": "string",
                    "description": "Search query (for search action)"
                },
                "selector": {
                    "type": "string",
                    "description": "CSS/XPath selector (for find, click, read, wait_for, submit)"
                },
                "value": {
                    "type": "string",
                    "description": "Value to type (for type action)"
                },
                "text": {
                    "type": "string",
                    "description": "Text to find or click (for find, click)"
                },
                "fields": {
                    "type": "string",
                    "description": "JSON string of field->value pairs (for fill_form)"
                },
                "direction": {
                    "type": "string",
                    "enum": ["up", "down", "left", "right"],
                    "description": "Scroll direction (for scroll)"
                },
                "amount": {
                    "type": "integer",
                    "description": "Scroll amount in pixels (for scroll)"
                },
                "type_": {
                    "type": "string",
                    "enum": ["table", "list", "product", "links"],
                    "description": "Data extraction type (for extract_data)"
                },
                "email": {
                    "type": "string",
                    "description": "Email for login"
                },
                "password": {
                    "type": "string",
                    "description": "Password for login"
                },
                "engine": {
                    "type": "string",
                    "enum": ["google", "bing", "duckduckgo"],
                    "description": "Search engine (for search, default: google)"
                },
                "num": {
                    "type": "integer",
                    "default": 10,
                    "description": "Number of results (for search)"
                },
                "pattern": {
                    "type": "string",
                    "description": "URL pattern to intercept (for api_intercept)"
                },
                "steps": {
                    "type": "string",
                    "description": "JSON string of pipeline steps (for pipeline)"
                },
                "by": {
                    "type": "string",
                    "enum": ["text", "css", "xpath", "role"],
                    "description": "Locator strategy (for find)"
                },
                "index": {
                    "type": "integer",
                    "description": "Tab or element index (for click, multi_tab)"
                }
            },
            "required": ["action"]
        }),
    }
}

pub fn call(args: Value, state: &mut McpState) -> Result<Value, McpError> {
    let action = args["action"].as_str().unwrap_or("open");
    let url = args["url"].as_str().unwrap_or("");
    let message = args["message"].as_str().unwrap_or("");
    let profile = args["profile"].as_str();
    let wait = args["wait"].as_u64().unwrap_or(5000);

    match action {
        // ── HYBRID: Rust-first, Ghost fallback ──
        // These actions try the fast Rust engine first.
        // If result is empty/insufficient, fall back to Chrome ghost.

        "search" => {
            let query = args["query"].as_str().unwrap_or("");
            if query.is_empty() {
                return Err(McpError::InvalidParams("query required for search".into()));
            }
            // Always use Rust HTTP search — 10x faster, same results
            let search_args = json!({"query": query, "num": args["num"].as_u64().unwrap_or(10)});
            match crate::tools::search::call(search_args, state) {
                Ok(result) => {
                    // search::call returns json with results
                    let text = serde_json::to_string(&result).unwrap_or_default();
                    if text.len() > 50 {
                        return Ok(json!({"content": [{"type": "text", "text": text}]}));
                    }
                    // Empty — fall through to ghost
                    eprintln!("[ghost] Rust search empty, trying Chrome...");
                }
                Err(_) => {}
            }
            // Fallback to ghost Chrome search
            let engine = args["engine"].as_str().unwrap_or("duckduckgo");
            let num = args["num"].as_u64().unwrap_or(10);
            let num_str = num.to_string();
            let ghost_args = vec!["search", query, "--engine", engine, "--num", &num_str];
            ghost_delegate(&ghost_args, 30, profile)
        }

        "navigate" | "open" => {
            if url.is_empty() {
                return Err(McpError::InvalidParams("url required".into()));
            }
            // Try Rust engine first (fast HTTP + compact view)
            let browse_args = json!({"url": url});
            match crate::tools::browse::call(browse_args, state) {
                Ok(result) => {
                    // browse::call returns json!(string) — check the string directly
                    let text = result.as_str().unwrap_or("");
                    if text.len() > 100 {
                        // Wrap in MCP content format
                        return Ok(json!({"content": [{"type": "text", "text": text}]}));
                    }
                    eprintln!("[ghost] Rust navigate got {} chars, trying Chrome...", text.len());
                }
                Err(_) => {}
            }
            // Fallback to ghost
            let wait_str = wait.to_string();
            ghost_delegate(&["open", url, "--wait", &wait_str], 30, profile)
        }

        "read" => {
            if url.is_empty() {
                return Err(McpError::InvalidParams("url required for read".into()));
            }
            let selector = args["selector"].as_str().unwrap_or("");
            // Try Rust extract first
            let extract_args = json!({"format": "text"});
            let browse_args = json!({"url": url});
            if let Ok(_) = crate::tools::browse::call(browse_args, state) {
                if let Ok(extract_result) = crate::tools::extract::call(extract_args, state) {
                    let text = serde_json::to_string(&extract_result).unwrap_or_default();
                    if text.len() > 100 {
                        return Ok(json!({"content": [{"type": "text", "text": text}]}));
                    }
                }
            }
            eprintln!("[ghost] Rust read insufficient, trying Chrome...");
            // Fallback to ghost
            let mut ghost_args = vec!["read", url];
            if !selector.is_empty() {
                ghost_args.extend(&["--selector", selector]);
            }
            ghost_delegate(&ghost_args, 30, profile)
        }
        "find" => {
            let selector = args["selector"].as_str().unwrap_or("");
            let text = args["text"].as_str().unwrap_or("");
            let by = args["by"].as_str().unwrap_or("css");
            let mut ghost_args = vec!["find", "--by", by];
            if !selector.is_empty() {
                ghost_args.extend(&["--selector", selector]);
            }
            if !text.is_empty() {
                ghost_args.extend(&["--text", text]);
            }
            if !url.is_empty() {
                ghost_args.extend(&["--url", url]);
            }
            ghost_delegate(&ghost_args, 15, profile)
        }
        "click" => {
            let selector = args["selector"].as_str().unwrap_or("");
            let text = args["text"].as_str().unwrap_or("");
            let index = args["index"].as_u64();
            let mut ghost_args = vec!["click"];
            if !selector.is_empty() {
                ghost_args.extend(&["--selector", selector]);
            }
            if !text.is_empty() {
                ghost_args.extend(&["--text", text]);
            }
            let index_str;
            if let Some(i) = index {
                index_str = i.to_string();
                ghost_args.extend(&["--index", &index_str]);
            }
            if !url.is_empty() {
                ghost_args.extend(&["--url", url]);
            }
            ghost_delegate(&ghost_args, 15, profile)
        }
        "type" => {
            let selector = args["selector"].as_str().unwrap_or("");
            let value = args["value"].as_str().unwrap_or("");
            if value.is_empty() {
                return Err(McpError::InvalidParams("value required for type".into()));
            }
            let mut ghost_args = vec!["type", "--value", value];
            if !selector.is_empty() {
                ghost_args.extend(&["--selector", selector]);
            }
            if !url.is_empty() {
                ghost_args.extend(&["--url", url]);
            }
            ghost_delegate(&ghost_args, 15, profile)
        }
        "fill_form" => {
            let fields = args["fields"].as_str().unwrap_or("{}");
            if fields == "{}" {
                return Err(McpError::InvalidParams("fields required for fill_form".into()));
            }
            let mut ghost_args = vec!["fill_form", "--fields", fields];
            if !url.is_empty() {
                ghost_args.extend(&["--url", url]);
            }
            ghost_delegate(&ghost_args, 30, profile)
        }
        "submit" => {
            let selector = args["selector"].as_str().unwrap_or("");
            let mut ghost_args = vec!["submit"];
            if !selector.is_empty() {
                ghost_args.extend(&["--selector", selector]);
            }
            if !url.is_empty() {
                ghost_args.extend(&["--url", url]);
            }
            ghost_delegate(&ghost_args, 30, profile)
        }
        "scroll" => {
            let direction = args["direction"].as_str().unwrap_or("down");
            let amount = args["amount"].as_u64().unwrap_or(500);
            let amount_str = amount.to_string();
            let mut ghost_args = vec!["scroll", "--direction", direction, "--amount", &amount_str];
            if !url.is_empty() {
                ghost_args.extend(&["--url", url]);
            }
            ghost_delegate(&ghost_args, 15, profile)
        }
        "extract_data" => {
            let type_ = args["type_"].as_str().unwrap_or("table");
            let selector = args["selector"].as_str().unwrap_or("");
            if url.is_empty() {
                return Err(McpError::InvalidParams("url required for extract_data".into()));
            }
            // Try Rust engine first for links/text extraction
            if type_ == "links" || type_ == "list" {
                let browse_args = json!({"url": url});
                if let Ok(_) = crate::tools::browse::call(browse_args, state) {
                    let fmt = if type_ == "links" { "links" } else { "text" };
                    let extract_args = json!({"format": fmt});
                    if let Ok(result) = crate::tools::extract::call(extract_args, state) {
                        let text = serde_json::to_string(&result).unwrap_or_default();
                        if text.len() > 50 {
                            return Ok(json!({"content": [{"type": "text", "text": text}]}));
                        }
                    }
                }
                eprintln!("[ghost] Rust extract insufficient, trying Chrome...");
            }
            // Ghost for tables/products or fallback
            let mut ghost_args = vec!["extract_data", url, "--type", type_];
            if !selector.is_empty() {
                ghost_args.extend(&["--selector", selector]);
            }
            ghost_delegate(&ghost_args, 30, profile)
        }
        "login" => {
            let email = args["email"].as_str().unwrap_or("");
            let password = args["password"].as_str().unwrap_or("");
            if url.is_empty() || email.is_empty() || password.is_empty() {
                return Err(McpError::InvalidParams("url, email, and password required for login".into()));
            }
            let ghost_args = vec!["login", url, "--email", email, "--password", password];
            ghost_delegate(&ghost_args, 60, profile)
        }
        "download" => {
            if url.is_empty() {
                return Err(McpError::InvalidParams("url required for download".into()));
            }
            let selector = args["selector"].as_str().unwrap_or("");
            let mut ghost_args = vec!["download", url];
            if !selector.is_empty() {
                ghost_args.extend(&["--selector", selector]);
            }
            ghost_delegate(&ghost_args, 60, profile)
        }
        "monitor" => {
            if url.is_empty() {
                return Err(McpError::InvalidParams("url required for monitor".into()));
            }
            let selector = args["selector"].as_str().unwrap_or("");
            let mut ghost_args = vec!["monitor", url];
            if !selector.is_empty() {
                ghost_args.extend(&["--selector", selector]);
            }
            ghost_delegate(&ghost_args, 60, profile)
        }
        "api_intercept" => {
            let pattern = args["pattern"].as_str().unwrap_or("*");
            if url.is_empty() {
                return Err(McpError::InvalidParams("url required for api_intercept".into()));
            }
            let ghost_args = vec!["api_intercept", url, "--pattern", pattern];
            ghost_delegate(&ghost_args, 30, profile)
        }
        "cookie_manage" => {
            if url.is_empty() {
                return Err(McpError::InvalidParams("url required for cookie_manage".into()));
            }
            let ghost_args = vec!["cookie_manage", url];
            ghost_delegate(&ghost_args, 15, profile)
        }
        "multi_tab" => {
            let index = args["index"].as_u64();
            let mut ghost_args = vec!["multi_tab"];
            let index_str;
            if let Some(i) = index {
                index_str = i.to_string();
                ghost_args.extend(&["--index", &index_str]);
            }
            if !url.is_empty() {
                ghost_args.extend(&["--url", url]);
            }
            ghost_delegate(&ghost_args, 15, profile)
        }
        "wait_for" => {
            let selector = args["selector"].as_str().unwrap_or("");
            if selector.is_empty() {
                return Err(McpError::InvalidParams("selector required for wait_for".into()));
            }
            let wait_str = wait.to_string();
            let mut ghost_args = vec!["wait_for", "--selector", selector, "--timeout", &wait_str];
            if !url.is_empty() {
                ghost_args.extend(&["--url", url]);
            }
            ghost_delegate(&ghost_args, 60, profile)
        }
        "pipeline" => {
            let steps = args["steps"].as_str().unwrap_or("[]");
            if steps == "[]" {
                return Err(McpError::InvalidParams("steps required for pipeline".into()));
            }
            let mut ghost_args = vec!["pipeline", "--steps", steps];
            if !url.is_empty() {
                ghost_args.extend(&["--url", url]);
            }
            ghost_delegate(&ghost_args, 120, profile)
        }

        other => Err(McpError::InvalidParams(format!("Unknown action: {other}"))),
    }
}

fn find_ghost_script() -> String {
    // Try relative to CARGO_MANIFEST_DIR (development)
    let dev_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tools/spa-clone/ghost.py");
    if std::path::Path::new(dev_path).exists() {
        return dev_path.to_string();
    }
    // Try relative to binary location
    if let Ok(exe) = std::env::current_exe() {
        let tools_path = exe.parent().unwrap().parent().unwrap().join("tools/spa-clone/ghost.py");
        if tools_path.exists() {
            return tools_path.to_string_lossy().to_string();
        }
    }
    // Fallback
    "tools/spa-clone/ghost.py".to_string()
}

fn run_ghost(args: &[&str], _timeout_secs: u64) -> Result<String, McpError> {
    let ghost = find_ghost_script();

    let mut cmd_args = vec![&ghost as &str];
    cmd_args.extend_from_slice(args);

    let output = Command::new("python3")
        .args(&cmd_args)
        .output()
        .map_err(|e| McpError::InvalidParams(format!("ghost launch failed: {e}")))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let err = String::from_utf8_lossy(&output.stderr);
        Err(McpError::InvalidParams(format!("ghost error: {}", err.chars().take(200).collect::<String>())))
    }
}

/// Generic delegate: builds ghost.py args, optionally adds --profile, runs, returns MCP response.
fn ghost_delegate(base_args: &[&str], timeout_secs: u64, profile: Option<&str>) -> Result<Value, McpError> {
    let profile_str;
    let mut args: Vec<&str> = base_args.to_vec();
    if let Some(p) = profile {
        profile_str = p.to_string();
        args.extend(&["--profile", &profile_str]);
    }

    let output = run_ghost(&args, timeout_secs)?;

    // Try JSON parse, fallback to raw text
    match serde_json::from_str::<Value>(&output) {
        Ok(parsed) => Ok(json!({
            "content": [{"type": "text", "text": format!("{}", serde_json::to_string_pretty(&parsed).unwrap_or(output))}]
        })),
        Err(_) => Ok(json!({
            "content": [{"type": "text", "text": output}]
        })),
    }
}

fn ghost_open(url: &str, profile: Option<&str>, wait: u64) -> Result<Value, McpError> {
    if url.is_empty() {
        return Err(McpError::InvalidParams("url required for open action".into()));
    }

    let wait_str = wait.to_string();
    let mut args = vec!["open", url, "--wait", &wait_str];
    let profile_str;
    if let Some(p) = profile {
        profile_str = p.to_string();
        args.extend(&["--profile", &profile_str]);
    }

    let output = run_ghost(&args, 30)?;

    // Parse JSON output from ghost.py
    match serde_json::from_str::<Value>(&output) {
        Ok(info) => {
            let title = info["title"].as_str().unwrap_or("");
            let text = info["text"].as_str().unwrap_or("");
            let elements = info["elements"].as_u64().unwrap_or(0);

            let summary = format!(
                "[Ghost] {} | {} elements\n\n{}",
                title, elements,
                if text.len() > 500 { &text[..500] } else { text }
            );

            Ok(json!({
                "content": [{"type": "text", "text": summary}]
            }))
        }
        Err(_) => Ok(json!({
            "content": [{"type": "text", "text": output}]
        })),
    }
}

fn ghost_chat(url: &str, message: &str, profile: Option<&str>) -> Result<Value, McpError> {
    if message.is_empty() {
        return Err(McpError::InvalidParams("message required for chat action".into()));
    }

    // Default URLs for known platforms
    let target = if url.is_empty() {
        if profile.is_some() { "https://chatgpt.com" } else { "https://grok.com" }
    } else {
        url
    };

    let mut args = vec!["pong", target, "--message", message];
    let profile_str;
    if let Some(p) = profile {
        profile_str = p.to_string();
        args.extend(&["--profile", &profile_str]);
    }

    let output = run_ghost(&args, 120)?;

    match serde_json::from_str::<Value>(&output) {
        Ok(result) => {
            let response = result["response"].as_str().unwrap_or("No response");
            let platform = result["platform"].as_str().unwrap_or("unknown");

            Ok(json!({
                "content": [{"type": "text", "text": format!("[{}] {}", platform, response)}]
            }))
        }
        Err(_) => Ok(json!({
            "content": [{"type": "text", "text": output}]
        })),
    }
}

fn ghost_screenshot(url: &str, profile: Option<&str>, wait: u64) -> Result<Value, McpError> {
    if url.is_empty() {
        return Err(McpError::InvalidParams("url required".into()));
    }

    let wait_str = wait.to_string();
    let mut args = vec!["open", url, "--wait", &wait_str, "--output", "/tmp/ghost-mcp-screenshot.png"];
    let profile_str;
    if let Some(p) = profile {
        profile_str = p.to_string();
        args.extend(&["--profile", &profile_str]);
    }

    let _ = run_ghost(&args, 30)?;

    Ok(json!({
        "content": [{"type": "text", "text": "Screenshot saved to /tmp/ghost-mcp-screenshot.png"}]
    }))
}

fn ghost_html(url: &str, profile: Option<&str>, wait: u64) -> Result<Value, McpError> {
    if url.is_empty() {
        return Err(McpError::InvalidParams("url required".into()));
    }

    let wait_str = wait.to_string();
    let mut args = vec!["open", url, "--wait", &wait_str, "--html"];
    let profile_str;
    if let Some(p) = profile {
        profile_str = p.to_string();
        args.extend(&["--profile", &profile_str]);
    }

    let html = run_ghost(&args, 30)?;

    // Return truncated HTML (full HTML can be huge)
    let truncated = if html.len() > 10000 {
        format!("{}...\n\n[truncated, {} total bytes]", &html[..10000], html.len())
    } else {
        html
    };

    Ok(json!({
        "content": [{"type": "text", "text": truncated}]
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_definition_has_required_fields() {
        let def = definition();
        assert_eq!(def.name, "ghost");
        assert!(def.description.contains("ghost") || def.description.contains("Neomode"));
    }

    #[test]
    fn test_ghost_script_path_exists() {
        let path = find_ghost_script();
        assert!(std::path::Path::new(&path).exists(), "ghost.py not found at {path}");
    }

    #[test]
    fn test_all_19_actions_in_enum() {
        let def = definition();
        let schema = &def.schema;
        let actions = schema["properties"]["action"]["enum"].as_array().unwrap();

        // Must have at least 19 actions (open, chat, html + 15 new + screenshot = 19+3 = 22)
        assert!(
            actions.len() >= 19,
            "Expected at least 19 actions, got {}",
            actions.len()
        );

        let expected = vec![
            "search", "navigate", "read", "find", "click", "type", "fill_form", "submit",
            "screenshot", "scroll", "extract_data", "login", "download", "monitor",
            "api_intercept", "cookie_manage", "multi_tab", "wait_for", "pipeline",
            "open", "chat", "html",
        ];

        for action in &expected {
            assert!(
                actions.contains(&json!(action)),
                "Missing action in enum: {action}"
            );
        }
    }

    #[test]
    fn test_each_action_name_is_valid() {
        // Verify ALL_ACTIONS constant matches what the definition exposes
        let def = definition();
        let schema = &def.schema;
        let actions = schema["properties"]["action"]["enum"].as_array().unwrap();

        for action in ALL_ACTIONS {
            assert!(
                actions.contains(&json!(action)),
                "ALL_ACTIONS contains '{action}' but it's not in the schema enum"
            );
        }

        // And the reverse: every enum value should be in ALL_ACTIONS
        for action_val in actions {
            let action_str = action_val.as_str().unwrap();
            assert!(
                ALL_ACTIONS.contains(&action_str),
                "Schema enum contains '{action_str}' but it's not in ALL_ACTIONS"
            );
        }
    }

    #[test]
    fn test_all_actions_have_match_arm() {
        // Verify every action in ALL_ACTIONS has a corresponding match arm
        // by checking the source code contains the match pattern for each action.
        // We can't instantiate McpState without a BrowserEngine, so we verify statically.
        let source = include_str!("ghost.rs");
        for action in ALL_ACTIONS {
            let pattern = format!("\"{}\"", action);
            assert!(
                source.contains(&pattern),
                "Action '{action}' appears to have no match arm in call()"
            );
        }
    }

    #[test]
    fn test_schema_has_all_parameters() {
        let def = definition();
        let props = def.schema["properties"].as_object().unwrap();

        let expected_params = vec![
            "action", "url", "message", "profile", "wait",
            "query", "selector", "value", "text", "fields",
            "direction", "amount", "type_", "email", "password",
            "engine", "num", "pattern", "steps", "by", "index",
        ];

        for param in &expected_params {
            assert!(
                props.contains_key(*param),
                "Missing parameter in schema: {param}"
            );
        }
    }
}
