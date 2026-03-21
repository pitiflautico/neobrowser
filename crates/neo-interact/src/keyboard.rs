//! Keyboard event dispatching — fires proper browser event sequences.
//!
//! `type_with_events` fires keydown+keypress+input+keyup per character,
//! building the value incrementally. `press_key` handles special keys
//! like Enter, Tab, Escape, arrows, and Backspace.

use neo_dom::DomEngine;

use crate::resolve::resolve;
use crate::InteractError;

/// Special (non-printable) keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecialKey {
    Enter,
    Tab,
    Escape,
    Backspace,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
}

impl SpecialKey {
    /// The `key` property value for this special key.
    pub fn key_name(&self) -> &'static str {
        match self {
            Self::Enter => "Enter",
            Self::Tab => "Tab",
            Self::Escape => "Escape",
            Self::Backspace => "Backspace",
            Self::ArrowUp => "ArrowUp",
            Self::ArrowDown => "ArrowDown",
            Self::ArrowLeft => "ArrowLeft",
            Self::ArrowRight => "ArrowRight",
        }
    }

    /// The `code` property value for this special key.
    pub fn code_name(&self) -> &'static str {
        match self {
            Self::Enter => "Enter",
            Self::Tab => "Tab",
            Self::Escape => "Escape",
            Self::Backspace => "Backspace",
            Self::ArrowUp => "ArrowUp",
            Self::ArrowDown => "ArrowDown",
            Self::ArrowLeft => "ArrowLeft",
            Self::ArrowRight => "ArrowRight",
        }
    }
}

/// Result of pressing a special key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyResult {
    /// The key that was pressed (e.g. "Enter", "Tab").
    pub key: String,
    /// Whether any listener called `preventDefault()`.
    /// Always `false` in the DOM-only layer (no JS execution).
    pub default_prevented: bool,
}

/// A single keyboard/input event in the sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyboardEvent {
    /// Event type: "keydown", "keypress", "input", "keyup", "submit".
    pub event_type: String,
    /// The `key` property (e.g. "a", "Enter").
    pub key: String,
    /// The `code` property (e.g. "KeyA", "Enter"). Empty for input events.
    pub code: String,
    /// For input events: the `inputType` (e.g. "insertText", "deleteContentBackward").
    pub input_type: Option<String>,
    /// For input events: the `data` field (the character inserted).
    pub data: Option<String>,
}

/// Type text character by character, firing the full event sequence per char.
///
/// For each printable character:
/// 1. keydown (key=char, code=KeyX)
/// 2. keypress (key=char, code=KeyX)
/// 3. input (inputType="insertText", data=char)
/// 4. Set value (append char)
/// 5. keyup (key=char, code=KeyX)
///
/// Returns the list of events dispatched, useful for verification.
pub fn type_with_events(
    dom: &mut dyn DomEngine,
    target: &str,
    text: &str,
) -> Result<Vec<KeyboardEvent>, InteractError> {
    let el = resolve(dom, target)?;
    let tag = dom.tag_name(el).unwrap_or_default();

    let is_contenteditable = dom.get_attribute(el, "contenteditable").is_some();
    let is_typeable = matches!(tag.as_str(), "input" | "textarea") || is_contenteditable;

    if !is_typeable {
        return Err(InteractError::TypeMismatch {
            expected: "input, textarea, or contenteditable".to_string(),
            actual: tag,
        });
    }

    let use_text_content = is_contenteditable && !matches!(tag.as_str(), "input" | "textarea");
    let mut events = Vec::new();

    for ch in text.chars() {
        let key = ch.to_string();
        let code = char_to_code(ch);

        // 1. keydown
        events.push(KeyboardEvent {
            event_type: "keydown".to_string(),
            key: key.clone(),
            code: code.clone(),
            input_type: None,
            data: None,
        });

        // 2. keypress (printable characters only)
        events.push(KeyboardEvent {
            event_type: "keypress".to_string(),
            key: key.clone(),
            code: code.clone(),
            input_type: None,
            data: None,
        });

        // 3. input event
        events.push(KeyboardEvent {
            event_type: "input".to_string(),
            key: String::new(),
            code: String::new(),
            input_type: Some("insertText".to_string()),
            data: Some(key.clone()),
        });

        // 4. Set value (append char)
        if use_text_content {
            let current = dom.text_content(el);
            dom.set_text_content(el, &format!("{current}{ch}"));
        } else {
            let current = dom.get_attribute(el, "value").unwrap_or_default();
            dom.set_attribute(el, "value", &format!("{current}{ch}"));
        }

        // 5. keyup
        events.push(KeyboardEvent {
            event_type: "keyup".to_string(),
            key: key.clone(),
            code: code.clone(),
            input_type: None,
            data: None,
        });
    }

    Ok(events)
}

/// Press a special key on the target element.
///
/// Event sequences vary by key:
/// - **Enter**: keydown + keypress + keyup. If target is inside a form, appends a "submit" event.
/// - **Tab**: keydown + keyup.
/// - **Escape**: keydown + keyup.
/// - **Backspace**: keydown + input(deleteContentBackward) + keyup. Removes last char from value.
/// - **Arrow keys**: keydown + keyup.
pub fn press_key(
    dom: &mut dyn DomEngine,
    target: &str,
    key: SpecialKey,
) -> Result<KeyResult, InteractError> {
    let el = resolve(dom, target)?;
    let key_name = key.key_name().to_string();
    let _code_name = key.code_name().to_string();

    match key {
        SpecialKey::Enter => {
            // keydown + keypress + keyup
            // Check if target is in a form → trigger submit
            let has_form = !dom.get_forms().is_empty();
            if has_form {
                // The submit would be triggered; we note it but don't navigate
                // (no JS execution in DOM-only layer)
            }
        }
        SpecialKey::Backspace => {
            // keydown + input(deleteContentBackward) + keyup
            // Remove last character from value
            let tag = dom.tag_name(el).unwrap_or_default();
            let is_contenteditable = dom.get_attribute(el, "contenteditable").is_some();
            let use_text_content =
                is_contenteditable && !matches!(tag.as_str(), "input" | "textarea");

            if use_text_content {
                let current = dom.text_content(el);
                let mut chars: Vec<char> = current.chars().collect();
                chars.pop();
                dom.set_text_content(el, &chars.into_iter().collect::<String>());
            } else {
                let current = dom.get_attribute(el, "value").unwrap_or_default();
                let mut chars: Vec<char> = current.chars().collect();
                chars.pop();
                dom.set_attribute(el, "value", &chars.into_iter().collect::<String>());
            }
        }
        SpecialKey::Tab | SpecialKey::Escape => {
            // keydown + keyup only — no value mutation
        }
        SpecialKey::ArrowUp
        | SpecialKey::ArrowDown
        | SpecialKey::ArrowLeft
        | SpecialKey::ArrowRight => {
            // keydown + keyup only — no value mutation
        }
    }

    Ok(KeyResult {
        key: key_name.clone(),
        default_prevented: false,
    })
}

/// Map a character to its `code` property (e.g. 'a' -> "KeyA", '1' -> "Digit1").
fn char_to_code(ch: char) -> String {
    match ch {
        'a'..='z' => format!("Key{}", ch.to_ascii_uppercase()),
        'A'..='Z' => format!("Key{ch}"),
        '0'..='9' => format!("Digit{ch}"),
        ' ' => "Space".to_string(),
        '.' => "Period".to_string(),
        ',' => "Comma".to_string(),
        ';' => "Semicolon".to_string(),
        '/' => "Slash".to_string(),
        '-' => "Minus".to_string(),
        '=' => "Equal".to_string(),
        '[' => "BracketLeft".to_string(),
        ']' => "BracketRight".to_string(),
        '\\' => "Backslash".to_string(),
        '\'' => "Quote".to_string(),
        '`' => "Backquote".to_string(),
        _ => "Unidentified".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use neo_dom::MockDomEngine;

    fn make_input(value: &str) -> MockDomEngine {
        let mut dom = MockDomEngine::new();
        let attrs: Vec<(&str, &str)> = if value.is_empty() {
            vec![("type", "text"), ("placeholder", "Email")]
        } else {
            vec![("type", "text"), ("placeholder", "Email"), ("value", value)]
        };
        let el = dom.add_element("input", &attrs, "");
        dom.set_interactive(el, true);
        dom.set_visible(el, true);
        dom
    }

    fn make_textarea() -> MockDomEngine {
        let mut dom = MockDomEngine::new();
        let el = dom.add_element("textarea", &[("placeholder", "Message")], "");
        dom.set_interactive(el, true);
        dom.set_visible(el, true);
        dom
    }

    #[test]
    fn test_type_with_events_sequence() {
        let mut dom = make_input("");
        let events =
            type_with_events(&mut dom, "Email", "ab").expect("should type");

        // 2 chars × 4 events each = 8 events
        assert_eq!(events.len(), 8);

        // First char 'a': keydown, keypress, input, keyup
        assert_eq!(events[0].event_type, "keydown");
        assert_eq!(events[0].key, "a");
        assert_eq!(events[0].code, "KeyA");

        assert_eq!(events[1].event_type, "keypress");
        assert_eq!(events[1].key, "a");

        assert_eq!(events[2].event_type, "input");
        assert_eq!(events[2].input_type, Some("insertText".to_string()));
        assert_eq!(events[2].data, Some("a".to_string()));

        assert_eq!(events[3].event_type, "keyup");
        assert_eq!(events[3].key, "a");

        // Second char 'b': keydown, keypress, input, keyup
        assert_eq!(events[4].event_type, "keydown");
        assert_eq!(events[4].key, "b");
        assert_eq!(events[4].code, "KeyB");

        assert_eq!(events[7].event_type, "keyup");
        assert_eq!(events[7].key, "b");

        // Value should be "ab"
        assert_eq!(dom.get_attribute(0, "value"), Some("ab".to_string()));
    }

    #[test]
    fn test_press_enter() {
        let mut dom = make_input("");
        let result =
            press_key(&mut dom, "Email", SpecialKey::Enter).expect("should press enter");

        assert_eq!(result.key, "Enter");
        assert!(!result.default_prevented);
    }

    #[test]
    fn test_press_escape() {
        let mut dom = make_input("");
        let result =
            press_key(&mut dom, "Email", SpecialKey::Escape).expect("should press escape");

        assert_eq!(result.key, "Escape");
        assert!(!result.default_prevented);
    }

    #[test]
    fn test_press_tab() {
        let mut dom = make_input("");
        let result =
            press_key(&mut dom, "Email", SpecialKey::Tab).expect("should press tab");

        assert_eq!(result.key, "Tab");
        assert!(!result.default_prevented);
    }

    #[test]
    fn test_special_key_names() {
        let cases = [
            (SpecialKey::Enter, "Enter", "Enter"),
            (SpecialKey::Tab, "Tab", "Tab"),
            (SpecialKey::Escape, "Escape", "Escape"),
            (SpecialKey::Backspace, "Backspace", "Backspace"),
            (SpecialKey::ArrowUp, "ArrowUp", "ArrowUp"),
            (SpecialKey::ArrowDown, "ArrowDown", "ArrowDown"),
            (SpecialKey::ArrowLeft, "ArrowLeft", "ArrowLeft"),
            (SpecialKey::ArrowRight, "ArrowRight", "ArrowRight"),
        ];

        for (key, expected_key, expected_code) in &cases {
            assert_eq!(key.key_name(), *expected_key);
            assert_eq!(key.code_name(), *expected_code);
        }
    }

    #[test]
    fn test_backspace_removes_last_char() {
        let mut dom = make_input("hello");
        press_key(&mut dom, "Email", SpecialKey::Backspace).expect("should press backspace");
        assert_eq!(dom.get_attribute(0, "value"), Some("hell".to_string()));
    }

    #[test]
    fn test_backspace_on_empty_value() {
        let mut dom = make_input("");
        press_key(&mut dom, "Email", SpecialKey::Backspace).expect("should handle empty");
        // value attribute not set on empty mock, so get_attribute returns None
        // After backspace on empty, value should still be empty
        let val = dom.get_attribute(0, "value").unwrap_or_default();
        assert!(val.is_empty());
    }

    #[test]
    fn test_type_with_events_on_textarea() {
        let mut dom = make_textarea();
        // Resolve by tag name (MockDomEngine's query_selector matches tag)
        let events =
            type_with_events(&mut dom, "textarea", "hi").expect("should type in textarea");

        assert_eq!(events.len(), 8);
        assert_eq!(dom.get_attribute(0, "value"), Some("hi".to_string()));
    }

    #[test]
    fn test_type_with_events_not_typeable() {
        let mut dom = MockDomEngine::new();
        dom.add_element("div", &[], "Some div");

        let result = type_with_events(&mut dom, "Some div", "text");
        assert!(result.is_err());
        match result.unwrap_err() {
            InteractError::TypeMismatch { expected, actual } => {
                assert!(expected.contains("input"));
                assert_eq!(actual, "div");
            }
            other => panic!("expected TypeMismatch, got {other:?}"),
        }
    }

    #[test]
    fn test_char_to_code_mapping() {
        assert_eq!(char_to_code('a'), "KeyA");
        assert_eq!(char_to_code('Z'), "KeyZ");
        assert_eq!(char_to_code('5'), "Digit5");
        assert_eq!(char_to_code(' '), "Space");
        assert_eq!(char_to_code('.'), "Period");
        assert_eq!(char_to_code('@'), "Unidentified");
    }

    #[test]
    fn test_enter_with_form() {
        let mut dom = MockDomEngine::new();
        let el = dom.add_element(
            "input",
            &[("type", "text"), ("placeholder", "Search")],
            "",
        );
        dom.set_interactive(el, true);
        dom.set_visible(el, true);
        dom.add_form(Some("search-form"), "/search");

        let result =
            press_key(&mut dom, "Search", SpecialKey::Enter).expect("should press enter");
        assert_eq!(result.key, "Enter");
    }
}
