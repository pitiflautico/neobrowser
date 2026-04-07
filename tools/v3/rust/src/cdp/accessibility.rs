//! CDP Accessibility domain — AX tree inspection, queries, traversal.

use super::{CdpResult, CdpTransport};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// ── Types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AXNode {
    pub node_id: String,
    pub ignored: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<AXValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<AXValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<AXValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<AXValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<Vec<AXProperty>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub child_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend_dom_node_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AXValue {
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AXProperty {
    pub name: String,
    pub value: AXValue,
}

// ── Methods ────────────────────────────────────────────────────────

pub async fn enable(transport: &dyn CdpTransport) -> CdpResult<()> {
    transport.send("Accessibility.enable", json!({})).await?;
    Ok(())
}

pub async fn disable(transport: &dyn CdpTransport) -> CdpResult<()> {
    transport.send("Accessibility.disable", json!({})).await?;
    Ok(())
}

pub async fn get_full_ax_tree(
    transport: &dyn CdpTransport,
    depth: Option<i32>,
    frame_id: Option<&str>,
) -> CdpResult<Vec<AXNode>> {
    let mut params = json!({});
    if let Some(d) = depth {
        params["depth"] = json!(d);
    }
    if let Some(fid) = frame_id {
        params["frameId"] = json!(fid);
    }
    let raw = transport.send("Accessibility.getFullAXTree", params).await?;
    let nodes: Vec<AXNode> = serde_json::from_value(raw["nodes"].clone())?;
    Ok(nodes)
}

pub async fn get_partial_ax_tree(
    transport: &dyn CdpTransport,
    node_id: Option<i64>,
    backend_node_id: Option<i64>,
    object_id: Option<&str>,
    fetch_relatives: Option<bool>,
) -> CdpResult<Vec<AXNode>> {
    let mut params = json!({});
    if let Some(nid) = node_id {
        params["nodeId"] = json!(nid);
    }
    if let Some(bnid) = backend_node_id {
        params["backendNodeId"] = json!(bnid);
    }
    if let Some(oid) = object_id {
        params["objectId"] = json!(oid);
    }
    if let Some(fr) = fetch_relatives {
        params["fetchRelatives"] = json!(fr);
    }
    let raw = transport.send("Accessibility.getPartialAXTree", params).await?;
    let nodes: Vec<AXNode> = serde_json::from_value(raw["nodes"].clone())?;
    Ok(nodes)
}

pub async fn query_ax_tree(
    transport: &dyn CdpTransport,
    node_id: Option<i64>,
    accessible_name: Option<&str>,
    role: Option<&str>,
) -> CdpResult<Vec<AXNode>> {
    let mut params = json!({});
    if let Some(nid) = node_id {
        params["nodeId"] = json!(nid);
    }
    if let Some(name) = accessible_name {
        params["accessibleName"] = json!(name);
    }
    if let Some(r) = role {
        params["role"] = json!(r);
    }
    let raw = transport.send("Accessibility.queryAXTree", params).await?;
    let nodes: Vec<AXNode> = serde_json::from_value(raw["nodes"].clone())?;
    Ok(nodes)
}

pub async fn get_root_ax_node(
    transport: &dyn CdpTransport,
    frame_id: Option<&str>,
) -> CdpResult<AXNode> {
    let mut params = json!({});
    if let Some(fid) = frame_id {
        params["frameId"] = json!(fid);
    }
    let raw = transport.send("Accessibility.getRootAXNode", params).await?;
    let node: AXNode = serde_json::from_value(raw["node"].clone())?;
    Ok(node)
}

pub async fn get_child_ax_nodes(
    transport: &dyn CdpTransport,
    id: &str,
) -> CdpResult<Vec<AXNode>> {
    let raw = transport
        .send("Accessibility.getChildAXNodes", json!({ "id": id }))
        .await?;
    let nodes: Vec<AXNode> = serde_json::from_value(raw["nodes"].clone())?;
    Ok(nodes)
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdp::MockTransport;
    use serde_json::json;

    #[tokio::test]
    async fn test_enable() {
        let mock = MockTransport::new();
        mock.expect("Accessibility.enable", json!({})).await;

        enable(&mock).await.unwrap();
        mock.assert_called_once("Accessibility.enable").await;
    }

    #[tokio::test]
    async fn test_get_full_ax_tree() {
        let mock = MockTransport::new();
        mock.expect(
            "Accessibility.getFullAXTree",
            json!({
                "nodes": [
                    {
                        "nodeId": "1",
                        "ignored": false,
                        "role": { "type": "role", "value": "WebArea" },
                        "name": { "type": "computedString", "value": "Example" },
                        "childIds": ["2", "3"]
                    },
                    {
                        "nodeId": "2",
                        "ignored": false,
                        "role": { "type": "role", "value": "button" },
                        "name": { "type": "computedString", "value": "Click me" },
                        "parentId": "1"
                    }
                ]
            }),
        )
        .await;

        let nodes = get_full_ax_tree(&mock, Some(2), None).await.unwrap();
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].node_id, "1");
        assert_eq!(nodes[0].role.as_ref().unwrap().value, Some(json!("WebArea")));
        assert_eq!(nodes[0].name.as_ref().unwrap().value, Some(json!("Example")));
        assert_eq!(nodes[1].role.as_ref().unwrap().value, Some(json!("button")));
        assert_eq!(nodes[1].parent_id, Some("1".into()));

        let params = mock.call_params("Accessibility.getFullAXTree", 0).await.unwrap();
        assert_eq!(params["depth"], 2);
    }

    #[tokio::test]
    async fn test_query_ax_tree_by_role() {
        let mock = MockTransport::new();
        mock.expect(
            "Accessibility.queryAXTree",
            json!({
                "nodes": [
                    {
                        "nodeId": "5",
                        "ignored": false,
                        "role": { "type": "role", "value": "button" },
                        "name": { "type": "computedString", "value": "OK" }
                    },
                    {
                        "nodeId": "9",
                        "ignored": false,
                        "role": { "type": "role", "value": "button" },
                        "name": { "type": "computedString", "value": "Cancel" }
                    }
                ]
            }),
        )
        .await;

        let nodes = query_ax_tree(&mock, None, None, Some("button")).await.unwrap();
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].node_id, "5");
        assert_eq!(nodes[1].name.as_ref().unwrap().value, Some(json!("Cancel")));

        let params = mock.call_params("Accessibility.queryAXTree", 0).await.unwrap();
        assert_eq!(params["role"], "button");
        assert!(params.get("accessibleName").is_none());
    }

    #[tokio::test]
    async fn test_query_ax_tree_by_name() {
        let mock = MockTransport::new();
        mock.expect(
            "Accessibility.queryAXTree",
            json!({
                "nodes": [
                    {
                        "nodeId": "7",
                        "ignored": false,
                        "role": { "type": "role", "value": "button" },
                        "name": { "type": "computedString", "value": "Submit" }
                    }
                ]
            }),
        )
        .await;

        let nodes = query_ax_tree(&mock, None, Some("Submit"), None).await.unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].name.as_ref().unwrap().value, Some(json!("Submit")));

        let params = mock.call_params("Accessibility.queryAXTree", 0).await.unwrap();
        assert_eq!(params["accessibleName"], "Submit");
        assert!(params.get("role").is_none());
    }

    #[tokio::test]
    async fn test_get_partial_ax_tree() {
        let mock = MockTransport::new();
        mock.expect(
            "Accessibility.getPartialAXTree",
            json!({
                "nodes": [
                    {
                        "nodeId": "10",
                        "ignored": false,
                        "role": { "type": "role", "value": "textbox" },
                        "name": { "type": "computedString", "value": "Email" },
                        "backendDomNodeId": 42
                    }
                ]
            }),
        )
        .await;

        let nodes = get_partial_ax_tree(&mock, None, Some(42), None, Some(true)).await.unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].node_id, "10");
        assert_eq!(nodes[0].backend_dom_node_id, Some(42));

        let params = mock.call_params("Accessibility.getPartialAXTree", 0).await.unwrap();
        assert_eq!(params["backendNodeId"], 42);
        assert_eq!(params["fetchRelatives"], true);
    }

    #[tokio::test]
    async fn test_get_root_and_children() {
        let mock = MockTransport::new();
        mock.expect(
            "Accessibility.getRootAXNode",
            json!({
                "node": {
                    "nodeId": "root",
                    "ignored": false,
                    "role": { "type": "role", "value": "RootWebArea" },
                    "name": { "type": "computedString", "value": "Page Title" },
                    "childIds": ["c1", "c2"]
                }
            }),
        )
        .await;

        let root = get_root_ax_node(&mock, None).await.unwrap();
        assert_eq!(root.node_id, "root");
        assert_eq!(root.role.as_ref().unwrap().value, Some(json!("RootWebArea")));
        assert_eq!(root.child_ids, Some(vec!["c1".into(), "c2".into()]));

        // Now get children
        mock.expect(
            "Accessibility.getChildAXNodes",
            json!({
                "nodes": [
                    {
                        "nodeId": "c1",
                        "ignored": false,
                        "role": { "type": "role", "value": "heading" },
                        "name": { "type": "computedString", "value": "Welcome" },
                        "parentId": "root"
                    },
                    {
                        "nodeId": "c2",
                        "ignored": true,
                        "parentId": "root"
                    }
                ]
            }),
        )
        .await;

        let children = get_child_ax_nodes(&mock, "root").await.unwrap();
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].node_id, "c1");
        assert!(!children[0].ignored);
        assert!(children[1].ignored);

        let params = mock.call_params("Accessibility.getChildAXNodes", 0).await.unwrap();
        assert_eq!(params["id"], "root");
    }
}
