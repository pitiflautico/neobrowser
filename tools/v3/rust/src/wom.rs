//! WOM: Web Object Model — AI-native page representation.
//!
//! Not HTML. Not markdown. Not accessibility tree dump.
//! A structured representation optimized for LLM decision-making:
//! - Stable node IDs (survive DOM mutations)
//! - Affordances first-class (what CAN you do, not just what exists)
//! - Importance scoring (what MATTERS)
//! - Compact serialization (16x compression over raw DOM)
//! - Delta-native (revision + incremental updates)

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::semantic;
use markup5ever_rcdom::Handle;

/// Safe truncation at a char boundary (stable replacement for str::floor_char_boundary).
fn truncate_at(s: &str, max: usize) -> &str {
    if max >= s.len() { return s; }
    let mut i = max;
    while i > 0 && !s.is_char_boundary(i) { i -= 1; }
    &s[..i]
}

// ─── Core Types ───

pub type NodeId = String; // e.g. "n_001", "fld_email", "btn_submit"
pub type Revision = u64;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WomDocument {
    pub session: SessionInfo,
    pub page: PageInfo,
    pub goal_surface: GoalSurface,
    pub nodes: Vec<WomNode>,
    pub actions: Vec<WomAction>,
    pub content: ContentBlock,
    pub observability: Observability,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta: Option<DeltaBlock>,
    pub compression: CompressionStats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub page_id: String,
    pub revision: Revision,
    pub timestamp_ms: u64,
    pub mode: String, // "light" | "chrome"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageInfo {
    pub url: String,
    pub origin: String,
    pub title: String,
    pub page_class: String,
    pub load_state: String,
    pub language: String,
    pub is_https: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalSurface {
    pub primary_intents: Vec<IntentInfo>,
    pub warnings: Vec<Warning>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentInfo {
    pub intent: String,
    pub confidence: f32,
    pub targets: Vec<NodeId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Warning {
    pub kind: String,
    pub node_id: Option<NodeId>,
    pub severity: String, // "low" | "medium" | "high"
    pub message: String,
}

// ─── Nodes ───

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WomNode {
    pub id: NodeId,
    pub kind: String,     // "element", "text"
    pub role: String,     // "textbox", "button", "link", "heading", etc.
    pub name: String,     // Human-readable label
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    pub state: NodeState,
    pub capabilities: Vec<String>, // ["focus", "type", "click", "submit"]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locator: Option<Locator>,
    pub importance: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeState {
    pub visible: bool,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focused: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invalid: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Locator {
    pub semantic_path: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
}

// ─── Actions ───

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WomAction {
    pub action_id: String,
    pub kind: String,     // "type", "click", "submit", "navigate", "scroll"
    pub target: NodeId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args_hint: Option<String>, // e.g. "text to type"
    pub preconditions: Vec<String>,
    pub expected_effects: Vec<String>,
    pub risk: String,     // "low", "medium", "high"
}

// ─── Content ───

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentBlock {
    pub headings: Vec<TextItem>,
    pub paragraphs: Vec<TextItem>,
    pub links: Vec<LinkItem>,
    pub forms: Vec<FormInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextItem {
    pub id: NodeId,
    pub text: String,
    pub importance: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkItem {
    pub id: NodeId,
    pub text: String,
    pub href: String,
    pub importance: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormInfo {
    pub id: NodeId,
    pub fields: Vec<NodeId>,
    pub submit: Option<NodeId>,
    pub intent: String,
}

// ─── Observability ───

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observability {
    pub dom_node_count: usize,
    pub semantic_node_count: usize,
    pub links_count: usize,
    pub buttons_count: usize,
    pub forms_count: usize,
}

// ─── Delta ───

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeltaBlock {
    pub from_revision: Revision,
    pub summary: String,
    pub ops: Vec<DeltaOp>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum DeltaOp {
    #[serde(rename = "add_node")]
    AddNode { node: WomNode },
    #[serde(rename = "remove_node")]
    RemoveNode { id: NodeId },
    #[serde(rename = "update_node")]
    UpdateNode { id: NodeId, patch: HashMap<String, serde_json::Value> },
    #[serde(rename = "emit_event")]
    EmitEvent { event: String, confidence: f32 },
}

// ─── Compression Stats ───

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionStats {
    pub raw_html_bytes: usize,
    pub wom_bytes: usize,
    pub compression_ratio: f32,
}

// ─── Compact format for fast agent loops ───

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WomCompact {
    pub rev: Revision,
    pub class: String,
    pub focus: Vec<String>,    // "n_001:textbox:Email", "n_002:button:Submit"
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<String>,
    pub next: Vec<String>,     // "type:n_001", "click:n_002"
}

// ─── Builder: DOM → WOM ───

struct WomBuilder {
    nodes: Vec<WomNode>,
    actions: Vec<WomAction>,
    headings: Vec<TextItem>,
    paragraphs: Vec<TextItem>,
    links: Vec<LinkItem>,
    forms: Vec<FormInfo>,
    node_counter: usize,
    action_counter: usize,
    // Track current form context
    current_form_fields: Vec<NodeId>,
    current_form_id: Option<NodeId>,
    current_form_submit: Option<NodeId>,
    current_form_intent: String,
    in_nav: bool,
    in_footer: bool,
}

impl WomBuilder {
    fn new() -> Self {
        Self {
            nodes: Vec::new(),
            actions: Vec::new(),
            headings: Vec::new(),
            paragraphs: Vec::new(),
            links: Vec::new(),
            forms: Vec::new(),
            node_counter: 0,
            action_counter: 0,
            current_form_fields: Vec::new(),
            current_form_id: None,
            current_form_submit: None,
            current_form_intent: String::new(),
            in_nav: false,
            in_footer: false,
        }
    }

    fn next_node_id(&mut self, prefix: &str) -> NodeId {
        self.node_counter += 1;
        format!("{}_{:03}", prefix, self.node_counter)
    }

    /// Read data-wom-id from element, fallback to generated ID
    fn node_id_from_dom(&mut self, handle: &Handle, prefix: &str) -> NodeId {
        if let Some(wom_id) = semantic::get_attr(handle, "data-wom-id") {
            if !wom_id.is_empty() {
                return wom_id;
            }
        }
        self.next_node_id(prefix)
    }

    fn next_action_id(&mut self) -> String {
        self.action_counter += 1;
        format!("a_{:03}", self.action_counter)
    }

    fn walk(&mut self, handle: &Handle, depth: usize) {
        use markup5ever_rcdom::NodeData;

        match &handle.data {
            NodeData::Element { name, .. } => {
                let tag = &name.local;

                if semantic::is_hidden_tag(tag) {
                    return;
                }

                let tag_str = &**tag;

                // Track nav/footer zones
                if tag_str == "nav" {
                    self.in_nav = true;
                    for child in handle.children.borrow().iter() {
                        self.walk(child, depth + 1);
                    }
                    self.in_nav = false;
                    return;
                }
                if tag_str == "footer" {
                    self.in_footer = true;
                    for child in handle.children.borrow().iter() {
                        self.walk(child, depth + 1);
                    }
                    self.in_footer = false;
                    return;
                }

                match tag_str {
                    "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                        let text = semantic::extract_text(handle).trim().to_string();
                        if !text.is_empty() {
                            let nid = self.node_id_from_dom(handle, "h");
                            let level: u8 = tag_str[1..].parse().unwrap_or(1);
                            let importance = 1.0 - (level as f32 - 1.0) * 0.12;
                            self.headings.push(TextItem {
                                id: nid.clone(),
                                text: text.clone(),
                                importance,
                            });
                            self.nodes.push(WomNode {
                                id: nid,
                                kind: "element".into(),
                                role: format!("heading-{level}"),
                                name: text,
                                value: None,
                                state: NodeState { visible: true, enabled: true, focused: None, invalid: None },
                                capabilities: vec![],
                                locator: None,
                                importance,
                            });
                        }
                        return; // Don't recurse into headings
                    }
                    "a" => {
                        let text = semantic::extract_text(handle).trim().to_string();
                        let href = semantic::get_attr(handle, "href").unwrap_or_default();
                        if !text.is_empty() && !self.in_nav && !self.in_footer {
                            let nid = self.node_id_from_dom(handle, "lnk");
                            let importance = if href.starts_with("http") { 0.5 } else { 0.3 };
                            self.links.push(LinkItem {
                                id: nid.clone(),
                                text: text.clone(),
                                href: href.clone(),
                                importance,
                            });
                            self.nodes.push(WomNode {
                                id: nid.clone(),
                                kind: "element".into(),
                                role: "link".into(),
                                name: text.clone(),
                                value: Some(href),
                                state: NodeState { visible: true, enabled: true, focused: None, invalid: None },
                                capabilities: vec!["click".into(), "navigate".into()],
                                locator: None,
                                importance,
                            });
                            let aid = self.next_action_id();
                            self.actions.push(WomAction {
                                action_id: aid,
                                kind: "navigate".into(),
                                target: nid,
                                args_hint: None,
                                preconditions: vec!["visible".into()],
                                expected_effects: vec!["navigation".into()],
                                risk: "low".into(),
                            });
                        }
                        return;
                    }
                    "button" | "summary" => {
                        let text = semantic::extract_text(handle).trim().to_string();
                        if !text.is_empty() && text.len() < 120 && !is_noise_text(&text) {
                            let nid = self.next_node_id("btn");
                            let display = if text.len() > 60 { format!("{}...", truncate_at(&text, 60)) } else { text.clone() };
                            let importance = 0.8;
                            let disabled = semantic::get_attr(handle, "disabled").is_some();

                            self.nodes.push(WomNode {
                                id: nid.clone(),
                                kind: "element".into(),
                                role: "button".into(),
                                name: display.clone(),
                                value: None,
                                state: NodeState {
                                    visible: true,
                                    enabled: !disabled,
                                    focused: None,
                                    invalid: None,
                                },
                                capabilities: vec!["click".into()],
                                locator: Some(Locator {
                                    semantic_path: format!("button[text={display}]"),
                                    aliases: vec![],
                                }),
                                importance,
                            });

                            // Determine action kind
                            let lower = display.to_lowercase();
                            let (kind, risk, effects) = if lower.contains("submit") || lower.contains("send") || lower.contains("enviar") {
                                ("submit", "medium", vec!["form_submitted".into()])
                            } else if lower.contains("sign in") || lower.contains("log in") || lower.contains("iniciar") {
                                ("login", "medium", vec!["session_established".into()])
                            } else if lower.contains("delete") || lower.contains("eliminar") || lower.contains("remove") {
                                ("click", "high", vec!["item_deleted".into()])
                            } else {
                                ("click", "low", vec!["state_changed".into()])
                            };

                            let aid = self.next_action_id();
                            self.actions.push(WomAction {
                                action_id: aid,
                                kind: kind.into(),
                                target: nid.clone(),
                                args_hint: None,
                                preconditions: vec!["visible".into(), "enabled".into()],
                                expected_effects: effects,
                                risk: risk.into(),
                            });

                            // Track as form submit if in form context
                            if kind == "submit" || kind == "login" {
                                self.current_form_submit = Some(nid);
                            }
                        }
                        return;
                    }
                    "input" | "textarea" => {
                        let itype = semantic::get_attr(handle, "type").unwrap_or("text".into());
                        if itype == "hidden" {
                            return;
                        }
                        let placeholder = semantic::get_attr(handle, "placeholder").unwrap_or_default();
                        let aria = semantic::get_attr(handle, "aria-label").unwrap_or_default();
                        let name_attr = semantic::get_attr(handle, "name").unwrap_or_default();
                        let label = if !placeholder.is_empty() {
                            placeholder.clone()
                        } else if !aria.is_empty() {
                            aria.clone()
                        } else {
                            name_attr.clone()
                        };

                        let nid = self.next_node_id("fld");
                        let role = match itype.as_str() {
                            "password" => "password-field",
                            "email" => "email-field",
                            "search" => "search-field",
                            "checkbox" => "checkbox",
                            "radio" => "radio",
                            "file" => "file-input",
                            _ if tag_str == "textarea" => "textarea",
                            _ => "textbox",
                        };

                        let capabilities = match role {
                            "checkbox" | "radio" => vec!["click".into()],
                            "file-input" => vec!["upload".into()],
                            _ => vec!["focus".into(), "type".into(), "clear".into()],
                        };

                        let importance = match role {
                            "password-field" | "email-field" => 0.95,
                            "search-field" => 0.9,
                            _ => 0.7,
                        };

                        self.nodes.push(WomNode {
                            id: nid.clone(),
                            kind: "element".into(),
                            role: role.into(),
                            name: label.clone(),
                            value: None,
                            state: NodeState {
                                visible: true,
                                enabled: true,
                                focused: None,
                                invalid: None,
                            },
                            capabilities: capabilities.clone(),
                            locator: Some(Locator {
                                semantic_path: format!("{role}[label={label}]"),
                                aliases: vec![placeholder, aria, name_attr].into_iter()
                                    .filter(|s| !s.is_empty())
                                    .collect(),
                            }),
                            importance,
                        });

                        // Action
                        let action_kind = if role.contains("search") {
                            "search"
                        } else if role == "checkbox" || role == "radio" {
                            "click"
                        } else {
                            "type"
                        };

                        let aid = self.next_action_id();
                        self.actions.push(WomAction {
                            action_id: aid,
                            kind: action_kind.into(),
                            target: nid.clone(),
                            args_hint: Some(format!("text for {label}")),
                            preconditions: vec!["visible".into(), "enabled".into()],
                            expected_effects: vec!["value_changed".into()],
                            risk: "low".into(),
                        });

                        // Track as form field
                        self.current_form_fields.push(nid);
                        return;
                    }
                    "form" => {
                        let action = semantic::get_attr(handle, "action").unwrap_or_default();
                        let fid = self.next_node_id("form");
                        self.current_form_id = Some(fid.clone());
                        self.current_form_fields.clear();
                        self.current_form_submit = None;

                        // Infer form intent from action URL and fields
                        let lower_action = action.to_lowercase();
                        self.current_form_intent = if lower_action.contains("login") || lower_action.contains("auth") || lower_action.contains("session") {
                            "authenticate".into()
                        } else if lower_action.contains("search") || lower_action.contains("query") {
                            "search".into()
                        } else if lower_action.contains("register") || lower_action.contains("signup") {
                            "register".into()
                        } else {
                            "submit_data".into()
                        };

                        // Recurse into form children
                        for child in handle.children.borrow().iter() {
                            self.walk(child, depth + 1);
                        }

                        // Now build FormInfo with collected fields
                        self.forms.push(FormInfo {
                            id: fid,
                            fields: self.current_form_fields.clone(),
                            submit: self.current_form_submit.clone(),
                            intent: self.current_form_intent.clone(),
                        });

                        self.current_form_id = None;
                        return;
                    }
                    "select" => {
                        let nid = self.next_node_id("sel");
                        let aria = semantic::get_attr(handle, "aria-label").unwrap_or_default();
                        let name_attr = semantic::get_attr(handle, "name").unwrap_or_default();
                        let label = if !aria.is_empty() { aria } else { name_attr };

                        self.nodes.push(WomNode {
                            id: nid.clone(),
                            kind: "element".into(),
                            role: "combobox".into(),
                            name: label,
                            value: None,
                            state: NodeState { visible: true, enabled: true, focused: None, invalid: None },
                            capabilities: vec!["click".into(), "select".into()],
                            locator: None,
                            importance: 0.6,
                        });
                        self.current_form_fields.push(nid);
                        return;
                    }
                    // Detect role="button" on non-button elements (e.g. div[role=button])
                    _ if semantic::get_attr(handle, "role").as_deref() == Some("button") => {
                        let text = semantic::extract_text(handle).trim().to_string();
                        if !text.is_empty() && text.len() < 120 && !is_noise_text(&text) {
                            let nid = self.next_node_id("btn");
                            let display = if text.len() > 60 { format!("{}...", truncate_at(&text, 60)) } else { text.clone() };
                            let disabled = semantic::get_attr(handle, "aria-disabled").as_deref() == Some("true");
                            self.nodes.push(WomNode {
                                id: nid.clone(),
                                kind: "element".into(),
                                role: "button".into(),
                                name: display.clone(),
                                value: None,
                                state: NodeState { visible: true, enabled: !disabled, focused: None, invalid: None },
                                capabilities: vec!["click".into()],
                                locator: Some(Locator { semantic_path: format!("button[text={display}]"), aliases: vec![] }),
                                importance: 0.8,
                            });
                            let aid = self.next_action_id();
                            self.actions.push(WomAction {
                                action_id: aid, kind: "click".into(), target: nid,
                                args_hint: None, preconditions: vec!["visible".into()],
                                expected_effects: vec!["state_changed".into()], risk: "low".into(),
                            });
                        }
                        return;
                    }
                    // Detect contenteditable or role="textbox" (e.g. LinkedIn message compose)
                    _ if semantic::get_attr(handle, "contenteditable").as_deref() == Some("true")
                      || semantic::get_attr(handle, "role").as_deref() == Some("textbox") => {
                        let placeholder = semantic::get_attr(handle, "data-placeholder")
                            .or_else(|| semantic::get_attr(handle, "aria-placeholder"))
                            .or_else(|| semantic::get_attr(handle, "aria-label"))
                            .unwrap_or_default();
                        let nid = self.next_node_id("fld");
                        self.nodes.push(WomNode {
                            id: nid.clone(),
                            kind: "element".into(),
                            role: "textbox".into(),
                            name: placeholder.clone(),
                            value: None,
                            state: NodeState { visible: true, enabled: true, focused: None, invalid: None },
                            capabilities: vec!["focus".into(), "type".into(), "clear".into()],
                            locator: Some(Locator {
                                semantic_path: format!("textbox[label={placeholder}]"),
                                aliases: vec![],
                            }),
                            importance: 0.85,
                        });
                        let aid = self.next_action_id();
                        self.actions.push(WomAction {
                            action_id: aid, kind: "type".into(), target: nid.clone(),
                            args_hint: Some(format!("text for {placeholder}")),
                            preconditions: vec!["visible".into(), "enabled".into()],
                            expected_effects: vec!["value_changed".into()], risk: "low".into(),
                        });
                        self.current_form_fields.push(nid);
                        return;
                    }
                    "img" => {
                        let alt = semantic::get_attr(handle, "alt").unwrap_or_default();
                        if !alt.is_empty() {
                            let nid = self.next_node_id("img");
                            self.nodes.push(WomNode {
                                id: nid,
                                kind: "element".into(),
                                role: "image".into(),
                                name: alt,
                                value: None,
                                state: NodeState { visible: true, enabled: true, focused: None, invalid: None },
                                capabilities: vec![],
                                locator: None,
                                importance: 0.3,
                            });
                        }
                        return;
                    }
                    "p" => {
                        let text = semantic::extract_text(handle).trim().to_string();
                        if !text.is_empty() && text.len() > 2 && !is_noise_text(&text) {
                            let nid = self.next_node_id("p");
                            self.paragraphs.push(TextItem {
                                id: nid.clone(),
                                text: if text.len() > 300 {
                                    format!("{}...", truncate_at(&text, 300))
                                } else {
                                    text.clone()
                                },
                                importance: 0.5,
                            });
                        }
                        return;
                    }
                    _ => {}
                }

                // Recurse for non-special elements
                for child in handle.children.borrow().iter() {
                    self.walk(child, depth + 1);
                }
            }
            NodeData::Text { contents } => {
                // Standalone text nodes handled by parent elements
                let _ = contents;
            }
            _ => {
                for child in handle.children.borrow().iter() {
                    self.walk(child, depth + 1);
                }
            }
        }
    }
}

fn is_noise_text(text: &str) -> bool {
    if text.contains('{') && text.contains('}') && text.contains(':') {
        return true;
    }
    if text.starts_with("data:") {
        return true;
    }
    if text.len() > 200 && !text.contains(' ') {
        return true;
    }
    false
}

// ─── Public API ───

/// Build a WOM document from a DOM tree.
pub fn build(
    document: &Handle,
    url: &str,
    title: &str,
    html_bytes: usize,
    mode: &str,
    revision: Revision,
) -> WomDocument {
    let mut builder = WomBuilder::new();
    builder.walk(document, 0);

    // Stats
    let mut stats = semantic::PageStats::new();
    semantic::count_nodes(document, &mut stats);

    // Page classification (reuse vision logic inline)
    let page_class = classify_from_nodes(&builder, url, title);

    // Goal surface
    let (intents, warnings) = infer_intents(&builder, &page_class);

    // Origin
    let origin = url::Url::parse(url)
        .map(|u| format!("{}://{}", u.scheme(), u.host_str().unwrap_or("")))
        .unwrap_or_default();

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    // Limit actions for output
    let actions = if builder.actions.len() > 25 {
        let mut kept = Vec::new();
        let mut nav_count = 0;
        for a in builder.actions {
            if a.kind == "navigate" {
                if nav_count < 10 {
                    kept.push(a);
                    nav_count += 1;
                }
            } else {
                kept.push(a);
            }
        }
        kept
    } else {
        builder.actions
    };

    // Add scroll action if there's significant content
    let mut actions = actions;
    if builder.nodes.len() > 20 {
        actions.push(WomAction {
            action_id: format!("a_scroll"),
            kind: "scroll".into(),
            target: "page".into(),
            args_hint: Some("down | up | top | bottom".into()),
            preconditions: vec![],
            expected_effects: vec!["viewport_changed".into()],
            risk: "low".into(),
        });
    }

    let wom_json_estimate = serde_json::to_string(&builder.nodes).unwrap_or_default().len()
        + serde_json::to_string(&actions).unwrap_or_default().len() + 500;

    WomDocument {
        session: SessionInfo {
            page_id: format!("pg_{:x}", timestamp % 0xFFFF),
            revision,
            timestamp_ms: timestamp,
            mode: mode.to_string(),
        },
        page: PageInfo {
            url: url.to_string(),
            origin,
            title: title.to_string(),
            page_class: page_class.clone(),
            load_state: "complete".into(),
            language: "en".into(), // TODO: detect
            is_https: url.starts_with("https"),
        },
        goal_surface: GoalSurface {
            primary_intents: intents,
            warnings,
        },
        nodes: builder.nodes,
        actions,
        content: ContentBlock {
            headings: builder.headings,
            paragraphs: if builder.paragraphs.len() > 30 {
                builder.paragraphs.into_iter().take(30).collect()
            } else {
                builder.paragraphs
            },
            links: if builder.links.len() > 40 {
                builder.links.into_iter().take(40).collect()
            } else {
                builder.links
            },
            forms: builder.forms,
        },
        observability: Observability {
            dom_node_count: stats.total_nodes,
            semantic_node_count: stats.semantic_nodes,
            links_count: stats.links,
            buttons_count: stats.buttons,
            forms_count: stats.forms,
        },
        delta: None,
        compression: CompressionStats {
            raw_html_bytes: html_bytes,
            wom_bytes: wom_json_estimate,
            compression_ratio: if wom_json_estimate > 0 {
                html_bytes as f32 / wom_json_estimate as f32
            } else {
                0.0
            },
        },
    }
}

/// Generate compact format for fast agent loops.
pub fn compact(doc: &WomDocument) -> WomCompact {
    // Sort by importance (high first) so fields/key actions appear before nav chrome
    let mut interactive: Vec<&WomNode> = doc.nodes.iter()
        .filter(|n| n.importance > 0.5 && !n.capabilities.is_empty())
        .collect();
    interactive.sort_by(|a, b| b.importance.partial_cmp(&a.importance).unwrap_or(std::cmp::Ordering::Equal));
    let focus: Vec<String> = interactive.into_iter()
        .take(25)
        .map(|n| format!("{}:{}:{}", n.id, n.role, n.name))
        .collect();

    let next: Vec<String> = doc.actions.iter()
        .filter(|a| a.risk != "high")
        .take(20)
        .map(|a| format!("{}:{}", a.kind, a.target))
        .collect();

    WomCompact {
        rev: doc.session.revision,
        class: doc.page.page_class.clone(),
        focus,
        events: vec![],
        next,
    }
}

// ─── Internals ───

fn classify_from_nodes(builder: &WomBuilder, url: &str, title: &str) -> String {
    let url_lower = url.to_lowercase();
    let title_lower = title.to_lowercase();

    let has_password = builder.nodes.iter().any(|n| n.role == "password-field");
    let has_email = builder.nodes.iter().any(|n| n.role == "email-field");
    let has_search = builder.nodes.iter().any(|n| n.role == "search-field");
    let textbox_count = builder.nodes.iter().filter(|n| n.role.contains("textbox") || n.role.contains("field") || n.role.contains("textarea")).count();
    let button_count = builder.nodes.iter().filter(|n| n.role == "button").count();
    let link_count = builder.nodes.iter().filter(|n| n.role == "link").count();
    let heading_count = builder.nodes.iter().filter(|n| n.role.starts_with("heading")).count();

    if has_password || (has_email && textbox_count <= 3) {
        return "login".into();
    }
    if url_lower.contains("chat") || url_lower.contains("gemini") {
        return "chat".into();
    }
    if (url_lower.contains("search") || url_lower.contains("results")) && link_count > 15 {
        return "search-results".into();
    }
    if has_search && link_count < 15 {
        return "search".into();
    }
    if heading_count >= 3 && builder.paragraphs.len() > 10 {
        return "article".into();
    }
    if url_lower.contains("/profile") || url_lower.contains("/@") || title_lower.contains("profile") {
        return "profile".into();
    }
    if textbox_count >= 3 {
        return "form".into();
    }
    if button_count > 10 && heading_count > 5 {
        return "dashboard".into();
    }
    if link_count > 30 {
        return "list".into();
    }
    "unknown".into()
}

fn infer_intents(builder: &WomBuilder, page_class: &str) -> (Vec<IntentInfo>, Vec<Warning>) {
    let mut intents = Vec::new();
    let mut warnings = Vec::new();

    match page_class {
        "login" => {
            let targets: Vec<NodeId> = builder.nodes.iter()
                .filter(|n| n.role.contains("field") || n.role == "button")
                .map(|n| n.id.clone())
                .collect();
            intents.push(IntentInfo {
                intent: "authenticate".into(),
                confidence: 0.95,
                targets,
            });
        }
        "search" | "search-results" => {
            let targets: Vec<NodeId> = builder.nodes.iter()
                .filter(|n| n.role.contains("search") || n.role == "textbox")
                .map(|n| n.id.clone())
                .collect();
            intents.push(IntentInfo {
                intent: "search".into(),
                confidence: 0.90,
                targets,
            });
        }
        "chat" => {
            let targets: Vec<NodeId> = builder.nodes.iter()
                .filter(|n| n.role == "textbox" || n.role == "textarea" || n.role == "button")
                .map(|n| n.id.clone())
                .collect();
            intents.push(IntentInfo {
                intent: "send_message".into(),
                confidence: 0.85,
                targets,
            });
        }
        "form" => {
            let targets: Vec<NodeId> = builder.nodes.iter()
                .filter(|n| !n.capabilities.is_empty())
                .map(|n| n.id.clone())
                .collect();
            intents.push(IntentInfo {
                intent: "fill_form".into(),
                confidence: 0.80,
                targets,
            });
        }
        "article" => {
            intents.push(IntentInfo {
                intent: "read_content".into(),
                confidence: 0.85,
                targets: vec![],
            });
        }
        _ => {}
    }

    // Warnings
    let captcha_nodes: Vec<&WomNode> = builder.nodes.iter()
        .filter(|n| n.name.to_lowercase().contains("captcha") || n.name.to_lowercase().contains("robot"))
        .collect();
    if !captcha_nodes.is_empty() {
        warnings.push(Warning {
            kind: "captcha".into(),
            node_id: Some(captcha_nodes[0].id.clone()),
            severity: "high".into(),
            message: "CAPTCHA detected — may block automation".into(),
        });
    }

    (intents, warnings)
}

/// Format WOM as pretty JSON for display.
pub fn format_json(doc: &WomDocument) -> String {
    serde_json::to_string_pretty(doc).unwrap_or_else(|e| format!("WOM serialization error: {e}"))
}

/// Format WOM compact as one-line JSON.
pub fn format_compact(compact: &WomCompact) -> String {
    serde_json::to_string(compact).unwrap_or_default()
}

/// Format WOM as AI-readable content: compact text with stable IDs.
/// Best of both worlds: readable like semantic text, actionable like WOM.
/// ~10x smaller than full JSON, ~3x larger than compact, but has actual content.
pub fn format_content(doc: &WomDocument) -> String {
    let mut lines: Vec<String> = Vec::new();

    // Header: classification + stats
    lines.push(format!(
        "[{}] {} | {} nodes | {} actions | rev {}",
        doc.page.page_class, doc.page.title, doc.nodes.len(),
        doc.actions.len(), doc.session.revision,
    ));
    lines.push(format!("url: {}", doc.page.url));
    lines.push(String::new());

    // Warnings first
    for w in &doc.goal_surface.warnings {
        lines.push(format!("⚠ {}: {}", w.kind, w.message));
    }

    // Intents
    for intent in &doc.goal_surface.primary_intents {
        lines.push(format!("→ intent: {} ({:.0}%)", intent.intent, intent.confidence * 100.0));
    }
    if !doc.goal_surface.primary_intents.is_empty() || !doc.goal_surface.warnings.is_empty() {
        lines.push(String::new());
    }

    // Forms (interactive elements first — most actionable)
    let forms = &doc.content.forms;
    if !forms.is_empty() {
        for form in forms {
            let field_descs: Vec<String> = form.fields.iter().map(|fid| {
                if let Some(node) = doc.nodes.iter().find(|n| &n.id == fid) {
                    format!("  {} [{}] {}", fid, node.role, node.name)
                } else {
                    format!("  {}", fid)
                }
            }).collect();
            lines.push(format!("{} form → {}", form.id,
                form.submit.as_deref().unwrap_or("no submit")));
            for fd in field_descs {
                lines.push(fd);
            }
        }
        lines.push(String::new());
    }

    // Content nodes — group by role, skip nav noise
    let nav_links: Vec<&WomNode> = doc.nodes.iter()
        .filter(|n| n.role == "link" && n.importance < 0.4)
        .collect();

    let content_links: Vec<&WomNode> = doc.nodes.iter()
        .filter(|n| n.role == "link" && n.importance >= 0.4)
        .collect();

    let buttons: Vec<&WomNode> = doc.nodes.iter()
        .filter(|n| n.role == "button")
        .collect();

    let fields: Vec<&WomNode> = doc.nodes.iter()
        .filter(|n| n.role.contains("field") || n.role.contains("textbox") || n.role.contains("textarea"))
        .collect();

    // Headings from content block
    for h in &doc.content.headings {
        lines.push(format!("# {}", h.text));
    }
    if !doc.content.headings.is_empty() {
        lines.push(String::new());
    }

    // Main content links (the interesting ones)
    if !content_links.is_empty() {
        for node in &content_links {
            let href = node.value.as_deref().unwrap_or("");
            let href_short = if href.len() > 50 { &href[..50] } else { href };
            if node.name.is_empty() {
                lines.push(format!("{} → {}", node.id, href_short));
            } else {
                lines.push(format!("{} {} → {}", node.id, node.name, href_short));
            }
        }
        lines.push(String::new());
    }

    // Paragraphs (article content)
    for p in &doc.content.paragraphs {
        lines.push(p.text.clone());
    }
    if !doc.content.paragraphs.is_empty() {
        lines.push(String::new());
    }

    // Buttons
    if !buttons.is_empty() {
        for node in &buttons {
            lines.push(format!("{} [btn] {}", node.id, node.name));
        }
        lines.push(String::new());
    }

    // Fields (standalone, not in forms)
    let form_field_ids: Vec<&str> = forms.iter()
        .flat_map(|f| f.fields.iter().map(|s| s.as_str()))
        .collect();
    let standalone_fields: Vec<&&WomNode> = fields.iter()
        .filter(|n| !form_field_ids.contains(&n.id.as_str()))
        .collect();
    if !standalone_fields.is_empty() {
        for node in standalone_fields {
            let val = node.value.as_deref().unwrap_or("");
            if val.is_empty() {
                lines.push(format!("{} [{}] {}", node.id, node.role, node.name));
            } else {
                lines.push(format!("{} [{}] {} = \"{}\"", node.id, node.role, node.name, val));
            }
        }
        lines.push(String::new());
    }

    // Nav links (collapsed)
    if !nav_links.is_empty() {
        let nav_summary: Vec<String> = nav_links.iter()
            .take(10)
            .map(|n| format!("{}:{}", n.id, n.name))
            .collect();
        lines.push(format!("nav: {} | {}", nav_links.len(),
            nav_summary.join(" | ")));
        if nav_links.len() > 10 {
            lines.push(format!("  ... +{} more", nav_links.len() - 10));
        }
    }

    lines.join("\n")
}

use markup5ever_rcdom::NodeData;
