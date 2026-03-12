//! Delta Engine v1 — pragmatic semantic diffing between WOM revisions.
//!
//! Strategy: role + name + container matching (not full fingerprint yet).
//! Good enough for: form state changes, button enable/disable, new modals,
//! content updates, navigation changes.
//!
//! V2 will add: full fingerprint vectors, AX tree fusion, hierarchical matching.

use crate::wom::{DeltaBlock, DeltaOp, WomDocument, WomNode};
use std::collections::HashMap;

/// Compute delta between two WOM revisions.
/// Uses role + name as primary key (pragmatic v1).
pub fn diff(prev: &WomDocument, curr: &WomDocument) -> DeltaBlock {
    let mut ops = Vec::new();

    // Build index: (role, name) → node for previous revision
    let prev_index: HashMap<(&str, &str), &WomNode> = prev
        .nodes
        .iter()
        .map(|n| ((n.role.as_str(), n.name.as_str()), n))
        .collect();

    let curr_index: HashMap<(&str, &str), &WomNode> = curr
        .nodes
        .iter()
        .map(|n| ((n.role.as_str(), n.name.as_str()), n))
        .collect();

    // Detect updates (nodes in both revisions with changes)
    for ((role, name), curr_node) in &curr_index {
        if let Some(prev_node) = prev_index.get(&(*role, *name)) {
            let mut patch = HashMap::new();

            // State changes
            if prev_node.state.enabled != curr_node.state.enabled {
                patch.insert(
                    "enabled".to_string(),
                    serde_json::Value::Bool(curr_node.state.enabled),
                );
            }
            if prev_node.state.visible != curr_node.state.visible {
                patch.insert(
                    "visible".to_string(),
                    serde_json::Value::Bool(curr_node.state.visible),
                );
            }
            if prev_node.state.invalid != curr_node.state.invalid {
                patch.insert(
                    "invalid".to_string(),
                    serde_json::json!(curr_node.state.invalid),
                );
            }
            if prev_node.state.focused != curr_node.state.focused {
                patch.insert(
                    "focused".to_string(),
                    serde_json::json!(curr_node.state.focused),
                );
            }
            // Value changes
            if prev_node.value != curr_node.value {
                patch.insert(
                    "value".to_string(),
                    serde_json::json!(curr_node.value),
                );
            }
            // Name changes (text updated)
            if prev_node.name != curr_node.name {
                patch.insert(
                    "name".to_string(),
                    serde_json::json!(curr_node.name),
                );
            }
            // Importance changes
            if (prev_node.importance - curr_node.importance).abs() > 0.05 {
                patch.insert(
                    "importance".to_string(),
                    serde_json::json!(curr_node.importance),
                );
            }

            if !patch.is_empty() {
                ops.push(DeltaOp::UpdateNode {
                    id: curr_node.id.clone(),
                    patch,
                });
            }
        }
    }

    // Detect additions (in curr but not in prev)
    for ((role, name), curr_node) in &curr_index {
        if !prev_index.contains_key(&(*role, *name)) {
            ops.push(DeltaOp::AddNode {
                node: (*curr_node).clone(),
            });
        }
    }

    // Detect removals (in prev but not in curr)
    for ((role, name), prev_node) in &prev_index {
        if !curr_index.contains_key(&(*role, *name)) {
            ops.push(DeltaOp::RemoveNode {
                id: prev_node.id.clone(),
            });
        }
    }

    // Detect semantic events
    // Page class changed
    if prev.page.page_class != curr.page.page_class {
        ops.push(DeltaOp::EmitEvent {
            event: format!(
                "page_class_changed: {} → {}",
                prev.page.page_class, curr.page.page_class
            ),
            confidence: 0.95,
        });
    }

    // URL changed
    if prev.page.url != curr.page.url {
        ops.push(DeltaOp::EmitEvent {
            event: format!("navigated: {}", curr.page.url),
            confidence: 1.0,
        });
    }

    // New form appeared
    let prev_forms: Vec<&str> = prev.content.forms.iter().map(|f| f.intent.as_str()).collect();
    for form in &curr.content.forms {
        if !prev_forms.contains(&form.intent.as_str()) {
            ops.push(DeltaOp::EmitEvent {
                event: format!("new_form: intent={}", form.intent),
                confidence: 0.85,
            });
        }
    }

    // Goals changed
    let prev_intents: Vec<&str> = prev
        .goal_surface
        .primary_intents
        .iter()
        .map(|i| i.intent.as_str())
        .collect();
    for intent in &curr.goal_surface.primary_intents {
        if !prev_intents.contains(&intent.intent.as_str()) {
            ops.push(DeltaOp::EmitEvent {
                event: format!("new_intent: {} (conf={:.2})", intent.intent, intent.confidence),
                confidence: intent.confidence,
            });
        }
    }

    // New warnings
    let prev_warnings: Vec<&str> = prev
        .goal_surface
        .warnings
        .iter()
        .map(|w| w.kind.as_str())
        .collect();
    for warn in &curr.goal_surface.warnings {
        if !prev_warnings.contains(&warn.kind.as_str()) {
            ops.push(DeltaOp::EmitEvent {
                event: format!("warning: {} — {}", warn.kind, warn.message),
                confidence: 0.9,
            });
        }
    }

    // Generate summary
    let summary = generate_summary(&ops, prev, curr);

    DeltaBlock {
        from_revision: prev.session.revision,
        summary,
        ops,
    }
}

fn generate_summary(ops: &[DeltaOp], prev: &WomDocument, curr: &WomDocument) -> String {
    let adds = ops.iter().filter(|o| matches!(o, DeltaOp::AddNode { .. })).count();
    let removes = ops.iter().filter(|o| matches!(o, DeltaOp::RemoveNode { .. })).count();
    let updates = ops.iter().filter(|o| matches!(o, DeltaOp::UpdateNode { .. })).count();
    let events: Vec<&str> = ops
        .iter()
        .filter_map(|o| match o {
            DeltaOp::EmitEvent { event, .. } => Some(event.as_str()),
            _ => None,
        })
        .collect();

    if ops.is_empty() {
        return "no changes".into();
    }

    let mut parts = Vec::new();
    if adds > 0 {
        parts.push(format!("+{adds} nodes"));
    }
    if removes > 0 {
        parts.push(format!("-{removes} nodes"));
    }
    if updates > 0 {
        parts.push(format!("~{updates} updates"));
    }
    for e in events.iter().take(3) {
        parts.push(e.to_string());
    }

    parts.join("; ")
}
