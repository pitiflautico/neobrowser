//! neo-interact — translates AI intents into DOM operations.
//!
//! AI emits intents like "click Submit" or "type email in the email field".
//! This crate resolves targets, validates interactivity, and performs
//! the corresponding DOM mutations via the `DomEngine` trait.

mod checkbox;
mod click;
mod doubleclick;
mod forms;
mod hover;
mod keyboard;
mod mock;
mod popups;
mod resolve;
mod right_click;
mod scroll;
mod select;
mod type_text;
mod upload;

pub use checkbox::check;
pub use click::click;
pub use doubleclick::doubleclick;
pub use forms::{collect_form_data, detect_csrf, fill_form, submit, submit_full};
pub use hover::{hover, HoverResult};
pub use right_click::{right_click, ContextMenuItem, RightClickEvent, RightClickResult, RIGHT_CLICK_EVENTS};
pub use mock::MockInteractor;
pub use popups::{detect_modal, dismiss_consent};
pub use resolve::{resolve, ResolveStrategy};
pub use scroll::{scroll, scroll_until_stable};
pub use select::select;
pub use type_text::{type_slowly, type_text};
pub use keyboard::{press_key, type_with_events, KeyboardEvent, KeyResult, SpecialKey};
pub use upload::{build_multipart, detect_content_type, set_file, FileUpload, MultipartField, MultipartValue};

use neo_dom::DomEngine;
use std::sync::{Arc, Mutex};

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

/// Extended submit result with CSRF token and collected form data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmitOutcome {
    /// Navigation/AJAX result.
    pub result: SubmitResult,
    /// CSRF token if detected (auto-injected into form_data).
    pub csrf: Option<CsrfToken>,
    /// All form data (name=value pairs), including hidden inputs and CSRF.
    pub form_data: HashMap<String, String>,
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

    /// Type text character by character. Returns char count typed.
    fn type_slowly(
        &mut self,
        target: &str,
        text: &str,
        delay_ms: u64,
    ) -> Result<usize, InteractError>;

    /// Scroll in a direction. Returns new visible element count.
    fn scroll(&mut self, direction: ScrollDirection, amount: u32) -> Result<usize, InteractError>;

    /// Scroll until no new content loads, or `max_scrolls` reached.
    /// Returns the last element count.
    fn scroll_until_stable(&mut self, max_scrolls: u32) -> Result<usize, InteractError>;

    /// Detect a modal/dialog on the page. Returns its element ID if found.
    fn detect_modal(&self) -> Option<neo_dom::ElementId>;

    /// Try to dismiss a cookie-consent banner. Returns true if dismissed.
    fn dismiss_consent(&mut self) -> bool;
}

/// Real interactor that delegates to the free functions using a shared DOM.
///
/// The DOM is shared with [`NeoSession`] via `Arc<Mutex<...>>` so that
/// interactions mutate the same DOM the session reads from.
pub struct DomInteractor {
    dom: Arc<Mutex<Box<dyn DomEngine>>>,
}

impl DomInteractor {
    /// Create a new interactor wrapping a shared DOM reference.
    pub fn new(dom: Arc<Mutex<Box<dyn DomEngine>>>) -> Self {
        Self { dom }
    }
}

impl Interactor for DomInteractor {
    fn click(&mut self, target: &str) -> Result<ClickResult, InteractError> {
        let mut dom = self.dom.lock().expect("dom lock poisoned");
        click::click(dom.as_mut(), target)
    }

    fn type_text(&mut self, target: &str, text: &str, clear: bool) -> Result<(), InteractError> {
        let mut dom = self.dom.lock().expect("dom lock poisoned");
        type_text::type_text(dom.as_mut(), target, text, clear)
    }

    fn fill_form(&mut self, fields: &HashMap<String, String>) -> Result<(), InteractError> {
        let mut dom = self.dom.lock().expect("dom lock poisoned");
        forms::fill_form(dom.as_mut(), fields)
    }

    fn select(&mut self, target: &str, value: &str) -> Result<(), InteractError> {
        let mut dom = self.dom.lock().expect("dom lock poisoned");
        select::select(dom.as_mut(), target, value)
    }

    fn check(&mut self, target: &str, checked: bool) -> Result<(), InteractError> {
        let mut dom = self.dom.lock().expect("dom lock poisoned");
        checkbox::check(dom.as_mut(), target, checked)
    }

    fn submit(&mut self, target: Option<&str>) -> Result<SubmitResult, InteractError> {
        let mut dom = self.dom.lock().expect("dom lock poisoned");
        forms::submit(dom.as_mut(), target)
    }

    fn type_slowly(
        &mut self,
        target: &str,
        text: &str,
        delay_ms: u64,
    ) -> Result<usize, InteractError> {
        let mut dom = self.dom.lock().expect("dom lock poisoned");
        type_text::type_slowly(dom.as_mut(), target, text, delay_ms)
    }

    fn scroll(&mut self, direction: ScrollDirection, amount: u32) -> Result<usize, InteractError> {
        let mut dom = self.dom.lock().expect("dom lock poisoned");
        scroll::scroll(dom.as_mut(), direction, amount)
    }

    fn scroll_until_stable(&mut self, max_scrolls: u32) -> Result<usize, InteractError> {
        let mut dom = self.dom.lock().expect("dom lock poisoned");
        scroll::scroll_until_stable(dom.as_mut(), max_scrolls)
    }

    fn detect_modal(&self) -> Option<neo_dom::ElementId> {
        let dom = self.dom.lock().expect("dom lock poisoned");
        popups::detect_modal(dom.as_ref())
    }

    fn dismiss_consent(&mut self) -> bool {
        let mut dom = self.dom.lock().expect("dom lock poisoned");
        popups::dismiss_consent(dom.as_mut())
    }
}
