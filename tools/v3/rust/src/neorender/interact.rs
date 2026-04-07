//! Interaction layer for NeoSession — semantic click, type, submit, select.
//!
//! Delegates to JS functions in browser.js (__neo_click, __neo_type, etc.)
//! and handles the resulting actions (navigation, form submission) on the Rust side.

use super::session::NeoSession;
use super::net::{RequestMode, RequestDestination};

/// Result of a click() operation.
#[derive(Debug, Clone)]
pub enum ClickResult {
    /// Element was clicked, no navigation triggered.
    Clicked { tag: String, text: String },
    /// Click triggered a navigation (e.g. <a href>). Page has been loaded.
    Navigated { url: String },
    /// Click triggered a form submission. Page has been loaded with the response.
    Submitted { url: String, method: String },
}

/// Result of a submit() operation.
#[derive(Debug, Clone)]
pub enum SubmitResult {
    /// Form was submitted and response loaded.
    Submitted { url: String, method: String },
    /// Submit failed (no form found, event not captured, etc.)
    Failed { error: String },
}

impl NeoSession {
    /// Click an element by target (CSS selector, text content, aria-label, placeholder, name).
    /// Returns what happened: simple click, navigation, or form submission.
    pub async fn click(&mut self, target: &str) -> Result<ClickResult, String> {
        let escaped = target.replace('\\', "\\\\").replace('\'', "\\'").replace('\n', "\\n");
        let js = format!("__neo_click('{}')", escaped);
        let result_json = self.eval(&js)?;

        let parsed: serde_json::Value = serde_json::from_str(&result_json)
            .map_err(|e| format!("click parse error: {e} — raw: {result_json}"))?;

        if parsed["ok"].as_bool() != Some(true) {
            return Err(parsed["error"].as_str().unwrap_or("click failed").to_string());
        }

        // Check if an action was triggered (navigation or submit)
        if let Some(action) = parsed.get("action") {
            let action_type = action["type"].as_str().unwrap_or("");
            match action_type {
                "navigate" => {
                    let url = action["url"].as_str().unwrap_or("").to_string();
                    if !url.is_empty() {
                        self.goto(&url).await?;
                        return Ok(ClickResult::Navigated { url });
                    }
                }
                "submit" => {
                    let result = self.handle_form_submit(action).await?;
                    match result {
                        SubmitResult::Submitted { url, method } => {
                            return Ok(ClickResult::Submitted { url, method });
                        }
                        SubmitResult::Failed { error } => {
                            return Err(error);
                        }
                    }
                }
                _ => {}
            }
        }

        // Simple click — no navigation
        let tag = parsed["clicked"].as_str().unwrap_or("").to_string();
        let text = parsed["text"].as_str().unwrap_or("").to_string();
        Ok(ClickResult::Clicked { tag, text })
    }

    /// Type text into an input/textarea found by target.
    /// Target resolution: CSS selector -> name -> placeholder -> aria-label -> label text -> data-testid.
    /// If `clear` is true, clears the field before typing.
    /// Dispatches char-by-char keyboard events (keydown/keypress/input/keyup) for framework compatibility.
    pub fn type_text(&mut self, target: &str, text: &str) -> Result<(), String> {
        self.type_text_opts(target, text, false)
    }

    /// Type text with option to clear the field first.
    pub fn type_text_opts(&mut self, target: &str, text: &str, clear: bool) -> Result<(), String> {
        let escaped_target = target.replace('\\', "\\\\").replace('\'', "\\'").replace('\n', "\\n");
        let escaped_text = text.replace('\\', "\\\\").replace('\'', "\\'").replace('\n', "\\n");
        let js = format!("__neo_type('{}', '{}', {})", escaped_target, escaped_text, clear);
        let result_json = self.eval(&js)?;

        let parsed: serde_json::Value = serde_json::from_str(&result_json)
            .map_err(|e| format!("type parse error: {e} — raw: {result_json}"))?;

        if parsed["ok"].as_bool() != Some(true) {
            return Err(parsed["error"].as_str().unwrap_or("type failed").to_string());
        }
        Ok(())
    }

    /// Find an element by target (CSS selector, text content, aria-label, placeholder, name, title, data-testid).
    /// Returns element info including tag, text, selector path, and attributes.
    pub fn find_element(&mut self, target: &str) -> Result<serde_json::Value, String> {
        let escaped = target.replace('\\', "\\\\").replace('\'', "\\'").replace('\n', "\\n");
        let js = format!("__neo_find('{}')", escaped);
        let result_json = self.eval(&js)?;

        let parsed: serde_json::Value = serde_json::from_str(&result_json)
            .map_err(|e| format!("find parse error: {e} — raw: {result_json}"))?;

        if parsed["ok"].as_bool() != Some(true) {
            return Err(parsed["error"].as_str().unwrap_or("element not found").to_string());
        }
        Ok(parsed)
    }

    /// Submit a form. If target is provided, finds the form by selector; otherwise submits the first form.
    /// Detects SPA protocol (Livewire, HTMX, Inertia, Turbo) and handles natively.
    /// For standard HTML forms, performs the actual HTTP request.
    pub async fn submit(&mut self, target: Option<&str>) -> Result<SubmitResult, String> {
        let js = match target {
            Some(t) => {
                let escaped = t.replace('\\', "\\\\").replace('\'', "\\'").replace('\n', "\\n");
                format!("__neo_submit('{}')", escaped)
            }
            None => "__neo_submit()".to_string(),
        };
        let result_json = self.eval(&js)?;

        let parsed: serde_json::Value = serde_json::from_str(&result_json)
            .map_err(|e| format!("submit parse error: {e} — raw: {result_json}"))?;

        if parsed["ok"].as_bool() != Some(true) {
            let error = parsed["error"].as_str().unwrap_or("submit failed").to_string();
            let error_msg = parsed["error_message"].as_str().map(|s| s.to_string());
            return Ok(SubmitResult::Failed {
                error: if let Some(msg) = error_msg {
                    format!("{} (server: {})", error, msg)
                } else {
                    error
                },
            });
        }

        let protocol = parsed["protocol"].as_str().unwrap_or("standard");

        // SPA protocols that already executed the HTTP request in JS
        match protocol {
            "livewire" | "htmx" | "inertia" | "turbo" => {
                let status = parsed["status"].as_u64().unwrap_or(0) as u16;
                let redirect = parsed["redirect"].as_str().filter(|s| !s.is_empty());

                if let Some(redirect_url) = redirect {
                    // Follow redirect — navigate to the new page
                    self.goto(redirect_url).await?;
                    return Ok(SubmitResult::Submitted {
                        url: redirect_url.to_string(),
                        method: "GET".to_string(),
                    });
                }

                if status >= 200 && status < 400 {
                    // Success but no redirect (e.g., validation error, or in-page update)
                    let url = parsed["action"]["url"].as_str().unwrap_or("").to_string();
                    let method = parsed["action"]["method"].as_str().unwrap_or("POST").to_string();
                    return Ok(SubmitResult::Submitted { url, method });
                }

                // Error from SPA protocol
                let error_msg = parsed["error_message"].as_str().unwrap_or("SPA submit failed");
                return Ok(SubmitResult::Failed {
                    error: format!("[{}] HTTP {} — {}", protocol, status, error_msg),
                });
            }
            "native" => {
                // Native JS handler captured the action
                if let Some(action) = parsed.get("action") {
                    return self.handle_form_submit(action).await;
                }
                return Ok(SubmitResult::Failed {
                    error: "Native submit: no action captured".to_string(),
                });
            }
            _ => {
                // Standard HTML form — handle_form_submit does the HTTP
                if let Some(action) = parsed.get("action") {
                    return self.handle_form_submit(action).await;
                }
                return Ok(SubmitResult::Failed {
                    error: "No action from submit".to_string(),
                });
            }
        }
    }

    /// Select an option in a <select> element.
    /// Target: CSS selector or name attribute of the select element.
    /// Value: the option value to select.
    pub fn select(&mut self, target: &str, value: &str) -> Result<(), String> {
        let escaped_target = target.replace('\\', "\\\\").replace('\'', "\\'").replace('\n', "\\n");
        let escaped_value = value.replace('\\', "\\\\").replace('\'', "\\'").replace('\n', "\\n");
        let js = format!("__neo_select('{}', '{}')", escaped_target, escaped_value);
        let result_json = self.eval(&js)?;

        let parsed: serde_json::Value = serde_json::from_str(&result_json)
            .map_err(|e| format!("select parse error: {e} — raw: {result_json}"))?;

        if parsed["ok"].as_bool() != Some(true) {
            return Err(parsed["error"].as_str().unwrap_or("select failed").to_string());
        }
        Ok(())
    }

    /// Fill multiple form fields at once.
    /// Fields: Vec of (target, value) where target can be CSS selector, name, placeholder, aria-label, id, or label text.
    pub fn fill_form(&mut self, fields: &[(String, String)]) -> Result<serde_json::Value, String> {
        let fields_map: serde_json::Map<String, serde_json::Value> = fields.iter()
            .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
            .collect();
        let fields_json = serde_json::to_string(&fields_map)
            .map_err(|e| format!("JSON serialize error: {e}"))?;
        let escaped = fields_json.replace('\\', "\\\\").replace('\'', "\\'");
        let js = format!("__neo_fill_form('{}')", escaped);
        let result_json = self.eval(&js)?;
        let parsed: serde_json::Value = serde_json::from_str(&result_json)
            .map_err(|e| format!("fill_form parse error: {e} — raw: {result_json}"))?;
        if parsed["ok"].as_bool() != Some(true) {
            return Err(parsed["error"].as_str().unwrap_or("fill_form failed").to_string());
        }
        Ok(parsed)
    }

    /// Handle a form submission action from browser.js.
    /// Collects the form data, performs the HTTP request, and navigates to the response.
    async fn handle_form_submit(&mut self, action: &serde_json::Value) -> Result<SubmitResult, String> {
        let url = action["url"].as_str().unwrap_or("").to_string();
        let method = action["method"].as_str().unwrap_or("GET").to_uppercase();
        let data = action["data"].as_object();

        if url.is_empty() {
            return Ok(SubmitResult::Failed {
                error: "No form action URL".to_string(),
            });
        }

        // Encode form data
        let encoded = if let Some(fields) = data {
            let pairs: Vec<(String, String)> = fields.iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                .collect();
            url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs(&pairs)
                .finish()
        } else {
            String::new()
        };

        match method.as_str() {
            "GET" => {
                // Append query params to URL
                let nav_url = if encoded.is_empty() {
                    url.clone()
                } else if url.contains('?') {
                    format!("{}&{}", url, encoded)
                } else {
                    format!("{}?{}", url, encoded)
                };
                self.goto(&nav_url).await?;
                Ok(SubmitResult::Submitted { url: nav_url, method })
            }
            _ => {
                // POST/PUT/etc — use network.fetch() then navigate to the response
                let headers_json = serde_json::json!({
                    "content-type": "application/x-www-form-urlencoded",
                }).to_string();

                let resp = self.fetch(
                    &url,
                    &method,
                    Some(&encoded),
                    Some(&headers_json),
                ).await?;

                // Parse the response to check if we got HTML back
                let resp_val: serde_json::Value = serde_json::from_str(&resp)
                    .unwrap_or_default();
                let status = resp_val["status"].as_u64().unwrap_or(0);
                let body = resp_val["body"].as_str().unwrap_or("");

                // If 3xx redirect, follow it
                if (300..400).contains(&(status as u16 as u64)) {
                    // The redirect URL would be in headers — for now, just report
                    return Ok(SubmitResult::Submitted { url, method });
                }

                // If we got HTML back, load it as the new page
                if body.contains("<html") || body.contains("<!DOCTYPE") || body.contains("<!doctype") {
                    // Inject the response HTML into the DOM
                    let html_json = serde_json::to_string(&body).unwrap_or_default();
                    let inject_js = format!(
                        r#"{{
                            const {{ document: freshDoc }} = __linkedom_parseHTML({});
                            if (freshDoc.head) document.head.innerHTML = freshDoc.head.innerHTML;
                            if (freshDoc.body) document.body.innerHTML = freshDoc.body.innerHTML;
                        }}"#,
                        html_json
                    );
                    self.eval(&inject_js).ok();
                }

                Ok(SubmitResult::Submitted { url, method })
            }
        }
    }
}
