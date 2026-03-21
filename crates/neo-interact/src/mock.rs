//! Mock interactor for testing AI agents without a real DOM.
//!
//! Records all interactions and returns configurable results.

use std::collections::HashMap;

use crate::{ClickResult, InteractError, Interactor, ScrollDirection, SubmitResult};

/// Recorded interaction from the mock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordedAction {
    Click(String),
    TypeText {
        target: String,
        text: String,
        clear: bool,
    },
    FillForm(HashMap<String, String>),
    Select {
        target: String,
        value: String,
    },
    Check {
        target: String,
        checked: bool,
    },
    Submit(Option<String>),
    Scroll {
        direction: ScrollDirection,
        amount: u32,
    },
}

/// Mock interactor that records calls and returns preset results.
pub struct MockInteractor {
    /// All recorded actions in order.
    pub actions: Vec<RecordedAction>,
    /// Default click result.
    pub click_result: ClickResult,
    /// Default submit result.
    pub submit_result: SubmitResult,
    /// Default scroll count.
    pub scroll_count: usize,
}

impl MockInteractor {
    /// Create a new mock with default results.
    pub fn new() -> Self {
        Self {
            actions: Vec::new(),
            click_result: ClickResult::NoEffect,
            submit_result: SubmitResult::NoAction,
            scroll_count: 0,
        }
    }
}

impl Default for MockInteractor {
    fn default() -> Self {
        Self::new()
    }
}

impl Interactor for MockInteractor {
    fn click(&mut self, target: &str) -> Result<ClickResult, InteractError> {
        self.actions.push(RecordedAction::Click(target.to_string()));
        Ok(self.click_result.clone())
    }

    fn type_text(&mut self, target: &str, text: &str, clear: bool) -> Result<(), InteractError> {
        self.actions.push(RecordedAction::TypeText {
            target: target.to_string(),
            text: text.to_string(),
            clear,
        });
        Ok(())
    }

    fn fill_form(&mut self, fields: &HashMap<String, String>) -> Result<(), InteractError> {
        self.actions.push(RecordedAction::FillForm(fields.clone()));
        Ok(())
    }

    fn select(&mut self, target: &str, value: &str) -> Result<(), InteractError> {
        self.actions.push(RecordedAction::Select {
            target: target.to_string(),
            value: value.to_string(),
        });
        Ok(())
    }

    fn check(&mut self, target: &str, checked: bool) -> Result<(), InteractError> {
        self.actions.push(RecordedAction::Check {
            target: target.to_string(),
            checked,
        });
        Ok(())
    }

    fn submit(&mut self, target: Option<&str>) -> Result<SubmitResult, InteractError> {
        self.actions
            .push(RecordedAction::Submit(target.map(String::from)));
        Ok(self.submit_result.clone())
    }

    fn scroll(&mut self, direction: ScrollDirection, amount: u32) -> Result<usize, InteractError> {
        self.actions
            .push(RecordedAction::Scroll { direction, amount });
        Ok(self.scroll_count)
    }
}
