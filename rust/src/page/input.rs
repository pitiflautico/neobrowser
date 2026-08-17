//! Putting values into a page: keystrokes and form controls.
//!
//! Both paths exist because neither is sufficient alone. Keystrokes are what a real user
//! produces, so they trigger the `keydown`/`input`/`keyup` handlers that React and Vue
//! listen for — but no sequence of keystrokes can set a `<select>` reliably. Form controls
//! take the direct route and then dispatch the events the frameworks need, because setting
//! `.value` without them leaves the DOM correct and the application's state stale.

use std::time::Duration;

use serde_json::{json, Value};

use crate::cdp::{CdpClient, CdpError};

use super::eval::js;

/// Type into the focused element. `human=true` emits per-key keydown/keyup with a
/// human-like cadence (isTrusted events anti-bot layers expect); `false` uses the
/// instant `Input.insertText` (React/Vue-safe paste).
pub async fn type_text(client: &CdpClient, text: &str, human: bool) -> Result<(), CdpError> {
    if !human {
        client
            .send("Input.insertText", json!({ "text": text }))
            .await?;
        return Ok(());
    }
    let mut rng = Jitter::new(text.len() as u64 ^ 0x9E37_79B9);
    for ch in text.chars() {
        // Control chars need real key metadata — `text: "\n"` alone is a key
        // event for *no key*, and editors (Draft.js, Quill, plain textareas)
        // silently drop it, collapsing the user's line breaks.
        if ch == '\n' {
            for ev in ["keyDown", "keyUp"] {
                client
                    .send(
                        "Input.dispatchKeyEvent",
                        json!({
                            "type": ev,
                            "key": "Enter",
                            "code": "Enter",
                            "windowsVirtualKeyCode": 13,
                            "nativeVirtualKeyCode": 13,
                            "text": "\r"
                        }),
                    )
                    .await?;
            }
        } else {
            let s = ch.to_string();
            client
                .send(
                    "Input.dispatchKeyEvent",
                    json!({ "type": "keyDown", "text": s, "key": s }),
                )
                .await?;
            client
                .send(
                    "Input.dispatchKeyEvent",
                    json!({ "type": "keyUp", "text": s, "key": s }),
                )
                .await?;
        }
        // 30–120ms inter-key delay, dependency-free pseudo-random.
        let ms = 30 + (rng.next() % 90);
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }
    Ok(())
}

/// Dependency-free xorshift for humanised typing jitter (not security-sensitive).
pub(super) struct Jitter(u64);
impl Jitter {
    pub(super) fn new(seed: u64) -> Self {
        Jitter(seed | 1)
    }
    pub(super) fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

// --- B3: interaction coverage ---------------------------------------------------

/// Named keys mapped to the fields CDP needs.
///
/// `Input.dispatchKeyEvent` wants `key`, `code`, `windowsVirtualKeyCode` and
/// `nativeVirtualKeyCode` to agree; sending only `key` produces an event a page's handler
/// ignores, which looks like a working keypress that does nothing.
fn key_spec(name: &str) -> Option<(&'static str, &'static str, i64)> {
    Some(match name.trim().to_ascii_lowercase().as_str() {
        "enter" | "return" => ("Enter", "Enter", 13),
        "tab" => ("Tab", "Tab", 9),
        "escape" | "esc" => ("Escape", "Escape", 27),
        "backspace" => ("Backspace", "Backspace", 8),
        "delete" | "del" => ("Delete", "Delete", 46),
        "arrowup" | "up" => ("ArrowUp", "ArrowUp", 38),
        "arrowdown" | "down" => ("ArrowDown", "ArrowDown", 40),
        "arrowleft" | "left" => ("ArrowLeft", "ArrowLeft", 37),
        "arrowright" | "right" => ("ArrowRight", "ArrowRight", 39),
        "home" => ("Home", "Home", 36),
        "end" => ("End", "End", 35),
        "pageup" => ("PageUp", "PageUp", 33),
        "pagedown" => ("PageDown", "PageDown", 34),
        "space" => (" ", "Space", 32),
        _ => return None,
    })
}

/// CDP modifier bitmask: Alt=1, Ctrl=2, Meta=4, Shift=8.
fn modifier_mask(modifiers: &[String]) -> i64 {
    modifiers.iter().fold(0, |acc, m| {
        acc | match m.trim().to_ascii_lowercase().as_str() {
            "alt" => 1,
            "ctrl" | "control" => 2,
            "meta" | "cmd" | "command" => 4,
            "shift" => 8,
            _ => 0,
        }
    })
}

/// Press a named key, optionally with modifiers — `press("Enter")`, `press("a", ["ctrl"])`.
pub async fn press_key(
    client: &CdpClient,
    key: &str,
    modifiers: &[String],
) -> Result<String, CdpError> {
    let mask = modifier_mask(modifiers);
    let (key_name, code, vk) = match key_spec(key) {
        Some(spec) => spec,
        None => {
            // A single printable character: send it as itself rather than refusing, since
            // `press("a", ["ctrl"])` is the natural way to express a shortcut.
            let mut chars = key.chars();
            let (Some(c), None) = (chars.next(), chars.next()) else {
                return Err(CdpError::Closed(format!(
                    "press: unknown key {key:?}. Use a printable character or one of \
                     Enter/Tab/Escape/Backspace/Delete/Arrow*/Home/End/PageUp/PageDown/Space"
                )));
            };
            let upper = c.to_ascii_uppercase() as i64;
            for ty in ["keyDown", "keyUp"] {
                client
                    .send(
                        "Input.dispatchKeyEvent",
                        json!({
                            "type": ty,
                            "key": c.to_string(),
                            // With a modifier held, a printable key must NOT carry text, or
                            // Ctrl+A would also insert an "a".
                            "text": if mask == 0 { c.to_string() } else { String::new() },
                            "modifiers": mask,
                            "windowsVirtualKeyCode": upper,
                            "nativeVirtualKeyCode": upper,
                        }),
                    )
                    .await?;
            }
            return Ok(format!("pressed {c:?} with modifiers {modifiers:?}"));
        }
    };
    for ty in ["keyDown", "keyUp"] {
        client
            .send(
                "Input.dispatchKeyEvent",
                json!({
                    "type": ty,
                    "key": key_name,
                    "code": code,
                    // `text` is what makes a printable key insert; a named key like Enter
                    // must NOT carry text, or it types a character instead of acting.
                    "text": if key_name.len() == 1 && mask == 0 { key_name } else { "" },
                    "modifiers": mask,
                    "windowsVirtualKeyCode": vk,
                    "nativeVirtualKeyCode": vk,
                }),
            )
            .await?;
    }
    Ok(format!("pressed {key_name} with modifiers {modifiers:?}"))
}

/// Hover over an element: move the real cursor there without pressing.
///
/// Needed for menus and tooltips that only render on `mouseover`; a JS `dispatchEvent` is
/// not `isTrusted` and many libraries check.
pub async fn set_control(
    client: &CdpClient,
    selector: &str,
    value: &str,
) -> Result<String, CdpError> {
    let snippet = crate::js::set_control()
        .with(
            "SEL",
            &serde_json::to_string(selector).unwrap_or_else(|_| "\"\"".into()),
        )
        .with(
            "VALUE",
            &serde_json::to_string(value).unwrap_or_else(|_| "\"\"".into()),
        );
    let raw = js(client, &snippet.returning()).await?;
    Ok(match raw {
        Value::String(s) => s,
        other => other.to_string(),
    })
}
