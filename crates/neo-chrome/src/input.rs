//! CDP input/form tools — fill, type, press keys, handle dialogs.
//!
//! All methods operate on the page via the active CDP session,
//! using `Input.dispatchKeyEvent`, `Runtime.evaluate`, and `DOM.*`.

use crate::session::ChromeSession;
use crate::Result;
use serde_json::json;

// ─── Key mapping ───

/// CDP modifier bitmask values.
pub const MOD_ALT: u32 = 1;
pub const MOD_CTRL: u32 = 2;
pub const MOD_META: u32 = 4;
pub const MOD_SHIFT: u32 = 8;

/// A parsed key combination (e.g., "Control+Shift+A").
#[derive(Debug, Clone, PartialEq)]
pub struct KeyCombo {
    /// Combined modifier bitmask.
    pub modifiers: u32,
    /// The final key name (e.g., "a", "Enter", "Tab").
    pub key: String,
    /// The key code (e.g., "KeyA", "Enter").
    pub code: String,
    /// Windows virtual key code.
    pub key_code: u32,
}

/// Parse a key string like "Control+Shift+A" or "Enter" into a `KeyCombo`.
pub fn parse_key_combo(combo: &str) -> KeyCombo {
    let parts: Vec<&str> = combo.split('+').collect();
    let mut modifiers = 0u32;
    let mut key_name = "";

    for part in &parts {
        match part.to_lowercase().as_str() {
            "control" | "ctrl" => modifiers |= MOD_CTRL,
            "alt" | "option" => modifiers |= MOD_ALT,
            "meta" | "command" | "cmd" => modifiers |= MOD_META,
            "shift" => modifiers |= MOD_SHIFT,
            _ => key_name = part,
        }
    }

    // If no non-modifier key found, the last part is the key.
    if key_name.is_empty() {
        key_name = parts.last().unwrap_or(&"");
    }

    let (key, code, key_code) = map_key(key_name);

    KeyCombo {
        modifiers,
        key: key.to_string(),
        code: code.to_string(),
        key_code,
    }
}

/// Map a key name to (key, code, windowsVirtualKeyCode).
fn map_key(name: &str) -> (&str, &str, u32) {
    match name {
        "Enter" | "Return" => ("Enter", "Enter", 13),
        "Tab" => ("Tab", "Tab", 9),
        "Escape" | "Esc" => ("Escape", "Escape", 27),
        "Backspace" => ("Backspace", "Backspace", 8),
        "Delete" => ("Delete", "Delete", 46),
        "ArrowUp" | "Up" => ("ArrowUp", "ArrowUp", 38),
        "ArrowDown" | "Down" => ("ArrowDown", "ArrowDown", 40),
        "ArrowLeft" | "Left" => ("ArrowLeft", "ArrowLeft", 37),
        "ArrowRight" | "Right" => ("ArrowRight", "ArrowRight", 39),
        "Home" => ("Home", "Home", 36),
        "End" => ("End", "End", 35),
        "PageUp" => ("PageUp", "PageUp", 33),
        "PageDown" => ("PageDown", "PageDown", 34),
        "Space" | " " => (" ", "Space", 32),
        // F-keys
        "F1" => ("F1", "F1", 112),
        "F2" => ("F2", "F2", 113),
        "F3" => ("F3", "F3", 114),
        "F4" => ("F4", "F4", 115),
        "F5" => ("F5", "F5", 116),
        "F6" => ("F6", "F6", 117),
        "F7" => ("F7", "F7", 118),
        "F8" => ("F8", "F8", 119),
        "F9" => ("F9", "F9", 120),
        "F10" => ("F10", "F10", 121),
        "F11" => ("F11", "F11", 122),
        "F12" => ("F12", "F12", 123),
        // Single character — letter or digit
        other => {
            if other.len() == 1 {
                let ch = other.chars().next().unwrap();
                let upper = ch.to_ascii_uppercase();
                let vk = upper as u32;
                // We can't return a borrowed str for dynamic values,
                // so we leak — these are always short-lived single chars.
                let key_str: &str = Box::leak(other.to_string().into_boxed_str());
                let code_str: &str = if ch.is_ascii_alphabetic() {
                    Box::leak(format!("Key{}", upper).into_boxed_str())
                } else if ch.is_ascii_digit() {
                    Box::leak(format!("Digit{}", ch).into_boxed_str())
                } else {
                    key_str
                };
                (key_str, code_str, vk)
            } else {
                // Unknown key — pass through.
                let key_str: &str = Box::leak(other.to_string().into_boxed_str());
                (key_str, key_str, 0)
            }
        }
    }
}

/// Action to take on a browser dialog.
#[derive(Debug, Clone, PartialEq)]
pub enum DialogAction {
    /// Accept the dialog (OK / confirm).
    Accept,
    /// Dismiss the dialog (Cancel / dismiss).
    Dismiss,
}

// ─── ChromeSession input methods ───

impl ChromeSession {
    /// Fill a single form element identified by CSS selector.
    ///
    /// For `<input>` and `<textarea>`: clears existing value and types the new one.
    /// For `<select>`: sets the selected option by value.
    pub async fn fill(&self, selector: &str, value: &str) -> Result<()> {
        let escaped_sel = selector.replace('\\', "\\\\").replace('\'', "\\'");
        let escaped_val = value.replace('\\', "\\\\").replace('\'', "\\'");

        // Resolve node and focus it via DOM commands.
        let doc = self
            .cdp
            .send_to(&self.page_session_id, "DOM.getDocument", None)
            .await?;
        let root_node_id = doc
            .get("root")
            .and_then(|r| r.get("nodeId"))
            .and_then(|n| n.as_i64())
            .unwrap_or(0);

        let qs_result = self
            .cdp
            .send_to(
                &self.page_session_id,
                "DOM.querySelector",
                Some(json!({
                    "nodeId": root_node_id,
                    "selector": selector,
                })),
            )
            .await?;
        let node_id = qs_result
            .get("nodeId")
            .and_then(|n| n.as_i64())
            .unwrap_or(0);

        if node_id == 0 {
            return Err(crate::ChromeError::CommandFailed {
                method: "DOM.querySelector".into(),
                error: format!("Element not found: {selector}"),
            });
        }

        // Focus the element.
        self.cdp
            .send_to(
                &self.page_session_id,
                "DOM.focus",
                Some(json!({ "nodeId": node_id })),
            )
            .await?;

        // Clear existing value and set new one via JS, dispatching proper events.
        let js = format!(
            r#"(() => {{
                const el = document.querySelector('{escaped_sel}');
                if (!el) return 'not_found';
                if (el.tagName === 'SELECT') {{
                    const opts = Array.from(el.options);
                    const idx = opts.findIndex(o => o.value === '{escaped_val}' || o.textContent.trim() === '{escaped_val}');
                    if (idx >= 0) el.selectedIndex = idx;
                    el.dispatchEvent(new Event('change', {{ bubbles: true }}));
                    return 'select_ok';
                }}
                el.value = '';
                el.dispatchEvent(new Event('input', {{ bubbles: true }}));
                el.value = '{escaped_val}';
                el.dispatchEvent(new Event('input', {{ bubbles: true }}));
                el.dispatchEvent(new Event('change', {{ bubbles: true }}));
                return 'ok';
            }})()"#
        );

        self.eval(&js).await?;
        Ok(())
    }

    /// Fill multiple form elements at once, sequentially.
    pub async fn fill_form(&self, elements: &[(&str, &str)]) -> Result<()> {
        for (selector, value) in elements {
            self.fill(selector, value).await?;
        }
        Ok(())
    }

    /// Type text via keyboard events into the currently focused element.
    ///
    /// Each character is dispatched as keyDown + char + keyUp.
    /// If `submit_key` is provided, that key is pressed after the text.
    pub async fn type_text(&self, text: &str, submit_key: Option<&str>) -> Result<()> {
        for ch in text.chars() {
            let text_str = ch.to_string();

            // keyDown with char event
            self.cdp
                .send_to(
                    &self.page_session_id,
                    "Input.dispatchKeyEvent",
                    Some(json!({
                        "type": "keyDown",
                        "text": text_str,
                        "key": text_str,
                        "code": "",
                        "windowsVirtualKeyCode": ch as u32,
                    })),
                )
                .await?;

            // char event
            self.cdp
                .send_to(
                    &self.page_session_id,
                    "Input.dispatchKeyEvent",
                    Some(json!({
                        "type": "char",
                        "text": text_str,
                        "key": text_str,
                        "code": "",
                        "windowsVirtualKeyCode": ch as u32,
                    })),
                )
                .await?;

            // keyUp
            self.cdp
                .send_to(
                    &self.page_session_id,
                    "Input.dispatchKeyEvent",
                    Some(json!({
                        "type": "keyUp",
                        "text": text_str,
                        "key": text_str,
                        "code": "",
                        "windowsVirtualKeyCode": ch as u32,
                    })),
                )
                .await?;
        }

        if let Some(key) = submit_key {
            self.press_key(key).await?;
        }

        Ok(())
    }

    /// Press a key or key combination (e.g., "Enter", "Control+A", "Control+Shift+R").
    pub async fn press_key(&self, key: &str) -> Result<()> {
        let combo = parse_key_combo(key);

        // keyDown with modifiers
        self.cdp
            .send_to(
                &self.page_session_id,
                "Input.dispatchKeyEvent",
                Some(json!({
                    "type": "rawKeyDown",
                    "key": combo.key,
                    "code": combo.code,
                    "windowsVirtualKeyCode": combo.key_code,
                    "modifiers": combo.modifiers,
                })),
            )
            .await?;

        // keyUp
        self.cdp
            .send_to(
                &self.page_session_id,
                "Input.dispatchKeyEvent",
                Some(json!({
                    "type": "keyUp",
                    "key": combo.key,
                    "code": combo.code,
                    "windowsVirtualKeyCode": combo.key_code,
                    "modifiers": combo.modifiers,
                })),
            )
            .await?;

        Ok(())
    }

    /// Handle a browser JavaScript dialog (alert, confirm, prompt).
    pub async fn handle_dialog(
        &self,
        action: DialogAction,
        prompt_text: Option<&str>,
    ) -> Result<()> {
        let accept = matches!(action, DialogAction::Accept);
        let mut params = json!({ "accept": accept });

        if let Some(text) = prompt_text {
            params["promptText"] = json!(text);
        }

        self.cdp
            .send_to(
                &self.page_session_id,
                "Page.handleJavaScriptDialog",
                Some(params),
            )
            .await?;

        Ok(())
    }
}
