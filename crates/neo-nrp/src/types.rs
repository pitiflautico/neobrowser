//! NRP protocol types — wire format for the NeoRender Protocol v0.1.
//!
//! All types are serializable and form the public API contract.
//! See `docs/PDR-NRP.md` for the full protocol specification.

use serde::{Deserialize, Serialize};

// ─── Identity ───

/// Page-level metadata returned by navigation and info commands.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageInfo {
    /// Monotonically increasing page identifier.
    pub page_id: u64,
    /// Incremented on full document replacement (not innerHTML).
    pub document_epoch: u64,
    /// Final URL after redirects.
    pub url: String,
    /// Document title.
    pub title: String,
    /// HTTP status code.
    pub status: u16,
    /// Current page lifecycle state.
    pub page_state: NrpPageState,
}

/// Page lifecycle states (NRP-specific, distinct from neo-types::PageState).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NrpPageState {
    /// No document loaded.
    Idle,
    /// Document is loading (network active).
    Loading,
    /// DOM is ready but subresources still loading.
    Interactive,
    /// Page is fully loaded and stable.
    Settled,
    /// Navigation or load failed.
    Failed,
}

// ─── Target (typed union) ───

/// Typed element targeting — resolved to node_id before dispatch.
///
/// Core Interact commands only accept `NodeId`. Other variants are
/// resolved by the `TargetResolver` (agent helper layer).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "by", content = "value")]
pub enum Target {
    /// Direct node reference (stable within a document_epoch).
    #[serde(rename = "node_id")]
    NodeId(String),
    /// CSS selector.
    #[serde(rename = "css")]
    Css(String),
    /// Text content match.
    #[serde(rename = "text")]
    Text {
        /// Text to match against element name/content.
        value: String,
        /// If true, requires exact match. Default: false (substring).
        exact: Option<bool>,
    },
    /// ARIA role with optional accessible name.
    #[serde(rename = "role")]
    Role {
        /// ARIA role (e.g. "button", "textbox", "link").
        value: String,
        /// Optional accessible name filter.
        name: Option<String>,
    },
    /// Label text — finds the associated input/control.
    #[serde(rename = "label")]
    Label(String),
}

// ─── ActionResult (uniform) ───

/// Uniform result from any interaction command.
///
/// Combines outcome classification with page state and optional
/// detail fields. Issue #3: struct instead of heavy enum.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionResult {
    /// What happened.
    pub outcome: ActionOutcomeKind,
    /// Current page ID after the action.
    pub page_id: u64,
    /// Current document epoch after the action.
    pub document_epoch: u64,
    /// Whether the DOM was mutated.
    pub dom_changed: bool,
    /// Value of the target element after the action (for inputs).
    pub value_after: Option<String>,
    /// Selection range after typing (start, end).
    pub selection_after: Option<(usize, usize)>,
    /// Navigation info if the action triggered navigation.
    pub navigation: Option<NavigationInfo>,
    /// Number of DOM mutations observed.
    pub mutations: Option<u32>,
    /// Focus change if focus moved.
    pub focus_change: Option<FocusChange>,
    /// Error message if outcome is Error.
    pub error: Option<String>,
}

/// Classification of what happened after an interaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionOutcomeKind {
    /// The action had no observable effect.
    NoEffect,
    /// DOM was mutated.
    DomChanged,
    /// Full HTTP navigation occurred.
    HttpNavigation,
    /// SPA-style navigation (pushState/replaceState).
    SpaNavigation,
    /// Form was submitted.
    FormSubmitted,
    /// Client-side validation blocked the action.
    ValidationBlocked,
    /// A dialog (alert/confirm/prompt) was opened.
    DialogOpened,
    /// A dialog was closed.
    DialogClosed,
    /// Focus moved to a different element.
    FocusMoved,
    /// Checkbox was toggled.
    CheckboxToggled,
    /// Radio button was selected.
    RadioSelected,
    /// An error occurred.
    Error,
}

/// Details about a navigation triggered by an action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NavigationInfo {
    /// Destination URL.
    pub url: String,
    /// HTTP method used.
    pub method: String,
    /// HTTP status code (None if SPA navigation).
    pub status: Option<u16>,
}

/// Details about focus movement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FocusChange {
    /// Node ID that lost focus (None if nothing was focused).
    pub from: Option<String>,
    /// Node ID that gained focus.
    pub to: String,
}

// ─── SemanticNode ───

/// A node in the semantic tree — the AI's view of a DOM element.
///
/// Heuristic roles and names. Not a real ARIA computation.
/// See PDR limitations section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticNode {
    /// Stable identifier within (session_id, page_id, document_epoch).
    pub node_id: String,
    /// ARIA role (computed heuristically from tag + attributes).
    pub role: String,
    /// Accessible name (aria-label > label > text > placeholder > alt).
    pub name: String,
    /// Current value (for inputs, selects, textareas).
    pub value: Option<String>,
    /// Description (aria-describedby, title).
    pub description: Option<String>,
    /// HTML tag name (lowercase).
    pub tag: String,
    /// Interaction-relevant properties.
    pub properties: NodeProperties,
    /// HTML-specific metadata (split from properties per issue #6).
    pub html_metadata: HtmlMetadata,
    /// Available actions on this node.
    pub actions: Vec<String>,
    /// Child nodes.
    pub children: Vec<SemanticNode>,
}

/// Interaction-relevant boolean properties of a semantic node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeProperties {
    /// Element is disabled (not interactive).
    pub disabled: bool,
    /// Element is required (form validation).
    pub required: bool,
    /// Checkbox/radio checked state.
    pub checked: Option<bool>,
    /// Option/tab selected state.
    pub selected: Option<bool>,
    /// Details/accordion expanded state.
    pub expanded: Option<bool>,
    /// Element currently has focus.
    pub focused: bool,
    /// Element is contenteditable.
    pub editable: bool,
    /// Element is readonly.
    pub readonly: bool,
}

/// HTML-specific metadata separated from semantic properties.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HtmlMetadata {
    /// Input type attribute (text, email, password, etc.).
    pub input_type: Option<String>,
    /// Link href.
    pub href: Option<String>,
    /// Form action URL.
    pub action: Option<String>,
    /// Form method (GET/POST).
    pub method: Option<String>,
    /// Element name attribute.
    pub name: Option<String>,
    /// Input placeholder text.
    pub placeholder: Option<String>,
    /// Associated form ID.
    pub form_id: Option<String>,
}

// ─── Protocol messages ───

/// JSON-RPC-style request from client to NRP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NrpRequest {
    /// Request identifier for correlation.
    pub id: u64,
    /// Domain-qualified method name (e.g. "Page.navigate").
    pub method: String,
    /// Method parameters.
    pub params: serde_json::Value,
}

/// JSON-RPC-style response from NRP server to client.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NrpResponse {
    /// Matches the request id.
    pub id: u64,
    /// Result on success.
    pub result: Option<serde_json::Value>,
    /// Error on failure.
    pub error: Option<NrpError>,
}

/// Protocol error with code and message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NrpError {
    /// Numeric error code (negative = protocol error, positive = domain error).
    pub code: i32,
    /// Human-readable error description.
    pub message: String,
}

/// Server-initiated event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NrpEvent {
    /// Domain-qualified event name (e.g. "Page.navigated").
    pub method: String,
    /// Event payload.
    pub params: serde_json::Value,
    /// Monotonically increasing sequence ID for ordering.
    pub sequence_id: u64,
}

// ─── Well-known error codes ───

impl NrpError {
    /// Standard JSON-RPC method not found.
    pub const METHOD_NOT_FOUND: i32 = -32601;
    /// Standard JSON-RPC invalid params.
    pub const INVALID_PARAMS: i32 = -32602;
    /// Target element not found.
    pub const TARGET_NOT_FOUND: i32 = -32001;
    /// Navigation failed.
    pub const NAVIGATION_FAILED: i32 = -32002;
    /// Engine error.
    pub const ENGINE_ERROR: i32 = -32003;
}

impl NrpResponse {
    /// Create a success response.
    pub fn ok(id: u64, result: serde_json::Value) -> Self {
        Self {
            id,
            result: Some(result),
            error: None,
        }
    }

    /// Create an error response.
    pub fn err(id: u64, code: i32, message: impl Into<String>) -> Self {
        Self {
            id,
            result: None,
            error: Some(NrpError {
                code,
                message: message.into(),
            }),
        }
    }
}
