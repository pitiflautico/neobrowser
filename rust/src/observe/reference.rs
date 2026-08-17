//! Stable references, and resolving one back to a live node.
//!
//! `role:name#nth` exists because a `backendNodeId` is invalidated by any re-render between
//! observing a page and acting on it — and a stale id does not fail, it silently addresses a
//! different element. So references are re-resolved against the page at the moment of use,
//! which costs a round trip and buys the guarantee that the thing acted on is the thing
//! described.

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

use super::capture::snapshot;
use super::types::{SnapshotMode, SnapshotNode};
use crate::cdp::{CdpClient, CdpError};

/// A reference that can be re-resolved after the DOM is rebuilt.
///
/// Encoded as `role:name#nth`. Role and accessible name are what a person would use
/// to describe the element ("the Continue button"), and they survive React throwing
/// the node away and making a new one. `nth` disambiguates repeated identical
/// controls (a "Remove" button on every row) by their order in the tree.
///
/// It is not a global unique id and does not pretend to be: if the page genuinely
/// changes so that a different element now best matches "button Continue", that is
/// the element a human would also mean.
pub struct StableRef;

impl StableRef {
    pub fn encode(role: &str, name: &str, nth: usize) -> String {
        // The name is truncated and stripped of the delimiter so the reference stays
        // parseable and short; collisions after truncation fall back to `nth`.
        let clean: String = name
            .chars()
            .filter(|c| *c != '#' && *c != '\n')
            .take(40)
            .collect();
        format!("{role}:{}#{nth}", clean.trim())
    }

    /// Split a reference back into its parts.
    pub fn decode(reference: &str) -> Option<(String, String, usize)> {
        let (head, nth) = reference.rsplit_once('#')?;
        let (role, name) = head.split_once(':')?;
        Some((role.to_string(), name.to_string(), nth.parse().ok()?))
    }
}
/// Re-resolve a stable reference against the live tree, returning a usable
/// `backendNodeId`.
///
/// This is what makes a reference outlive a re-render: the id is looked up again at
/// use time instead of being trusted from when it was handed out.
pub async fn resolve(client: &CdpClient, reference: &str) -> Result<Option<i64>, CdpError> {
    let (role, name, nth) = match StableRef::decode(reference) {
        Some(parts) => parts,
        None => return Ok(None),
    };
    // Full mode: a reference may point at a node that the interactive filter drops
    // (an unnamed control resolved by nth), and refusing to find it would make the
    // reference less durable than the snapshot that produced it.
    let snap = snapshot(client, SnapshotMode::Full).await?;
    // Exact match first.
    if let Some(hit) = snap.nodes.iter().find(|n| n.reference == reference) {
        return Ok(Some(hit.backend_node_id));
    }
    // The element moved: same role and name, different position. Prefer the closest
    // remaining index rather than failing, since "the Nth Remove button" after a row
    // was deleted is still meaningful.
    let mut candidates: Vec<&SnapshotNode> = snap
        .nodes
        .iter()
        .filter(|n| n.role == role && n.name == name)
        .collect();
    if candidates.is_empty() {
        return Ok(None);
    }
    candidates.sort_by_key(|n| {
        StableRef::decode(&n.reference)
            .map(|(_, _, i)| i.abs_diff(nth))
            .unwrap_or(usize::MAX)
    });
    Ok(Some(candidates[0].backend_node_id))
}
