//! `interact` tool -- click, type, fill form, submit on the current page.
//!
//! After every successful interaction, auto-extracts the updated WOM
//! so the AI immediately sees what changed. Also detects SPA navigation
//! (URL changes via pushState/replaceState) and reports it.

use serde_json::Value;
use std::collections::HashMap;

use crate::state::McpState;
use crate::McpError;

use super::ToolDef;

/// Tool definition for `tools/list`.
pub(crate) fn definition() -> ToolDef {
    ToolDef {
        name: "interact",
        description: "Interact with the current page (click, type, fill, submit, scroll, hover, analyze). \
                       Returns updated page state (WOM) after every action.",
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["click", "type", "fill_form", "submit", "press_key", "find", "scroll", "hover", "analyze_forms"],
                    "description": "Interaction type"
                },
                "target": {
                    "type": "string",
                    "description": "Element target (CSS selector, text, aria-label)"
                },
                "text": {
                    "type": "string",
                    "description": "Text to type (for action=type)"
                },
                "fields": {
                    "type": "object",
                    "description": "Field name->value map (for action=fill_form). \
                                    Supports text, email, password, checkbox (true/false), \
                                    radio (option value), select (option value/text).",
                    "additionalProperties": { "type": "string" }
                },
                "key": {
                    "type": "string",
                    "description": "Key to press (for action=press_key): Enter, Tab, Escape, etc."
                },
                "direction": {
                    "type": "string",
                    "enum": ["up", "down", "top", "bottom"],
                    "description": "Scroll direction (for action=scroll, default: down)"
                },
                "amount": {
                    "type": "integer",
                    "description": "Pixels to scroll (for action=scroll, default: 500)"
                },
                "role": {
                    "type": "string",
                    "description": "ARIA role to filter by (for action=find): button, link, textbox, checkbox, radio, combobox, heading, img, navigation, search, form, dialog, alert, tab, menu, menuitem, listbox, option, switch, table"
                },
                "attribute": {
                    "type": "string",
                    "description": "Attribute to search by (for action=find): 'name=email', 'placeholder=Search', 'title=Close', 'data-testid=login-btn'"
                },
                "near": {
                    "type": "string",
                    "description": "Find elements near a reference element (for action=find): CSS selector or text of reference element. Returns results sorted by DOM proximity."
                }
            },
            "required": ["action"]
        }),
    }
}

/// Execute the `interact` tool.
pub fn call(args: Value, state: &mut McpState) -> Result<Value, McpError> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::InvalidParams("missing 'action'".into()))?;

    match action {
        "click" => call_click(&args, state),
        "type" => call_type(&args, state),
        "fill_form" => call_fill_form(&args, state),
        "submit" => call_submit(&args, state),
        "press_key" => call_press_key(&args, state),
        "find" => call_find(&args, state),
        "scroll" => call_scroll(&args, state),
        "hover" => call_hover(&args, state),
        "analyze_forms" => call_analyze_forms(state),
        other => Err(McpError::InvalidParams(format!("unknown action: {other}"))),
    }
}

/// Record URL before interaction, run action, then build response with
/// auto-extracted WOM and SPA navigation detection.
fn with_auto_extract(
    state: &mut McpState,
    action: &str,
    action_result: Value,
) -> Result<Value, McpError> {
    let wom = state.engine.extract()?;
    let url = state.engine.current_url().unwrap_or_default();
    let view = super::view::render_wom(&url, &wom);
    let fallback = action_result.to_string();
    let result_str = action_result.as_str().unwrap_or(&fallback);

    Ok(serde_json::json!(format!("[{action}] {result_str}\n\n{view}")))
}

/// Like `with_auto_extract` but also detects SPA navigation by comparing
/// URLs before and after the interaction.
fn with_navigation_detect(
    state: &mut McpState,
    action: &str,
    url_before: String,
    action_result: Value,
) -> Result<Value, McpError> {
    let url_after = state.engine.current_url().unwrap_or_default();
    let navigated = !url_before.is_empty() && url_after != url_before;

    if navigated {
        std::thread::sleep(std::time::Duration::from_millis(500));
    }

    let wom = state.engine.extract()?;
    let view = super::view::render_wom(&url_after, &wom);
    let fallback = action_result.to_string();
    let result_str = action_result.as_str().unwrap_or(&fallback);

    let nav_info = if navigated {
        format!("[navigated] {} → {}\n", url_before, url_after)
    } else {
        String::new()
    };

    Ok(serde_json::json!(format!("[{action}] {result_str}\n{nav_info}\n{view}")))
}

/// Execute a navigation-aware interaction (click, submit, press_key).
///
/// Records URL before, runs the action, detects SPA navigation, and
/// returns the response with auto-extracted WOM.
fn execute_nav_action<F>(
    state: &mut McpState,
    action_name: &str,
    action_fn: F,
) -> Result<Value, McpError>
where
    F: FnOnce(&mut McpState) -> Result<Value, McpError>,
{
    let url_before = state.engine.current_url().unwrap_or_default();
    let action_result = action_fn(state)?;
    with_navigation_detect(state, action_name, url_before, action_result)
}

fn call_click(args: &Value, state: &mut McpState) -> Result<Value, McpError> {
    let target = require_str(args, "target")?;
    execute_nav_action(state, "click", |s| {
        let result = s.engine.click(target)?;
        Ok(serde_json::to_value(result)?)
    })
}

fn call_type(args: &Value, state: &mut McpState) -> Result<Value, McpError> {
    let target = require_str(args, "target")?;
    let text = require_str(args, "text")?;
    state.engine.type_text(target, text)?;
    with_auto_extract(state, "type", serde_json::json!({ "typed": text }))
}

/// Enhanced fill_form with smart field matching:
/// - Auto-detects field type from WOM nodes (email, password, checkbox, select, radio, file)
/// - Handles select/dropdown by selecting the matching option
/// - Handles checkbox/radio with "true"/"false" values
/// - Skips file inputs with a warning
/// - Fills fields in document order (tab order)
/// - Post-fill validation: checks for error messages
fn call_fill_form(args: &Value, state: &mut McpState) -> Result<Value, McpError> {
    let fields_val = args
        .get("fields")
        .ok_or_else(|| McpError::InvalidParams("missing 'fields'".into()))?;

    let fields: HashMap<String, String> = serde_json::from_value(fields_val.clone())?;
    let field_count = fields.len();

    // Get current WOM to analyze form fields.
    let wom = state.engine.extract()?;

    let mut filled: Vec<Value> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();

    // Build a lookup of WOM nodes by name, label, id, and placeholder.
    let form_nodes: Vec<_> = wom
        .nodes
        .iter()
        .filter(|n| {
            matches!(n.tag.as_str(), "input" | "textarea" | "select")
                && n.interactive
                && !n.disabled
                && !n.readonly
        })
        .collect();

    // Sort fields by their document order in WOM (tab order).
    let mut ordered_fields: Vec<(&String, &String)> = fields.iter().collect();
    ordered_fields.sort_by_key(|(field_name, _)| {
        form_nodes
            .iter()
            .position(|n| field_matches(n, field_name))
            .unwrap_or(usize::MAX)
    });

    for (field_name, field_value) in &ordered_fields {
        // Find matching WOM node.
        let matched_node = form_nodes.iter().find(|n| field_matches(n, field_name));

        match matched_node {
            Some(node) => {
                let input_type = node.input_type.as_deref().unwrap_or("text");

                match input_type {
                    // File inputs: skip with warning.
                    "file" => {
                        warnings.push(format!(
                            "field '{}': file inputs cannot be filled via fill_form",
                            field_name
                        ));
                        skipped.push(field_name.to_string());
                        continue;
                    }

                    // Checkbox: interpret "true"/"false".
                    "checkbox" => {
                        let want_checked = matches!(
                            field_value.to_lowercase().as_str(),
                            "true" | "1" | "yes" | "on"
                        );
                        let is_checked = node.checked.unwrap_or(false);

                        if want_checked != is_checked {
                            let target = best_selector(node);
                            let _ = state.engine.click(&target);
                        }

                        filled.push(serde_json::json!({
                            "field": field_name,
                            "type": "checkbox",
                            "value": want_checked,
                        }));
                        continue;
                    }

                    // Radio: click the matching option.
                    "radio" => {
                        let target = format!(
                            "input[type=radio][name=\"{}\"][value=\"{}\"]",
                            node.name.as_deref().unwrap_or(field_name),
                            field_value
                        );
                        let _ = state.engine.click(&target);
                        filled.push(serde_json::json!({
                            "field": field_name,
                            "type": "radio",
                            "value": field_value,
                        }));
                        continue;
                    }

                    _ => {}
                }

                // Handle <select> elements.
                if node.tag == "select" {
                    let selector = best_selector(node);
                    let js = format!(
                        r#"(function() {{
                            var el = document.querySelector('{}');
                            if (!el) return 'not_found';
                            var opts = el.options;
                            for (var i = 0; i < opts.length; i++) {{
                                if (opts[i].value === '{}' || opts[i].text === '{}') {{
                                    el.selectedIndex = i;
                                    el.dispatchEvent(new Event('change', {{bubbles: true}}));
                                    return 'selected:' + opts[i].value;
                                }}
                            }}
                            return 'no_match';
                        }})()"#,
                        selector.replace('\'', "\\'"),
                        field_value.replace('\'', "\\'"),
                        field_value.replace('\'', "\\'"),
                    );
                    let result = state
                        .engine
                        .eval(&js)
                        .unwrap_or_else(|_| "eval_error".into());
                    filled.push(serde_json::json!({
                        "field": field_name,
                        "type": "select",
                        "value": field_value,
                        "result": result,
                    }));
                    continue;
                }

                // Validate format hints before filling.
                let validation_warning = validate_field_value(input_type, field_value);
                if let Some(warn) = validation_warning {
                    warnings.push(format!("field '{}': {}", field_name, warn));
                }

                // Standard text/email/password/etc: use type_text.
                let target = best_selector(node);
                state.engine.type_text(&target, field_value)?;

                filled.push(serde_json::json!({
                    "field": field_name,
                    "type": input_type,
                    "value": field_value,
                }));
            }

            None => {
                // No WOM node matched -- fall back to basic engine fill_form.
                let single: HashMap<String, String> =
                    [(field_name.to_string(), field_value.to_string())]
                        .into_iter()
                        .collect();
                match state.engine.fill_form(&single) {
                    Ok(()) => {
                        filled.push(serde_json::json!({
                            "field": field_name,
                            "type": "fallback",
                            "value": field_value,
                        }));
                    }
                    Err(e) => {
                        warnings.push(format!(
                            "field '{}': not found in WOM, fallback failed: {}",
                            field_name, e
                        ));
                        skipped.push(field_name.to_string());
                    }
                }
            }
        }
    }

    // Post-fill validation: check for error messages on the page.
    let validation_errors = check_post_fill_errors(state);

    let action_result = serde_json::json!({
        "filled": filled.len(),
        "total_fields": field_count,
        "details": filled,
        "skipped": skipped,
        "warnings": warnings,
        "validation_errors": validation_errors,
    });

    with_auto_extract(state, "fill_form", action_result)
}

fn call_submit(args: &Value, state: &mut McpState) -> Result<Value, McpError> {
    let target = args.get("target").and_then(|v| v.as_str());
    execute_nav_action(state, "submit", |s| {
        let result = s.engine.submit(target)?;
        Ok(serde_json::to_value(result)?)
    })
}

fn call_press_key(args: &Value, state: &mut McpState) -> Result<Value, McpError> {
    let target = require_str(args, "target")?;
    let key = require_str(args, "key")?;
    execute_nav_action(state, "press_key", |s| {
        s.engine.press_key(target, key)?;
        Ok(serde_json::json!({ "key": key }))
    })
}

fn call_find(args: &Value, state: &mut McpState) -> Result<Value, McpError> {
    let target = require_str(args, "target")?;
    let elements = state.engine.find_element(target)?;
    Ok(serde_json::json!({ "ok": true, "elements": elements, "count": elements.len() }))
}

fn call_scroll(args: &Value, state: &mut McpState) -> Result<Value, McpError> {
    let target = args.get("target").and_then(|v| v.as_str());
    let direction = args
        .get("direction")
        .and_then(|v| v.as_str())
        .unwrap_or("down");
    let amount = args
        .get("amount")
        .and_then(|v| v.as_i64())
        .unwrap_or(500);

    let js = match (direction, target) {
        ("top", Some(sel)) => format!(
            r#"(function(){{
                var el=document.querySelector({sel});
                if(!el) return JSON.stringify({{error:"element not found"}});
                el.scrollTo(0,0);
                return JSON.stringify({{scrollTop:el.scrollTop,scrollHeight:el.scrollHeight,clientHeight:el.clientHeight,can_scroll_more:el.scrollTop+el.clientHeight<el.scrollHeight,at_bottom:false}});
            }})()"#,
            sel = serde_json::to_string(sel).unwrap_or_default()
        ),
        ("bottom", Some(sel)) => format!(
            r#"(function(){{
                var el=document.querySelector({sel});
                if(!el) return JSON.stringify({{error:"element not found"}});
                el.scrollTo(0,el.scrollHeight);
                return JSON.stringify({{scrollTop:el.scrollTop,scrollHeight:el.scrollHeight,clientHeight:el.clientHeight,can_scroll_more:false,at_bottom:true}});
            }})()"#,
            sel = serde_json::to_string(sel).unwrap_or_default()
        ),
        ("top", None) => r#"(function(){
            window.scrollTo(0,0);
            return JSON.stringify({scrollTop:window.pageYOffset||document.documentElement.scrollTop,scrollHeight:document.body.scrollHeight,clientHeight:window.innerHeight||document.documentElement.clientHeight,can_scroll_more:true,at_bottom:false});
        })()"#.to_string(),
        ("bottom", None) => r#"(function(){
            window.scrollTo(0,document.body.scrollHeight);
            var st=window.pageYOffset||document.documentElement.scrollTop;
            var sh=document.body.scrollHeight;
            var ch=window.innerHeight||document.documentElement.clientHeight;
            return JSON.stringify({scrollTop:st,scrollHeight:sh,clientHeight:ch,can_scroll_more:false,at_bottom:true});
        })()"#.to_string(),
        (_, Some(sel)) => {
            let pixels = if direction == "up" { -amount } else { amount };
            format!(
                r#"(function(){{
                    var el=document.querySelector({sel});
                    if(!el) return JSON.stringify({{error:"element not found"}});
                    el.scrollBy(0,{pixels});
                    return JSON.stringify({{scrollTop:el.scrollTop,scrollHeight:el.scrollHeight,clientHeight:el.clientHeight,can_scroll_more:el.scrollTop+el.clientHeight<el.scrollHeight,at_bottom:el.scrollTop+el.clientHeight>=el.scrollHeight}});
                }})()"#,
                sel = serde_json::to_string(sel).unwrap_or_default(),
                pixels = pixels
            )
        }
        (_, None) => {
            let pixels = if direction == "up" { -amount } else { amount };
            format!(
                r#"(function(){{
                    window.scrollBy(0,{pixels});
                    var st=window.pageYOffset||document.documentElement.scrollTop;
                    var sh=document.body.scrollHeight;
                    var ch=window.innerHeight||document.documentElement.clientHeight;
                    return JSON.stringify({{scrollTop:st,scrollHeight:sh,clientHeight:ch,can_scroll_more:st+ch<sh,at_bottom:st+ch>=sh}});
                }})()"#,
                pixels = pixels
            )
        }
    };

    let result_str = state.engine.eval(&js)?;
    let scroll_data: Value = serde_json::from_str(&result_str).unwrap_or_else(|_| {
        serde_json::json!({ "raw": result_str })
    });

    Ok(serde_json::json!({
        "ok": true,
        "action": "scroll",
        "direction": direction,
        "scroll_state": scroll_data,
    }))
}

fn call_hover(args: &Value, state: &mut McpState) -> Result<Value, McpError> {
    let target = require_str(args, "target")?;
    let sel_json = serde_json::to_string(target).unwrap_or_default();

    let js = format!(
        r#"(function(){{
            var el=document.querySelector({sel});
            if(!el) return JSON.stringify({{error:"element not found",selector:{sel}}});
            el.dispatchEvent(new MouseEvent('mouseenter',{{bubbles:false,cancelable:true}}));
            el.dispatchEvent(new MouseEvent('mouseover',{{bubbles:true,cancelable:true}}));
            return JSON.stringify({{ok:true,tag:el.tagName.toLowerCase(),text:(el.textContent||'').trim().substring(0,100)}});
        }})()"#,
        sel = sel_json
    );

    let result_str = state.engine.eval(&js)?;
    let hover_data: Value = serde_json::from_str(&result_str).unwrap_or_else(|_| {
        serde_json::json!({ "raw": result_str })
    });

    // Re-extract WOM to capture any tooltips/dropdowns that appeared.
    let wom = state.engine.extract()?;
    let wom_value = serde_json::to_value(&wom)?;

    Ok(serde_json::json!({
        "ok": true,
        "action": "hover",
        "hover_result": hover_data,
        "url": state.engine.current_url().unwrap_or_default(),
        "page_type": wom.page_type,
        "summary": wom.summary,
        "node_count": wom.nodes.len(),
        "wom": wom_value,
    }))
}

/// Analyze all forms on the current page, returning field metadata.
fn call_analyze_forms(state: &mut McpState) -> Result<Value, McpError> {
    let wom = state.engine.extract()?;

    // Find all form nodes and their associated fields.
    let form_nodes: Vec<_> = wom.nodes.iter().filter(|n| n.tag == "form").collect();

    // Find all input/select/textarea nodes.
    let field_nodes: Vec<_> = wom
        .nodes
        .iter()
        .filter(|n| matches!(n.tag.as_str(), "input" | "textarea" | "select"))
        .collect();

    // Find submit buttons.
    let submit_buttons: Vec<_> = wom
        .nodes
        .iter()
        .filter(|n| {
            (n.tag == "button" || n.tag == "input")
                && (n.input_type.as_deref() == Some("submit")
                    || n.label.to_lowercase().contains("submit")
                    || n.label.to_lowercase().contains("sign in")
                    || n.label.to_lowercase().contains("log in")
                    || n.label.to_lowercase().contains("send")
                    || n.label.to_lowercase().contains("register")
                    || n.label.to_lowercase().contains("sign up")
                    || n.role == "button")
        })
        .collect();

    // Build forms array. If no <form> tags, create a synthetic one grouping all fields.
    let mut forms: Vec<Value> = Vec::new();

    if form_nodes.is_empty() && !field_nodes.is_empty() {
        let fields_json = build_field_list(&field_nodes);
        let submit = submit_buttons.first().map(|b| {
            serde_json::json!({
                "text": b.label,
                "selector": best_selector(b),
            })
        });

        forms.push(serde_json::json!({
            "action": Value::Null,
            "method": Value::Null,
            "fields": fields_json,
            "submit_button": submit,
            "implicit": true,
        }));
    } else {
        for form_node in &form_nodes {
            let form_fields: Vec<_> = field_nodes
                .iter()
                .filter(|f| {
                    f.form_id.as_deref() == Some(&form_node.label) || f.form_id.is_none()
                })
                .copied()
                .collect();

            let fields_json = build_field_list(&form_fields);

            let submit = submit_buttons.first().map(|b| {
                serde_json::json!({
                    "text": b.label,
                    "selector": best_selector(b),
                })
            });

            forms.push(serde_json::json!({
                "action": form_node.href,
                "method": Value::Null,
                "fields": fields_json,
                "submit_button": submit,
            }));
        }
    }

    Ok(serde_json::json!({
        "ok": true,
        "forms": forms,
        "total_forms": forms.len(),
        "total_fields": field_nodes.len(),
    }))
}

// -- Helper functions --

/// Check if a WOM node matches a field name (by name, label, id, placeholder, autocomplete).
fn field_matches(node: &neo_extract::WomNode, field_name: &str) -> bool {
    let name_lower = field_name.to_lowercase();

    if let Some(ref name) = node.name {
        if name.to_lowercase() == name_lower {
            return true;
        }
    }

    if node.label.to_lowercase() == name_lower {
        return true;
    }

    if node.id.to_lowercase().contains(&name_lower) {
        return true;
    }

    if let Some(ref ph) = node.placeholder {
        if ph.to_lowercase().contains(&name_lower) {
            return true;
        }
    }

    if let Some(ref ac) = node.autocomplete {
        if ac.to_lowercase() == name_lower {
            return true;
        }
    }

    false
}

/// Get the best CSS selector for a WOM node.
fn best_selector(node: &neo_extract::WomNode) -> String {
    if let Some(ref name) = node.name {
        if !name.is_empty() {
            return format!("[name=\"{}\"]", name);
        }
    }
    if !node.label.is_empty() {
        return node.label.clone();
    }
    format!("#{}", node.id)
}

/// Validate a field value based on input type. Returns a warning if invalid.
fn validate_field_value(input_type: &str, value: &str) -> Option<String> {
    match input_type {
        "email" => {
            if !value.contains('@') || !value.contains('.') {
                Some(format!("'{}' may not be a valid email address", value))
            } else {
                None
            }
        }
        "url" => {
            if !value.starts_with("http://") && !value.starts_with("https://") {
                Some(format!(
                    "'{}' may not be a valid URL (missing http(s)://)",
                    value
                ))
            } else {
                None
            }
        }
        "tel" => {
            let digits: usize = value.chars().filter(|c| c.is_ascii_digit()).count();
            if digits < 7 {
                Some(format!(
                    "'{}' may not be a valid phone number (too few digits)",
                    value
                ))
            } else {
                None
            }
        }
        "number" => {
            if value.parse::<f64>().is_err() {
                Some(format!("'{}' is not a valid number", value))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Check for post-fill validation errors on the page (common CSS patterns).
fn check_post_fill_errors(state: &mut McpState) -> Vec<String> {
    let error_selectors = [
        ".error",
        ".invalid",
        ".form-error",
        ".field-error",
        ".validation-error",
        "[role=\"alert\"]",
        ".alert-danger",
        ".has-error",
        "[aria-invalid=\"true\"]",
    ];

    let mut errors = Vec::new();

    for selector in &error_selectors {
        if let Ok(elements) = state.engine.find_element(selector) {
            for el in &elements {
                if !el.label.is_empty() && el.label.len() < 200 {
                    errors.push(format!("[{}] {}", selector, el.label));
                }
            }
        }
    }

    errors
}

/// Build a JSON array of field metadata from WOM nodes.
fn build_field_list(nodes: &[&neo_extract::WomNode]) -> Vec<Value> {
    nodes
        .iter()
        .map(|n| {
            let field_type = n.input_type.as_deref().unwrap_or(match n.tag.as_str() {
                "textarea" => "textarea",
                "select" => "select",
                _ => "text",
            });

            let mut field = serde_json::json!({
                "name": n.name.as_deref().unwrap_or(&n.label),
                "type": field_type,
                "label": n.label,
                "required": n.required,
                "selector": best_selector(n),
            });

            if let Some(ref ph) = n.placeholder {
                field["placeholder"] = serde_json::json!(ph);
            }
            if let Some(ref val) = n.value {
                field["current_value"] = serde_json::json!(val);
            }
            if let Some(checked) = n.checked {
                field["checked"] = serde_json::json!(checked);
            }
            if !n.options.is_empty() {
                let opts: Vec<Value> = n
                    .options
                    .iter()
                    .map(|o| {
                        serde_json::json!({
                            "value": o.value,
                            "text": o.text,
                            "selected": o.selected,
                        })
                    })
                    .collect();
                field["options"] = serde_json::json!(opts);
            }
            if n.disabled {
                field["disabled"] = serde_json::json!(true);
            }
            if n.readonly {
                field["readonly"] = serde_json::json!(true);
            }
            if let Some(ref pattern) = n.pattern {
                field["pattern"] = serde_json::json!(pattern);
            }
            if let Some(ref ac) = n.autocomplete {
                field["autocomplete"] = serde_json::json!(ac);
            }
            if let Some(ref min) = n.min {
                field["min"] = serde_json::json!(min);
            }
            if let Some(ref max) = n.max {
                field["max"] = serde_json::json!(max);
            }

            field
        })
        .collect()
}

/// Extract a required string field from args.
fn require_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, McpError> {
    args.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::InvalidParams(format!("missing '{key}'")))
}
