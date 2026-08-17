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

use crate::cdp::{CdpClient, CdpError};

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
const INTERACTIVE_ROLES: &[&str] = &[
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
const STATIC_ROLES: &[&str] = &[
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
    fn render(&self) -> String {
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

fn render_state(state: &BTreeMap<String, String>) -> String {
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
fn ax_str(node: &Value, key: &str) -> String {
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

fn extract_state(node: &Value) -> BTreeMap<String, String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn node(reference: &str, role: &str, name: &str, state: &[(&str, &str)]) -> SnapshotNode {
        SnapshotNode {
            reference: reference.into(),
            role: role.into(),
            name: name.into(),
            state: state
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            backend_node_id: 1,
        }
    }

    fn snap(nodes: Vec<SnapshotNode>) -> Snapshot {
        Snapshot {
            mode: SnapshotMode::Interactive,
            url: "https://a.test/".into(),
            nodes,
            truncated: false,
        }
    }

    #[test]
    fn stable_refs_round_trip() {
        let r = StableRef::encode("button", "Continue", 0);
        assert_eq!(r, "button:Continue#0");
        assert_eq!(
            StableRef::decode(&r),
            Some(("button".into(), "Continue".into(), 0))
        );
    }

    /// A `#` inside the accessible name would make the reference ambiguous to
    /// `rsplit_once('#')`, so it must be stripped at encode time.
    #[test]
    fn stable_refs_survive_a_hash_in_the_name() {
        let r = StableRef::encode("link", "Issue #42", 3);
        assert!(!r[..r.rfind('#').unwrap()].contains('#'));
        let (role, name, nth) = StableRef::decode(&r).unwrap();
        assert_eq!(role, "link");
        assert_eq!(name, "Issue 42");
        assert_eq!(nth, 3);
    }

    #[test]
    fn stable_refs_are_bounded_in_length() {
        let long = "x".repeat(500);
        let r = StableRef::encode("button", &long, 0);
        assert!(r.len() < 60, "reference too long: {}", r.len());
        assert!(StableRef::decode(&r).is_some());
    }

    #[test]
    fn identical_snapshots_diff_to_nothing() {
        let s = snap(vec![node("button:Go#0", "button", "Go", &[])]);
        assert!(diff(&s, &s).is_empty());
        assert_eq!(diff(&s, &s).to_json()["unchanged"], true);
    }

    #[test]
    fn diff_reports_additions_removals_and_state_changes() {
        let before = snap(vec![
            node("button:Go#0", "button", "Go", &[]),
            node("button:Old#0", "button", "Old", &[]),
        ]);
        let after = snap(vec![
            node("button:Go#0", "button", "Go", &[("disabled", "true")]),
            node("button:New#0", "button", "New", &[]),
        ]);
        let d = diff(&before, &after);
        assert_eq!(d.added.len(), 1);
        assert!(d.added[0].contains("New"));
        assert_eq!(d.removed.len(), 1);
        assert!(d.removed[0].contains("Old"));
        assert_eq!(d.changed.len(), 1);
        assert!(d.changed[0].contains("button:Go#0"));
        assert!(d.changed[0].contains("(none) -> disabled=true"));
    }

    /// Keying the diff on the reference rather than the index is the point: adding
    /// one element at the top must not report every following element as changed.
    #[test]
    fn inserting_at_the_top_reports_one_addition_not_a_shift() {
        let before = snap(vec![
            node("button:A#0", "button", "A", &[]),
            node("button:B#0", "button", "B", &[]),
            node("button:C#0", "button", "C", &[]),
        ]);
        let after = snap(vec![
            node("button:Z#0", "button", "Z", &[]),
            node("button:A#0", "button", "A", &[]),
            node("button:B#0", "button", "B", &[]),
            node("button:C#0", "button", "C", &[]),
        ]);
        let d = diff(&before, &after);
        assert_eq!(d.added.len(), 1);
        assert!(d.removed.is_empty());
        assert!(d.changed.is_empty());
    }

    #[test]
    fn render_respects_the_character_budget() {
        let nodes: Vec<SnapshotNode> = (0..200)
            .map(|i| node(&format!("button:B{i}#0"), "button", &format!("B{i}"), &[]))
            .collect();
        let s = snap(nodes);
        let (text, truncated) = s.render(200);
        assert!(truncated, "should report truncation");
        assert!(text.len() <= 200, "budget exceeded: {}", text.len());
        // And the untruncated case must not claim truncation.
        let (_, truncated) = s.render(1_000_000);
        assert!(!truncated);
    }

    #[test]
    fn state_extraction_drops_defaults_and_unknown_props() {
        let n = json!({
            "properties": [
                { "name": "disabled", "value": { "value": true } },
                // false is the default for all of these: recording it is pure noise.
                { "name": "checked", "value": { "value": false } },
                // Not in STATE_PROPS: it does not change what an action would do.
                { "name": "live", "value": { "value": "polite" } },
            ]
        });
        let state = extract_state(&n);
        assert_eq!(state.get("disabled").map(String::as_str), Some("true"));
        assert!(!state.contains_key("checked"));
        assert!(!state.contains_key("live"));
    }

    #[test]
    fn snapshot_mode_parsing_is_strict() {
        assert_eq!(
            SnapshotMode::parse("interactive"),
            Some(SnapshotMode::Interactive)
        );
        assert_eq!(SnapshotMode::parse("FULL"), Some(SnapshotMode::Full));
        assert_eq!(SnapshotMode::parse("nonsense"), None);
    }

    /// State is a sorted map so the same page always renders byte-identically —
    /// otherwise the diff would report phantom changes from map ordering.
    #[test]
    fn rendering_is_deterministic_across_state_insertion_order() {
        let a = node(
            "button:X#0",
            "button",
            "X",
            &[("disabled", "true"), ("focused", "true")],
        );
        let mut b = a.clone();
        b.state = [("focused", "true"), ("disabled", "true")]
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        assert_eq!(a.render(), b.render());
    }
}
