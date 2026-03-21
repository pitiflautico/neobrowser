//! WOM delta computation — what changed between two snapshots.
//!
//! Matches nodes by stable ID to detect additions, removals, and changes.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::wom::{WomDocument, WomNode};

/// Delta between two WOM snapshots.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WomDelta {
    /// Nodes present in `after` but not `before`.
    pub added: Vec<WomNode>,
    /// IDs present in `before` but not `after`.
    pub removed: Vec<String>,
    /// (ID, description of what changed).
    pub changed: Vec<(String, String)>,
    /// Human-readable summary: "3 results updated, pagination changed".
    pub summary: String,
}

/// Compute delta between two WOM snapshots.
pub fn compute_delta(before: &WomDocument, after: &WomDocument) -> WomDelta {
    let before_map: HashMap<&str, &WomNode> =
        before.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let after_map: HashMap<&str, &WomNode> =
        after.nodes.iter().map(|n| (n.id.as_str(), n)).collect();

    let added: Vec<WomNode> = after
        .nodes
        .iter()
        .filter(|n| !before_map.contains_key(n.id.as_str()))
        .cloned()
        .collect();

    let removed: Vec<String> = before
        .nodes
        .iter()
        .filter(|n| !after_map.contains_key(n.id.as_str()))
        .map(|n| n.id.clone())
        .collect();

    let changed: Vec<(String, String)> = after
        .nodes
        .iter()
        .filter_map(|n| {
            let old = before_map.get(n.id.as_str())?;
            let diffs = diff_node(old, n);
            if diffs.is_empty() {
                None
            } else {
                Some((n.id.clone(), diffs))
            }
        })
        .collect();

    let summary = build_summary(&added, &removed, &changed);

    WomDelta {
        added,
        removed,
        changed,
        summary,
    }
}

/// Describe what changed between two nodes with the same ID.
fn diff_node(old: &WomNode, new: &WomNode) -> String {
    let mut changes = Vec::new();
    if old.label != new.label {
        changes.push(format!("label: '{}' -> '{}'", old.label, new.label));
    }
    if old.value != new.value {
        changes.push(format!("value: {:?} -> {:?}", old.value, new.value));
    }
    if old.visible != new.visible {
        changes.push(format!("visibility: {} -> {}", old.visible, new.visible));
    }
    changes.join(", ")
}

/// Build human-readable summary of the delta.
fn build_summary(added: &[WomNode], removed: &[String], changed: &[(String, String)]) -> String {
    let mut parts = Vec::new();
    if !added.is_empty() {
        parts.push(format!("{} added", added.len()));
    }
    if !removed.is_empty() {
        parts.push(format!("{} removed", removed.len()));
    }
    if !changed.is_empty() {
        parts.push(format!("{} changed", changed.len()));
    }
    if parts.is_empty() {
        "no changes".to_string()
    } else {
        parts.join(", ")
    }
}
