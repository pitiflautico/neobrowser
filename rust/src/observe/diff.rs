//! Comparing two snapshots.
//!
//! Keyed on stable reference rather than position, because a page that inserts one node at
//! the top would otherwise report every element below it as changed — which is
//! indistinguishable from a page that actually changed everywhere.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use super::capture::render_state;
use super::types::{Snapshot, SnapshotNode};

/// What changed between two snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    /// `reference: before -> after` for elements whose state changed.
    pub changed: Vec<String>,
}

impl SnapshotDiff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }

    pub fn to_json(&self) -> Value {
        json!({
            "added": self.added,
            "removed": self.removed,
            "changed": self.changed,
            "unchanged": self.is_empty(),
        })
    }
}

/// Diff two snapshots by stable reference.
///
/// Keyed on the reference rather than on position, so inserting one element at the
/// top of a list reports one addition instead of shifting — and thus rewriting —
/// every line after it.
pub fn diff(before: &Snapshot, after: &Snapshot) -> SnapshotDiff {
    let index = |s: &Snapshot| -> BTreeMap<String, SnapshotNode> {
        s.nodes
            .iter()
            .map(|n| (n.reference.clone(), n.clone()))
            .collect()
    };
    let (b, a) = (index(before), index(after));

    let added = a
        .iter()
        .filter(|(k, _)| !b.contains_key(*k))
        .map(|(_, n)| n.render())
        .collect();
    let removed = b
        .iter()
        .filter(|(k, _)| !a.contains_key(*k))
        .map(|(_, n)| n.render())
        .collect();
    let changed = a
        .iter()
        .filter_map(|(k, after_node)| {
            let before_node = b.get(k)?;
            if before_node.state == after_node.state {
                return None;
            }
            Some(format!(
                "{k}: {} -> {}",
                render_state(&before_node.state),
                render_state(&after_node.state)
            ))
        })
        .collect();

    SnapshotDiff {
        added,
        removed,
        changed,
    }
}
