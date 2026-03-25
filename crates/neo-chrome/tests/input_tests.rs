//! Unit tests for input.rs — key parsing, modifiers, dialog actions.
//!
//! These tests exercise pure functions without a live Chrome instance.

use neo_chrome::input::{
    parse_key_combo, DialogAction, MOD_ALT, MOD_CTRL, MOD_META, MOD_SHIFT,
};

// ─── Key parsing ───

#[test]
fn parse_single_key_enter() {
    let combo = parse_key_combo("Enter");
    assert_eq!(combo.key, "Enter");
    assert_eq!(combo.code, "Enter");
    assert_eq!(combo.key_code, 13);
    assert_eq!(combo.modifiers, 0);
}

#[test]
fn parse_single_key_tab() {
    let combo = parse_key_combo("Tab");
    assert_eq!(combo.key, "Tab");
    assert_eq!(combo.key_code, 9);
    assert_eq!(combo.modifiers, 0);
}

#[test]
fn parse_single_letter() {
    let combo = parse_key_combo("a");
    assert_eq!(combo.key, "a");
    assert_eq!(combo.code, "KeyA");
    assert_eq!(combo.key_code, 65); // 'A' as u32
    assert_eq!(combo.modifiers, 0);
}

#[test]
fn parse_control_a() {
    let combo = parse_key_combo("Control+a");
    assert_eq!(combo.key, "a");
    assert_eq!(combo.code, "KeyA");
    assert_eq!(combo.modifiers, MOD_CTRL);
}

#[test]
fn parse_control_shift_a() {
    let combo = parse_key_combo("Control+Shift+A");
    assert_eq!(combo.key, "A");
    assert_eq!(combo.code, "KeyA");
    assert_eq!(combo.modifiers, MOD_CTRL | MOD_SHIFT);
}

#[test]
fn parse_ctrl_alias() {
    let combo = parse_key_combo("Ctrl+c");
    assert_eq!(combo.modifiers, MOD_CTRL);
    assert_eq!(combo.key, "c");
}

#[test]
fn parse_meta_shift_r() {
    let combo = parse_key_combo("Meta+Shift+R");
    assert_eq!(combo.modifiers, MOD_META | MOD_SHIFT);
    assert_eq!(combo.key, "R");
}

#[test]
fn parse_alt_key() {
    let combo = parse_key_combo("Alt+Tab");
    assert_eq!(combo.modifiers, MOD_ALT);
    assert_eq!(combo.key, "Tab");
    assert_eq!(combo.key_code, 9);
}

// ─── Modifier bitmask ───

#[test]
fn modifier_bitmask_values() {
    assert_eq!(MOD_ALT, 1);
    assert_eq!(MOD_CTRL, 2);
    assert_eq!(MOD_META, 4);
    assert_eq!(MOD_SHIFT, 8);
}

#[test]
fn modifier_bitmask_combination() {
    // All modifiers combined
    let all = MOD_ALT | MOD_CTRL | MOD_META | MOD_SHIFT;
    assert_eq!(all, 15);
    // Individual bits
    assert!(all & MOD_ALT != 0);
    assert!(all & MOD_CTRL != 0);
    assert!(all & MOD_META != 0);
    assert!(all & MOD_SHIFT != 0);
}

// ─── Dialog action ───

#[test]
fn dialog_action_accept() {
    let action = DialogAction::Accept;
    assert_eq!(action, DialogAction::Accept);
    assert_ne!(action, DialogAction::Dismiss);
}

#[test]
fn dialog_action_dismiss() {
    let action = DialogAction::Dismiss;
    assert_eq!(action, DialogAction::Dismiss);
    assert_ne!(action, DialogAction::Accept);
}

// ─── Special keys ───

#[test]
fn parse_escape() {
    let combo = parse_key_combo("Escape");
    assert_eq!(combo.key, "Escape");
    assert_eq!(combo.key_code, 27);
}

#[test]
fn parse_backspace() {
    let combo = parse_key_combo("Backspace");
    assert_eq!(combo.key, "Backspace");
    assert_eq!(combo.key_code, 8);
}

#[test]
fn parse_arrow_keys() {
    let up = parse_key_combo("ArrowUp");
    assert_eq!(up.key, "ArrowUp");
    assert_eq!(up.key_code, 38);

    let down = parse_key_combo("ArrowDown");
    assert_eq!(down.key, "ArrowDown");
    assert_eq!(down.key_code, 40);
}

#[test]
fn parse_f_key() {
    let f5 = parse_key_combo("F5");
    assert_eq!(f5.key, "F5");
    assert_eq!(f5.key_code, 116);
}

#[test]
fn parse_digit() {
    let combo = parse_key_combo("5");
    assert_eq!(combo.key, "5");
    assert_eq!(combo.code, "Digit5");
}

#[test]
fn parse_space() {
    let combo = parse_key_combo("Space");
    assert_eq!(combo.key, " ");
    assert_eq!(combo.code, "Space");
    assert_eq!(combo.key_code, 32);
}
