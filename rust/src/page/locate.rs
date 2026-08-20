//! Finding an element by intent, through the accessibility tree.
//!
//! CSS selectors describe a document's structure; a model describes what it wants. This
//! module bridges the two by searching the accessibility tree — the same representation a
//! screen reader consumes — so "the login button" matches an element by its accessible
//! role and name rather than by a class name that a redeploy will change.

use serde_json::json;

use crate::cdp::{CdpClient, CdpError};

use super::eval::nudge_frame;

/// A semantic node from the accessibility tree.
#[derive(Debug, Clone)]
pub struct AxNode {
    pub role: String,
    pub name: String,
    pub backend_node_id: i64,
}

/// Interactive roles worth surfacing for `find`.
pub(super) const INTERACTIVE_ROLES: &[&str] = &[
    "button",
    "textbox",
    "combobox",
    "searchbox",
    "link",
    "checkbox",
    "radio",
    "menuitem",
    "tab",
    "switch",
    "slider",
    "option",
];

/// Extract interactive nodes with names from the accessibility tree.
pub async fn ax_interactive_nodes(client: &CdpClient) -> Result<Vec<AxNode>, CdpError> {
    let tree = client
        .send("Accessibility.getFullAXTree", json!({}))
        .await?;
    let mut out = Vec::new();
    let Some(nodes) = tree.get("nodes").and_then(|n| n.as_array()) else {
        return Ok(out);
    };
    for node in nodes {
        if node
            .get("ignored")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            continue;
        }
        let role = node
            .get("role")
            .and_then(|r| r.get("value"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let name = node
            .get("name")
            .and_then(|r| r.get("value"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let backend = node
            .get("backendDOMNodeId")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        if backend == 0 {
            continue;
        }
        // Keep interactive roles even without a name; keep named nodes of any role
        // only if interactive (avoids flooding with StaticText).
        let interactive = INTERACTIVE_ROLES.contains(&role.as_str());
        if interactive {
            out.push(AxNode {
                role,
                name,
                backend_node_id: backend,
            });
        }
    }
    Ok(out)
}

/// Semantic find: score interactive AX nodes against the intent (zero-cost
/// heuristic, Layers 1–2), then fall back to an optional LLM (Layer 3) that only
/// runs when `ANTHROPIC_API_KEY` is set. The LLM only *chooses among* the
/// backendNodeIds we already extracted, and its choice is validated against that
/// set — a prompt injection in page text can't point us at a node not in the snapshot.
pub async fn find(client: &CdpClient, intent: &str) -> Result<Option<AxNode>, CdpError> {
    // The AX tree only contains rendered nodes; force a frame so deferred UI is in it.
    nudge_frame(client).await;
    let nodes = ax_interactive_nodes(client).await?;
    let intent_l = intent.to_lowercase();
    let tokens: Vec<&str> = intent_l
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() > 1)
        .collect();

    // Role hints from the intent phrasing.
    let wants_button =
        intent_l.contains("button") || intent_l.contains("submit") || intent_l.contains("send");
    let wants_input = intent_l.contains("input")
        || intent_l.contains("box")
        || intent_l.contains("field")
        || intent_l.contains("search")
        || intent_l.contains("type")
        || intent_l.contains("write");
    let wants_link = intent_l.contains("link");

    let mut best: Option<(i64, &AxNode)> = None;
    for n in &nodes {
        let name_l = n.name.to_lowercase();
        let mut score: i64 = 0;
        for t in &tokens {
            if name_l == *t {
                score += 10;
            } else if name_l.contains(t) {
                score += 5;
            }
        }
        match n.role.as_str() {
            "button" if wants_button => score += 4,
            "textbox" | "combobox" | "searchbox" if wants_input => score += 4,
            "searchbox" if intent_l.contains("search") => score += 3,
            "link" if wants_link => score += 4,
            _ => {}
        }
        if !n.name.is_empty() {
            score += 1;
        }
        if score > 0 {
            match &best {
                Some((bs, _)) if *bs >= score => {}
                _ => best = Some((score, n)),
            }
        }
    }
    if let Some((_, n)) = best {
        return Ok(Some(n.clone()));
    }

    // Layer 3: optional LLM fallback (no-op + zero cost unless a key is configured).
    if crate::llm::available() && !nodes.is_empty() {
        let snapshot = nodes
            .iter()
            .map(|n| format!("{} | {:?} | {}", n.role, n.name, n.backend_node_id))
            .collect::<Vec<_>>()
            .join("\n");
        if let Some(id) = crate::llm::find_by_intent(&snapshot, intent).await {
            // Validate the LLM's choice against the snapshot (anti prompt-injection).
            if let Some(n) = nodes.iter().find(|n| n.backend_node_id == id) {
                return Ok(Some(n.clone()));
            }
        }
    }
    Ok(None)
}
