//! NeoRender shared types — data structures used across all crates.
//! NO logic here. Only types, enums, and trait-less structs.

use serde::{Deserialize, Serialize};

/// HTTP response from any request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: std::collections::HashMap<String, String>,
    pub body: String,
    pub url: String,
    pub duration_ms: u64,
}

/// A cookie with all metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub expires: Option<i64>,
    pub http_only: bool,
    pub secure: bool,
    pub same_site: Option<String>,
}

/// Page state in the navigation lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PageState {
    Idle,
    Navigating,
    Loading,
    Interactive,
    Hydrated,
    Settled,
    Complete,
    Blocked,
    Failed,
}

/// Result of a page navigation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageResult {
    pub url: String,
    pub title: String,
    pub state: PageState,
    pub render_ms: u64,
    pub links: usize,
    pub forms: usize,
    pub inputs: usize,
    pub buttons: usize,
    pub scripts: usize,
    pub errors: Vec<String>,
    /// URLs visited during redirect chain (empty if no redirects).
    #[serde(default)]
    pub redirect_chain: Vec<String>,
}

/// Entry in the network log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkLogEntry {
    pub url: String,
    pub method: String,
    pub status: u16,
    pub duration_ms: u64,
    pub kind: String,
    pub initiator: String,
}

/// A trace entry — one action or event in the execution log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEntry {
    pub timestamp_ms: u64,
    pub action: String,
    pub target: Option<String>,
    pub state_before: Option<PageState>,
    pub state_after: Option<PageState>,
    pub duration_ms: u64,
    pub network_requests: usize,
    pub dom_mutations: usize,
    pub error: Option<String>,
    pub metadata: serde_json::Value,
}

/// Link extracted from a page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Link {
    pub text: String,
    pub href: String,
    pub rel: Option<String>,
}

/// Form extracted from a page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Form {
    pub id: Option<String>,
    pub action: String,
    pub method: String,
    pub fields: Vec<FormField>,
}

/// A form field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormField {
    pub name: String,
    pub field_type: String,
    pub value: Option<String>,
    pub required: bool,
    pub placeholder: Option<String>,
    pub label: Option<String>,
}

/// DOM mutation detected by observer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomMutation {
    pub mutation_type: String,
    pub target_selector: String,
    pub added_nodes: usize,
    pub removed_nodes: usize,
}
