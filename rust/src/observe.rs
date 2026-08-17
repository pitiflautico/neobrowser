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
//! mode, and [`mod@diff`] reports only what changed since the previous one — which is
//! usually a handful of lines instead of a few thousand.
//!
//! Split into [`types`] (what a snapshot is made of), [`mod@reference`] (stable references and
//! resolution), [`capture`] (taking one, and the state that makes change detectable) and
//! [`mod@diff`] (comparing two).

pub mod capture;
pub mod diff;
pub mod reference;
pub mod types;

pub use capture::snapshot;
pub use diff::{diff, SnapshotDiff};
pub use reference::{resolve, StableRef};
pub use types::{Snapshot, SnapshotMode, SnapshotNode};

#[cfg(test)]
mod tests {
    use super::capture::extract_state;
    use super::*;
    use serde_json::json;

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
