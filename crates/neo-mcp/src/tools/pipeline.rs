//! `pipeline` tool — multi-step automation in a single tool call.
//!
//! Executes a sequence of actions atomically, returning results for each step.
//! Stops on first error unless the step has `continue_on_error: true`.

use std::collections::HashMap;
use std::time::Instant;

use serde_json::Value;

use crate::state::McpState;
use crate::McpError;

use super::ToolDef;

/// Tool definition for `tools/list`.
pub(crate) fn definition() -> ToolDef {
    ToolDef {
        name: "pipeline",
        description: "Execute a sequence of browser actions atomically. \
                       Actions: browse, navigate, click, type, fill_form, submit, \
                       press_key, wait, extract, eval, find, analyze_forms, cookie_consent. \
                       Stops on first error unless step has continue_on_error=true.",
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "steps": {
                    "type": "array",
                    "description": "Ordered list of actions to execute",
                    "items": {
                        "type": "object",
                        "properties": {
                            "action": {
                                "type": "string",
                                "enum": [
                                    "browse", "navigate", "click", "type",
                                    "fill_form", "submit", "press_key", "wait",
                                    "extract", "eval", "find", "analyze_forms",
                                    "cookie_consent"
                                ],
                                "description": "Action to perform"
                            },
                            "url": {
                                "type": "string",
                                "description": "URL (for browse/navigate)"
                            },
                            "target": {
                                "type": "string",
                                "description": "Element target (CSS selector, text, aria-label)"
                            },
                            "text": {
                                "type": "string",
                                "description": "Text to type or wait for"
                            },
                            "fields": {
                                "type": "object",
                                "description": "Field name→value map (for fill_form)",
                                "additionalProperties": { "type": "string" }
                            },
                            "key": {
                                "type": "string",
                                "description": "Key to press (Enter, Tab, Escape, etc.)"
                            },
                            "kind": {
                                "type": "string",
                                "description": "Extraction kind (for extract): wom, text, links, semantic"
                            },
                            "code": {
                                "type": "string",
                                "description": "JavaScript code (for eval)"
                            },
                            "selector": {
                                "type": "string",
                                "description": "CSS selector (for wait)"
                            },
                            "timeout_ms": {
                                "type": "integer",
                                "description": "Timeout in ms (for wait)",
                                "default": 5000
                            },
                            "continue_on_error": {
                                "type": "boolean",
                                "description": "Continue pipeline even if this step fails",
                                "default": false
                            }
                        },
                        "required": ["action"]
                    }
                }
            },
            "required": ["steps"]
        }),
    }
}

/// Execute the `pipeline` tool.
pub fn call(args: Value, state: &mut McpState) -> Result<Value, McpError> {
    let steps = args
        .get("steps")
        .and_then(|v| v.as_array())
        .ok_or_else(|| McpError::InvalidParams("missing 'steps' array".into()))?;

    if steps.is_empty() {
        return Err(McpError::InvalidParams("'steps' array is empty".into()));
    }

    let pipeline_start = Instant::now();
    let mut results: Vec<Value> = Vec::with_capacity(steps.len());
    let mut steps_completed: usize = 0;
    let mut had_error = false;

    for (i, step) in steps.iter().enumerate() {
        let action = step
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                McpError::InvalidParams(format!("step {i}: missing 'action'"))
            })?;

        let continue_on_error = step
            .get("continue_on_error")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let step_start = Instant::now();
        let step_result = execute_step(action, step, state);
        let step_ms = step_start.elapsed().as_millis() as u64;

        match step_result {
            Ok(result) => {
                results.push(serde_json::json!({
                    "step": i,
                    "action": action,
                    "ok": true,
                    "result": result,
                    "ms": step_ms,
                }));
                steps_completed += 1;
            }
            Err(e) => {
                let error_entry = serde_json::json!({
                    "step": i,
                    "action": action,
                    "ok": false,
                    "error": e.to_string(),
                    "ms": step_ms,
                });
                results.push(error_entry);
                had_error = true;

                if !continue_on_error {
                    break;
                }
                steps_completed += 1;
            }
        }
    }

    let total_ms = pipeline_start.elapsed().as_millis() as u64;

    Ok(serde_json::json!({
        "ok": !had_error,
        "steps_completed": steps_completed,
        "steps_total": steps.len(),
        "results": results,
        "total_ms": total_ms,
    }))
}

/// Dispatch a single pipeline step to the appropriate handler.
fn execute_step(action: &str, step: &Value, state: &mut McpState) -> Result<Value, McpError> {
    match action {
        "browse" => {
            let url = require_step_str(step, "url", "browse")?;
            let page = state.engine.navigate(url)?;
            let wom_value = serde_json::to_value(&page.wom)?;
            Ok(serde_json::json!({
                "url": page.url,
                "title": page.title,
                "render_ms": page.render_ms,
                "page_type": page.wom.page_type,
                "summary": page.wom.summary,
                "node_count": page.wom.nodes.len(),
                "wom": wom_value,
            }))
        }

        "navigate" => {
            let nav_action = step.get("action_type").and_then(|v| v.as_str());
            let url = step.get("url").and_then(|v| v.as_str());

            let page = match (url, nav_action) {
                (Some(u), _) => state.engine.navigate(u)?,
                (None, Some("back")) => state.engine.back()?,
                (None, Some("forward")) => state.engine.forward()?,
                _ => {
                    return Err(McpError::InvalidParams(
                        "navigate step requires 'url' or 'action_type'".into(),
                    ))
                }
            };
            Ok(serde_json::json!({
                "url": page.url,
                "title": page.title,
            }))
        }

        "click" => {
            let target = require_step_str(step, "target", "click")?;
            let result = state.engine.click(target)?;
            Ok(serde_json::to_value(result)?)
        }

        "type" => {
            let target = require_step_str(step, "target", "type")?;
            let text = require_step_str(step, "text", "type")?;
            state.engine.type_text(target, text)?;
            Ok(serde_json::json!({ "typed": text }))
        }

        "fill_form" => {
            let fields_val = step
                .get("fields")
                .ok_or_else(|| McpError::InvalidParams("fill_form: missing 'fields'".into()))?;
            let fields: HashMap<String, String> = serde_json::from_value(fields_val.clone())?;
            let count = fields.len();

            // Use enhanced fill_form via interact tool.
            super::interact::call(
                serde_json::json!({
                    "action": "fill_form",
                    "fields": fields_val,
                }),
                state,
            )
            .or_else(|_| {
                // Fallback: basic fill.
                state.engine.fill_form(&fields)?;
                Ok(serde_json::json!({ "filled": count }))
            })
        }

        "submit" => {
            let target = step.get("target").and_then(|v| v.as_str());
            let result = state.engine.submit(target)?;
            Ok(serde_json::to_value(result)?)
        }

        "press_key" => {
            let target = require_step_str(step, "target", "press_key")?;
            let key = require_step_str(step, "key", "press_key")?;
            state.engine.press_key(target, key)?;
            Ok(serde_json::json!({ "key": key }))
        }

        "wait" => {
            let timeout_ms = step
                .get("timeout_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(5000) as u32;

            let selector = step.get("selector").and_then(|v| v.as_str());
            let text = step.get("text").and_then(|v| v.as_str());

            match (selector, text) {
                (Some(sel), _) => {
                    let found = state.engine.wait_for(sel, timeout_ms)?;
                    Ok(serde_json::json!({ "found": found, "selector": sel }))
                }
                (None, Some(txt)) => {
                    let found = state.engine.wait_for_text(txt, timeout_ms)?;
                    Ok(serde_json::json!({ "found": found, "text": txt }))
                }
                _ => Err(McpError::InvalidParams(
                    "wait: requires 'selector' or 'text'".into(),
                )),
            }
        }

        "extract" => {
            let kind = step
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("text");
            super::extract::call(serde_json::json!({ "kind": kind }), state)
        }

        "eval" => {
            let code = require_step_str(step, "code", "eval")?;
            let result = state.engine.eval(code)?;
            Ok(serde_json::json!({ "result": result }))
        }

        "find" => {
            let target = require_step_str(step, "target", "find")?;
            let elements = state.engine.find_element(target)?;
            Ok(serde_json::json!({ "elements": elements, "count": elements.len() }))
        }

        "analyze_forms" => {
            super::interact::call(
                serde_json::json!({ "action": "analyze_forms" }),
                state,
            )
        }

        "cookie_consent" => {
            // Try common cookie consent selectors.
            let consent_selectors = [
                "button[id*='accept']",
                "button[class*='accept']",
                "button[id*='consent']",
                "button[class*='consent']",
                "[aria-label*='Accept']",
                "[aria-label*='accept']",
                "button[id*='agree']",
                "#onetrust-accept-btn-handler",
                ".cc-accept",
                ".cc-btn.cc-dismiss",
            ];

            for selector in &consent_selectors {
                let found = state.engine.find_element(selector);
                if let Ok(elements) = found {
                    if !elements.is_empty() {
                        let _ = state.engine.click(selector);
                        return Ok(serde_json::json!({
                            "dismissed": true,
                            "selector": selector,
                        }));
                    }
                }
            }

            Ok(serde_json::json!({
                "dismissed": false,
                "reason": "no cookie consent banner found",
            }))
        }

        other => Err(McpError::InvalidParams(format!(
            "unknown pipeline action: {other}"
        ))),
    }
}

/// Extract a required string field from a step, with context for errors.
fn require_step_str<'a>(step: &'a Value, key: &str, action: &str) -> Result<&'a str, McpError> {
    step.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::InvalidParams(format!("{action}: missing '{key}'")))
}
