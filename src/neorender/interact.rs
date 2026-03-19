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
    /// Target resolution: CSS selector -> name -> placeholder -> aria-label -> label text.
    pub fn type_text(&mut self, target: &str, text: &str) -> Result<(), String> {
        let escaped_target = target.replace('\\', "\\\\").replace('\'', "\\'").replace('\n', "\\n");
        let escaped_text = text.replace('\\', "\\\\").replace('\'', "\\'").replace('\n', "\\n");
        let js = format!("__neo_type('{}', '{}')", escaped_target, escaped_text);
        let result_json = self.eval(&js)?;

        let parsed: serde_json::Value = serde_json::from_str(&result_json)
            .map_err(|e| format!("type parse error: {e} — raw: {result_json}"))?;

        if parsed["ok"].as_bool() != Some(true) {
            return Err(parsed["error"].as_str().unwrap_or("type failed").to_string());
        }
        Ok(())
    }

    /// Submit a form. If target is provided, finds the form by selector; otherwise submits the first form.
    /// Performs the actual HTTP request (GET with query params or POST with form-encoded body).
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
            return Ok(SubmitResult::Failed {
                error: parsed["error"].as_str().unwrap_or("submit failed").to_string(),
            });
        }

        if let Some(action) = parsed.get("action") {
            return self.handle_form_submit(action).await;
        }

        Ok(SubmitResult::Failed {
            error: "Submit event not captured".to_string(),
        })
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
