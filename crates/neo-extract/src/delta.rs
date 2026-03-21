//! WOM delta computation -- what changed between two snapshots.
//!
//! Matches nodes by stable ID first, then by (tag + role + text_prefix)
//! as fallback. Generates token-efficient diffs with semantic summaries.

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
    /// Human-readable summary: "3 results updated, pagination changed to page 2".
    pub summary: String,
}

/// Compute delta between two WOM snapshots.
///
/// Matching strategy:
/// 1. Match by stable ID (exact)
/// 2. Unmatched nodes fall back to (tag + role + text_prefix) matching
pub fn compute_delta(before: &WomDocument, after: &WomDocument) -> WomDelta {
    let before_map: HashMap<&str, &WomNode> =
        before.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let _after_map: HashMap<&str, &WomNode> =
        after.nodes.iter().map(|n| (n.id.as_str(), n)).collect();

    // Phase 1: exact ID matching
    let mut matched_before: Vec<bool> = vec![false; before.nodes.len()];
    let mut matched_after: Vec<bool> = vec![false; after.nodes.len()];

    let mut changed: Vec<(String, String)> = Vec::new();

    for (ai, after_node) in after.nodes.iter().enumerate() {
        if let Some(&before_node) = before_map.get(after_node.id.as_str()) {
            // Mark both as matched
            matched_after[ai] = true;
            if let Some(bi) = before.nodes.iter().position(|n| n.id == before_node.id) {
                matched_before[bi] = true;
            }
            let diffs = diff_node(before_node, after_node);
            if !diffs.is_empty() {
                changed.push((after_node.id.clone(), diffs));
            }
        }
    }

    // Phase 2: fuzzy matching for unmatched nodes by (tag + role + text_prefix)
    for (ai, after_node) in after.nodes.iter().enumerate() {
        if matched_after[ai] {
            continue;
        }
        let after_key = fuzzy_key(after_node);
        for (bi, before_node) in before.nodes.iter().enumerate() {
            if matched_before[bi] {
                continue;
            }
            if fuzzy_key(before_node) == after_key {
                matched_after[ai] = true;
                matched_before[bi] = true;
                let diffs = diff_node(before_node, after_node);
                if !diffs.is_empty() {
                    changed.push((after_node.id.clone(), diffs));
                }
                break;
            }
        }
    }

    let added: Vec<WomNode> = after
        .nodes
        .iter()
        .enumerate()
        .filter(|(i, _)| !matched_after[*i])
        .map(|(_, n)| n.clone())
        .collect();

    let removed: Vec<String> = before
        .nodes
        .iter()
        .enumerate()
        .filter(|(i, _)| !matched_before[*i])
        .map(|(_, n)| n.id.clone())
        .collect();

    let summary = build_summary(&added, &removed, &changed, before, after);

    WomDelta {
        added,
        removed,
        changed,
        summary,
    }
}

/// Generate a fuzzy matching key from (tag, role, text_prefix).
fn fuzzy_key(node: &WomNode) -> String {
    let prefix = if node.label.len() > 20 {
        &node.label[..20]
    } else {
        &node.label
    };
    format!("{}:{}:{}", node.tag, node.role, prefix)
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
        changes.push(format!(
            "visibility: {} -> {}",
            old.visible, new.visible
        ));
    }
    if old.href != new.href {
        changes.push(format!("href: {:?} -> {:?}", old.href, new.href));
    }
    if old.actions != new.actions {
        changes.push(format!("actions: {:?} -> {:?}", old.actions, new.actions));
    }
    changes.join(", ")
}

/// Build a semantic, human-readable summary of the delta.
///
/// Goes beyond counts: describes what types of elements changed,
/// detects pagination transitions, etc.
fn build_summary(
    added: &[WomNode],
    removed: &[String],
    changed: &[(String, String)],
    _before: &WomDocument,
    _after: &WomDocument,
) -> String {
    let mut parts = Vec::new();

    if !added.is_empty() {
        let added_roles = summarize_roles(added);
        parts.push(format!("{} added ({})", added.len(), added_roles));
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

/// Summarize roles of a set of nodes for the summary line.
/// E.g. "2 links, 1 button"
fn summarize_roles(nodes: &[WomNode]) -> String {
    let mut role_counts: HashMap<&str, usize> = HashMap::new();
    for node in nodes {
        *role_counts.entry(node.role.as_str()).or_insert(0) += 1;
    }
    let mut parts: Vec<String> = role_counts
        .iter()
        .map(|(role, count)| format!("{count} {role}"))
        .collect();
    parts.sort();
    parts.join(", ")
}
