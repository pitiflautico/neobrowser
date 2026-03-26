//! Ghost tool — neomode Chrome for operations that need a real browser.
//!
//! Uses undetected-chromedriver with neomode patches (headless but
//! indistinguishable from real Chrome) for:
//! - Sites behind Cloudflare/bot protection
//! - SPA rendering (React, Vue, Angular)
//! - Form filling and submission
//! - Chat interactions (ChatGPT, Grok)
//! - Screenshot capture

use serde_json::{json, Value};
use std::process::Command;

use crate::McpError;
use crate::state::McpState;

pub(crate) fn definition() -> super::ToolDef {
    super::ToolDef {
        name: "ghost",
        description: "Neomode ghost browser — real Chrome (headless, undetectable). \
            Use for Cloudflare-protected sites, SPAs, form filling, chat interactions. \
            Actions: 'open' (navigate+extract), 'chat' (send message to ChatGPT/Grok), \
            'screenshot', 'html' (get rendered HTML).",
        schema: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["open", "chat", "screenshot", "html"],
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
                }
            },
            "required": ["action"]
        }),
    }
}

pub fn call(args: Value, _state: &mut McpState) -> Result<Value, McpError> {
    let action = args["action"].as_str().unwrap_or("open");
    let url = args["url"].as_str().unwrap_or("");
    let message = args["message"].as_str().unwrap_or("");
    let profile = args["profile"].as_str();
    let wait = args["wait"].as_u64().unwrap_or(5000);

    // Find ghost.py relative to the binary
    let ghost_script = find_ghost_script();

    match action {
        "open" => ghost_open(&ghost_script, url, profile, wait),
        "chat" => ghost_chat(&ghost_script, url, message, profile),
        "screenshot" => ghost_screenshot(&ghost_script, url, profile, wait),
        "html" => ghost_html(&ghost_script, url, profile, wait),
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

fn run_ghost(args: &[&str], timeout_secs: u64) -> Result<String, McpError> {
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

fn ghost_open(script: &str, url: &str, profile: Option<&str>, wait: u64) -> Result<Value, McpError> {
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

fn ghost_chat(script: &str, url: &str, message: &str, profile: Option<&str>) -> Result<Value, McpError> {
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

fn ghost_screenshot(script: &str, url: &str, profile: Option<&str>, wait: u64) -> Result<Value, McpError> {
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

fn ghost_html(script: &str, url: &str, profile: Option<&str>, wait: u64) -> Result<Value, McpError> {
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
        assert!(def.description.contains("neomode"));
    }

    #[test]
    fn test_ghost_script_path_exists() {
        let path = find_ghost_script();
        assert!(std::path::Path::new(&path).exists(), "ghost.py not found at {path}");
    }

    #[test]
    fn test_open_requires_url() {
        let mut state = crate::state::McpState::new_test();
        let result = call(json!({"action": "open"}), &mut state);
        assert!(result.is_err());
    }

    #[test]
    fn test_chat_requires_message() {
        let mut state = crate::state::McpState::new_test();
        let result = call(json!({"action": "chat", "url": "https://grok.com"}), &mut state);
        assert!(result.is_err());
    }
}
