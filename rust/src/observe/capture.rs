//! Taking a snapshot, and reading the state that makes changes detectable.
//!
//! The state properties matter more than they look. Detecting that an action worked means
//! comparing before and after, and the interesting changes are often not structural — a
//! checkbox's `checked`, a field's value, an `aria-expanded` flipping. Miss those and a
//! successful action reports as unverified.

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

use super::reference::StableRef;
use super::types::{Snapshot, SnapshotMode, SnapshotNode, INTERACTIVE_ROLES, STATIC_ROLES};
use crate::cdp::{CdpClient, CdpError};

/// Capture an accessibility snapshot with stable references.
pub async fn snapshot(client: &CdpClient, mode: SnapshotMode) -> Result<Snapshot, CdpError> {
    let tree = client
        .send("Accessibility.getFullAXTree", json!({}))
        .await?;
    let url = crate::page::current_url(client).await.unwrap_or_default();

    let mut nodes = Vec::new();
    // Per-(role, name) counters produce the `nth` component. Because the AX tree is
    // returned in document order, the same element gets the same nth across
    // snapshots as long as the elements before it are unchanged.
    let mut seen: BTreeMap<(String, String), usize> = BTreeMap::new();

    let Some(raw_nodes) = tree.get("nodes").and_then(|n| n.as_array()) else {
        return Ok(Snapshot {
            mode,
            url,
            nodes,
            truncated: false,
        });
    };

    for node in raw_nodes {
        if node
            .get("ignored")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            continue;
        }
        let role = ax_str(node, "role");
        let name = ax_str(node, "name");
        let Some(backend) = node.get("backendDOMNodeId").and_then(Value::as_i64) else {
            continue;
        };

        let keep = match mode {
            SnapshotMode::Interactive => INTERACTIVE_ROLES.contains(&role.as_str()),
            SnapshotMode::Visible => {
                INTERACTIVE_ROLES.contains(&role.as_str()) || STATIC_ROLES.contains(&role.as_str())
            }
            SnapshotMode::Full => true,
        };
        if !keep {
            continue;
        }
        // An unnamed control is not actionable by description and only adds noise —
        // except in Full mode, which exists precisely to show everything.
        if name.is_empty() && mode != SnapshotMode::Full {
            continue;
        }

        let counter = seen.entry((role.clone(), name.clone())).or_insert(0);
        let reference = StableRef::encode(&role, &name, *counter);
        *counter += 1;

        nodes.push(SnapshotNode {
            reference,
            role,
            name,
            state: extract_state(node),
            backend_node_id: backend,
        });
    }

    Ok(Snapshot {
        mode,
        url,
        nodes,
        truncated: false,
    })
}
pub(super) fn render_state(state: &BTreeMap<String, String>) -> String {
    if state.is_empty() {
        return "(none)".into();
    }
    state
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(",")
}

/// Pull `role`/`name` out of an AX node, which nests them under `{value: …}`.
pub(super) fn ax_str(node: &Value, key: &str) -> String {
    node.get(key)
        .and_then(|r| r.get("value"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string()
}

/// The AX properties worth reporting as state.
///
/// A deliberately small list: these are the ones that change what an action would do
/// (a disabled button, a collapsed section, a checked box). Including everything
/// would make the diff noisy enough to be ignored.
const STATE_PROPS: &[&str] = &[
    "disabled", "checked", "expanded", "selected", "pressed", "required", "invalid", "focused",
    "readonly",
];

pub(super) fn extract_state(node: &Value) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(props) = node.get("properties").and_then(Value::as_array) else {
        return out;
    };
    for p in props {
        let Some(name) = p.get("name").and_then(Value::as_str) else {
            continue;
        };
        if !STATE_PROPS.contains(&name) {
            continue;
        }
        let value = p
            .get("value")
            .and_then(|v| v.get("value"))
            .map(|v| match v {
                Value::Bool(b) => b.to_string(),
                Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .unwrap_or_default();
        // `false` is the default for every one of these, so recording it would
        // double the size of the state map without adding information.
        if value.is_empty() || value == "false" {
            continue;
        }
        out.insert(name.to_string(), value);
    }
    out
}
