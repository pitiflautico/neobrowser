//! What a snapshot is made of: modes, the roles worth reporting, and the node shape.
//!
//! The role tables are the whole reason a snapshot is usable by a model. A full accessibility
//! tree is thousands of nodes; an agent needs the ones it can act on and the ones that carry
//! meaning, which is what `INTERACTIVE_ROLES` and `STATIC_ROLES` separate.

//! Accessibility snapshots with stable references, and diffs between them.
//!
//! Two problems this solves.
//!
//! **References that survive a re-render.** `find` returns a `backendNodeId`, which
//! Chrome invalidates whenever the node is recreated — so on any SPA the id a model
//! was handed a moment ago is already dead, and the click lands nowhere or, worse, on
//! whatever now occupies that id. A [`StableRef`] is derived from what the element
//! *is* (role, accessible name, position among its same-role siblings) rather than
//! from a pointer, so it can be re-resolved against a fresh tree.
//!
//! **Context cost.** Returning the whole tree on every observation is what makes
//! browser tools expensive to drive. A snapshot here has a character budget and a
//! mode, and [`diff`] reports only what changed since the previous one — which is
//! usually a handful of lines instead of a few thousand.

use std::collections::BTreeMap;

use serde_json::{json, Value};

/// How much of the tree to include.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotMode {
    /// Only elements a user could act on. The default: it is what an agent needs to
    /// decide its next move, and it is the smallest useful view.
    Interactive,
    /// Interactive elements plus static text, for reading comprehension.
    Visible,
    /// Everything the accessibility tree exposes. Expensive; for debugging.
    Full,
}

impl SnapshotMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "interactive" => Some(SnapshotMode::Interactive),
            "visible" => Some(SnapshotMode::Visible),
            "full" => Some(SnapshotMode::Full),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            SnapshotMode::Interactive => "interactive",
            SnapshotMode::Visible => "visible",
            SnapshotMode::Full => "full",
        }
    }
}

/// Roles that are worth acting on.
pub(super) const INTERACTIVE_ROLES: &[&str] = &[
    "button",
    "textbox",
    "combobox",
    "searchbox",
    "link",
    "checkbox",
    "radio",
    "menuitem",
    "menuitemcheckbox",
    "menuitemradio",
    "option",
    "switch",
    "slider",
    "spinbutton",
    "tab",
    "listbox",
    "textarea",
];

/// Roles that carry meaning to read but nothing to click.
pub(super) const STATIC_ROLES: &[&str] = &[
    "heading",
    "paragraph",
    "StaticText",
    "text",
    "list",
    "listitem",
    "table",
    "row",
    "cell",
    "columnheader",
    "rowheader",
    "img",
    "alert",
    "status",
];

/// One element in a snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotNode {
    /// Stable, re-resolvable identity — see [`StableRef`].
    pub reference: String,
    pub role: String,
    pub name: String,
    /// `disabled`, `checked`, `expanded`, … — rendered as a sorted list so two
    /// snapshots of the same page produce byte-identical output.
    pub state: BTreeMap<String, String>,
    /// Chrome's current id. Useful *now*, invalid after a re-render, which is why it
    /// is not the identity.
    pub backend_node_id: i64,
}

impl SnapshotNode {
    /// One line of the rendered snapshot.
    pub(super) fn render(&self) -> String {
        let mut line = format!("{} \"{}\" [{}]", self.role, self.name, self.reference);
        if !self.state.is_empty() {
            let states: Vec<String> = self
                .state
                .iter()
                .map(|(k, v)| {
                    if v == "true" {
                        k.clone()
                    } else {
                        format!("{k}={v}")
                    }
                })
                .collect();
            line.push_str(&format!(" ({})", states.join(", ")));
        }
        line
    }
}
/// A rendered snapshot plus the nodes behind it.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub mode: SnapshotMode,
    pub url: String,
    pub nodes: Vec<SnapshotNode>,
    /// True when the character budget cut the listing short.
    pub truncated: bool,
}

impl Snapshot {
    /// The listing a model reads.
    pub fn render(&self, budget_chars: usize) -> (String, bool) {
        let mut out = String::new();
        let mut truncated = false;
        for node in &self.nodes {
            let line = node.render();
            // +1 for the newline. Stop before exceeding the budget rather than after,
            // so the result is never over the limit the caller asked for.
            if out.len() + line.len() + 1 > budget_chars {
                truncated = true;
                break;
            }
            out.push_str(&line);
            out.push('\n');
        }
        (out, truncated)
    }

    pub fn to_json(&self, budget_chars: usize) -> Value {
        let (listing, truncated) = self.render(budget_chars);
        json!({
            "mode": self.mode.as_str(),
            "url": self.url,
            "elements": self.nodes.len(),
            "truncated": truncated || self.truncated,
            "snapshot": listing,
        })
    }
}
