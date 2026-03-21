//! neo-interact — translates AI intents into DOM operations.
//!
//! AI emits intents like "click Submit" or "type email in the email field".
//! This crate resolves targets, validates interactivity, and performs
//! the corresponding DOM mutations via the `DomEngine` trait.

mod click;
mod forms;
mod mock;
mod resolve;
mod scroll;
mod type_text;

pub use click::click;
pub use forms::{detect_csrf, fill_form, submit};
pub use mock::MockInteractor;
pub use resolve::{resolve, ResolveStrategy};
pub use scroll::scroll;
pub use type_text::type_text;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

/// Errors from interaction operations.
#[derive(Debug, Error)]
pub enum InteractError {
    /// Target element could not be found.
    #[error("element not found: {target}")]
    NotFound {
        target: String,
        suggestions: Vec<String>,
    },

    /// Element exists but is not interactive.
    #[error("element not interactive: {0}")]
    NotInteractive(String),

    /// Element type mismatch (e.g., typing into a div without contenteditable).
    #[error("type mismatch: expected {expected}, got {actual}")]
    TypeMismatch { expected: String, actual: String },

    /// DOM engine error.
    #[error("dom error: {0}")]
    Dom(#[from] neo_dom::DomError),
}

/// Result of clicking an element.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClickResult {
    /// Click caused a navigation (contains target URL).
    Navigation(String),
    /// Click caused DOM changes (contains mutation count).
    DomChanged(usize),
    /// Click had no observable effect.
    NoEffect,
}

/// Result of submitting a form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubmitResult {
    /// Form submit navigates (contains URL).
    Navigation(String),
    /// Form submit triggers AJAX (contains action URL).
    AjaxResponse(String),
    /// No form found or no action.
    NoAction,
}

/// Scroll direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScrollDirection {
    Down,
    Up,
}

/// Detected CSRF token from a form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsrfToken {
    /// The field name (e.g., "_token", "csrf_token").
    pub name: String,
    /// The token value.
    pub value: String,
}

/// Interaction trait — the high-level API for AI agents.
///
/// Wraps a `DomEngine` and provides intent-based operations.
pub trait Interactor {
    /// Click an element by target (CSS selector, text, aria-label).
    fn click(&mut self, target: &str) -> Result<ClickResult, InteractError>;

    /// Type text into an element. Clears existing if `clear` is true.
    fn type_text(&mut self, target: &str, text: &str, clear: bool) -> Result<(), InteractError>;

    /// Fill multiple form fields at once.
    fn fill_form(&mut self, fields: &HashMap<String, String>) -> Result<(), InteractError>;

    /// Select an option in a dropdown.
    fn select(&mut self, target: &str, value: &str) -> Result<(), InteractError>;

    /// Check/uncheck a checkbox.
    fn check(&mut self, target: &str, checked: bool) -> Result<(), InteractError>;

    /// Submit the form containing target element.
    fn submit(&mut self, target: Option<&str>) -> Result<SubmitResult, InteractError>;

    /// Scroll in a direction. Returns new visible element count.
    fn scroll(&mut self, direction: ScrollDirection, amount: u32) -> Result<usize, InteractError>;
}
