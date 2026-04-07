//! CDP DOM domain — typed wrappers for Chrome DevTools Protocol DOM methods.

use super::{CdpResult, CdpTransport};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// ── Types ───────────────────────────────────────────────────────────

pub type NodeId = i64;
pub type BackendNodeId = i64;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    pub node_id: NodeId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<NodeId>,
    pub backend_node_id: BackendNodeId,
    pub node_type: i32,
    pub node_name: String,
    pub local_name: String,
    pub node_value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub child_node_count: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<Node>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attributes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_document: Option<Box<Node>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shadow_roots: Option<Vec<Node>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoxModel {
    pub content: Vec<f64>,
    pub padding: Vec<f64>,
    pub border: Vec<f64>,
    pub margin: Vec<f64>,
    pub width: i32,
    pub height: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shape_outside: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Quad(pub Vec<f64>);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RGBA {
    pub r: i32,
    pub g: i32,
    pub b: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub a: Option<f64>,
}

// ── Helper ──────────────────────────────────────────────────────────

/// Calculate center point from content quads (useful for clicking).
pub fn quad_center(quad: &Quad) -> (f64, f64) {
    let x = (quad.0[0] + quad.0[2] + quad.0[4] + quad.0[6]) / 4.0;
    let y = (quad.0[1] + quad.0[3] + quad.0[5] + quad.0[7]) / 4.0;
    (x, y)
}

// ── Methods ─────────────────────────────────────────────────────────

pub async fn enable(transport: &dyn CdpTransport) -> CdpResult<()> {
    transport.send("DOM.enable", json!({})).await?;
    Ok(())
}

pub async fn disable(transport: &dyn CdpTransport) -> CdpResult<()> {
    transport.send("DOM.disable", json!({})).await?;
    Ok(())
}

pub async fn get_document(
    transport: &dyn CdpTransport,
    depth: Option<i32>,
    pierce: Option<bool>,
) -> CdpResult<Node> {
    let mut params = json!({});
    if let Some(d) = depth {
        params["depth"] = json!(d);
    }
    if let Some(p) = pierce {
        params["pierce"] = json!(p);
    }
    let res = transport.send("DOM.getDocument", params).await?;
    let node: Node = serde_json::from_value(res["root"].clone())?;
    Ok(node)
}

pub async fn query_selector(
    transport: &dyn CdpTransport,
    node_id: NodeId,
    selector: &str,
) -> CdpResult<NodeId> {
    let res = transport
        .send("DOM.querySelector", json!({"nodeId": node_id, "selector": selector}))
        .await?;
    Ok(res["nodeId"].as_i64().unwrap_or(0))
}

pub async fn query_selector_all(
    transport: &dyn CdpTransport,
    node_id: NodeId,
    selector: &str,
) -> CdpResult<Vec<NodeId>> {
    let res = transport
        .send("DOM.querySelectorAll", json!({"nodeId": node_id, "selector": selector}))
        .await?;
    let ids: Vec<NodeId> = serde_json::from_value(res["nodeIds"].clone())?;
    Ok(ids)
}

pub async fn get_outer_html(
    transport: &dyn CdpTransport,
    node_id: Option<NodeId>,
    backend_node_id: Option<BackendNodeId>,
    object_id: Option<&str>,
) -> CdpResult<String> {
    let mut params = json!({});
    if let Some(id) = node_id {
        params["nodeId"] = json!(id);
    }
    if let Some(id) = backend_node_id {
        params["backendNodeId"] = json!(id);
    }
    if let Some(id) = object_id {
        params["objectId"] = json!(id);
    }
    let res = transport.send("DOM.getOuterHTML", params).await?;
    Ok(res["outerHTML"].as_str().unwrap_or("").to_string())
}

pub async fn set_outer_html(
    transport: &dyn CdpTransport,
    node_id: NodeId,
    outer_html: &str,
) -> CdpResult<()> {
    transport
        .send("DOM.setOuterHTML", json!({"nodeId": node_id, "outerHTML": outer_html}))
        .await?;
    Ok(())
}

pub async fn get_box_model(
    transport: &dyn CdpTransport,
    node_id: Option<NodeId>,
    backend_node_id: Option<BackendNodeId>,
    object_id: Option<&str>,
) -> CdpResult<BoxModel> {
    let mut params = json!({});
    if let Some(id) = node_id {
        params["nodeId"] = json!(id);
    }
    if let Some(id) = backend_node_id {
        params["backendNodeId"] = json!(id);
    }
    if let Some(id) = object_id {
        params["objectId"] = json!(id);
    }
    let res = transport.send("DOM.getBoxModel", params).await?;
    let model: BoxModel = serde_json::from_value(res["model"].clone())?;
    Ok(model)
}

pub async fn get_content_quads(
    transport: &dyn CdpTransport,
    node_id: Option<NodeId>,
    backend_node_id: Option<BackendNodeId>,
    object_id: Option<&str>,
) -> CdpResult<Vec<Quad>> {
    let mut params = json!({});
    if let Some(id) = node_id {
        params["nodeId"] = json!(id);
    }
    if let Some(id) = backend_node_id {
        params["backendNodeId"] = json!(id);
    }
    if let Some(id) = object_id {
        params["objectId"] = json!(id);
    }
    let res = transport.send("DOM.getContentQuads", params).await?;
    let quads: Vec<Quad> = serde_json::from_value(res["quads"].clone())?;
    Ok(quads)
}

pub async fn get_attributes(
    transport: &dyn CdpTransport,
    node_id: NodeId,
) -> CdpResult<Vec<String>> {
    let res = transport
        .send("DOM.getAttributes", json!({"nodeId": node_id}))
        .await?;
    let attrs: Vec<String> = serde_json::from_value(res["attributes"].clone())?;
    Ok(attrs)
}

pub async fn set_attribute_value(
    transport: &dyn CdpTransport,
    node_id: NodeId,
    name: &str,
    value: &str,
) -> CdpResult<()> {
    transport
        .send("DOM.setAttributeValue", json!({"nodeId": node_id, "name": name, "value": value}))
        .await?;
    Ok(())
}

pub async fn set_attributes_as_text(
    transport: &dyn CdpTransport,
    node_id: NodeId,
    text: &str,
    name: Option<&str>,
) -> CdpResult<()> {
    let mut params = json!({"nodeId": node_id, "text": text});
    if let Some(n) = name {
        params["name"] = json!(n);
    }
    transport.send("DOM.setAttributesAsText", params).await?;
    Ok(())
}

pub async fn remove_attribute(
    transport: &dyn CdpTransport,
    node_id: NodeId,
    name: &str,
) -> CdpResult<()> {
    transport
        .send("DOM.removeAttribute", json!({"nodeId": node_id, "name": name}))
        .await?;
    Ok(())
}

pub async fn remove_node(
    transport: &dyn CdpTransport,
    node_id: NodeId,
) -> CdpResult<()> {
    transport
        .send("DOM.removeNode", json!({"nodeId": node_id}))
        .await?;
    Ok(())
}

pub async fn set_file_input_files(
    transport: &dyn CdpTransport,
    files: &[&str],
    node_id: Option<NodeId>,
    backend_node_id: Option<BackendNodeId>,
    object_id: Option<&str>,
) -> CdpResult<()> {
    let mut params = json!({"files": files});
    if let Some(id) = node_id {
        params["nodeId"] = json!(id);
    }
    if let Some(id) = backend_node_id {
        params["backendNodeId"] = json!(id);
    }
    if let Some(id) = object_id {
        params["objectId"] = json!(id);
    }
    transport.send("DOM.setFileInputFiles", params).await?;
    Ok(())
}

pub async fn describe_node(
    transport: &dyn CdpTransport,
    node_id: Option<NodeId>,
    backend_node_id: Option<BackendNodeId>,
    object_id: Option<&str>,
    depth: Option<i32>,
    pierce: Option<bool>,
) -> CdpResult<Node> {
    let mut params = json!({});
    if let Some(id) = node_id {
        params["nodeId"] = json!(id);
    }
    if let Some(id) = backend_node_id {
        params["backendNodeId"] = json!(id);
    }
    if let Some(id) = object_id {
        params["objectId"] = json!(id);
    }
    if let Some(d) = depth {
        params["depth"] = json!(d);
    }
    if let Some(p) = pierce {
        params["pierce"] = json!(p);
    }
    let res = transport.send("DOM.describeNode", params).await?;
    let node: Node = serde_json::from_value(res["node"].clone())?;
    Ok(node)
}

pub async fn request_child_nodes(
    transport: &dyn CdpTransport,
    node_id: NodeId,
    depth: Option<i32>,
    pierce: Option<bool>,
) -> CdpResult<()> {
    let mut params = json!({"nodeId": node_id});
    if let Some(d) = depth {
        params["depth"] = json!(d);
    }
    if let Some(p) = pierce {
        params["pierce"] = json!(p);
    }
    transport.send("DOM.requestChildNodes", params).await?;
    Ok(())
}

pub async fn scroll_into_view_if_needed(
    transport: &dyn CdpTransport,
    node_id: Option<NodeId>,
    backend_node_id: Option<BackendNodeId>,
    object_id: Option<&str>,
    rect: Option<Value>,
) -> CdpResult<()> {
    let mut params = json!({});
    if let Some(id) = node_id {
        params["nodeId"] = json!(id);
    }
    if let Some(id) = backend_node_id {
        params["backendNodeId"] = json!(id);
    }
    if let Some(id) = object_id {
        params["objectId"] = json!(id);
    }
    if let Some(r) = rect {
        params["rect"] = r;
    }
    transport.send("DOM.scrollIntoViewIfNeeded", params).await?;
    Ok(())
}

pub async fn focus(
    transport: &dyn CdpTransport,
    node_id: Option<NodeId>,
    backend_node_id: Option<BackendNodeId>,
    object_id: Option<&str>,
) -> CdpResult<()> {
    let mut params = json!({});
    if let Some(id) = node_id {
        params["nodeId"] = json!(id);
    }
    if let Some(id) = backend_node_id {
        params["backendNodeId"] = json!(id);
    }
    if let Some(id) = object_id {
        params["objectId"] = json!(id);
    }
    transport.send("DOM.focus", params).await?;
    Ok(())
}

pub async fn set_node_value(
    transport: &dyn CdpTransport,
    node_id: NodeId,
    value: &str,
) -> CdpResult<()> {
    transport
        .send("DOM.setNodeValue", json!({"nodeId": node_id, "value": value}))
        .await?;
    Ok(())
}

pub async fn get_node_for_location(
    transport: &dyn CdpTransport,
    x: i32,
    y: i32,
    include_user_agent_shadow_dom: Option<bool>,
    ignore_pointer_events_none: Option<bool>,
) -> CdpResult<Value> {
    let mut params = json!({"x": x, "y": y});
    if let Some(v) = include_user_agent_shadow_dom {
        params["includeUserAgentShadowDOM"] = json!(v);
    }
    if let Some(v) = ignore_pointer_events_none {
        params["ignorePointerEventsNone"] = json!(v);
    }
    let res = transport.send("DOM.getNodeForLocation", params).await?;
    Ok(res)
}

pub async fn perform_search(
    transport: &dyn CdpTransport,
    query: &str,
    include_user_agent_shadow_dom: Option<bool>,
) -> CdpResult<(String, i32)> {
    let mut params = json!({"query": query});
    if let Some(v) = include_user_agent_shadow_dom {
        params["includeUserAgentShadowDOM"] = json!(v);
    }
    let res = transport.send("DOM.performSearch", params).await?;
    let search_id = res["searchId"].as_str().unwrap_or("").to_string();
    let result_count = res["resultCount"].as_i64().unwrap_or(0) as i32;
    Ok((search_id, result_count))
}

pub async fn get_search_results(
    transport: &dyn CdpTransport,
    search_id: &str,
    from_index: i32,
    to_index: i32,
) -> CdpResult<Vec<NodeId>> {
    let res = transport
        .send(
            "DOM.getSearchResults",
            json!({"searchId": search_id, "fromIndex": from_index, "toIndex": to_index}),
        )
        .await?;
    let ids: Vec<NodeId> = serde_json::from_value(res["nodeIds"].clone())?;
    Ok(ids)
}

pub async fn discard_search_results(
    transport: &dyn CdpTransport,
    search_id: &str,
) -> CdpResult<()> {
    transport
        .send("DOM.discardSearchResults", json!({"searchId": search_id}))
        .await?;
    Ok(())
}

pub async fn move_to(
    transport: &dyn CdpTransport,
    node_id: NodeId,
    target_node_id: NodeId,
    insert_before_node_id: Option<NodeId>,
) -> CdpResult<NodeId> {
    let mut params = json!({"nodeId": node_id, "targetNodeId": target_node_id});
    if let Some(id) = insert_before_node_id {
        params["insertBeforeNodeId"] = json!(id);
    }
    let res = transport.send("DOM.moveTo", params).await?;
    Ok(res["nodeId"].as_i64().unwrap_or(0))
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdp::MockTransport;
    use serde_json::json;

    fn sample_node() -> Value {
        json!({
            "nodeId": 1,
            "backendNodeId": 10,
            "nodeType": 9,
            "nodeName": "#document",
            "localName": "",
            "nodeValue": "",
            "childNodeCount": 2,
            "children": [
                {
                    "nodeId": 2,
                    "backendNodeId": 11,
                    "nodeType": 10,
                    "nodeName": "html",
                    "localName": "html",
                    "nodeValue": ""
                },
                {
                    "nodeId": 3,
                    "backendNodeId": 12,
                    "nodeType": 1,
                    "nodeName": "HTML",
                    "localName": "html",
                    "nodeValue": "",
                    "attributes": ["lang", "en"]
                }
            ]
        })
    }

    #[tokio::test]
    async fn test_enable() {
        let mock = MockTransport::new();
        mock.expect("DOM.enable", json!({})).await;
        enable(&mock).await.unwrap();
        mock.assert_called_once("DOM.enable").await;
    }

    #[tokio::test]
    async fn test_get_document() {
        let mock = MockTransport::new();
        mock.expect("DOM.getDocument", json!({"root": sample_node()})).await;

        let node = get_document(&mock, None, None).await.unwrap();
        assert_eq!(node.node_id, 1);
        assert_eq!(node.node_name, "#document");
        assert_eq!(node.children.as_ref().unwrap().len(), 2);
        assert_eq!(node.children.as_ref().unwrap()[1].node_name, "HTML");
    }

    #[tokio::test]
    async fn test_get_document_with_depth() {
        let mock = MockTransport::new();
        mock.expect("DOM.getDocument", json!({"root": sample_node()})).await;

        get_document(&mock, Some(3), None).await.unwrap();

        let params = mock.call_params("DOM.getDocument", 0).await.unwrap();
        assert_eq!(params["depth"], 3);
    }

    #[tokio::test]
    async fn test_query_selector() {
        let mock = MockTransport::new();
        mock.expect("DOM.querySelector", json!({"nodeId": 42})).await;

        let id = query_selector(&mock, 1, "div.main").await.unwrap();
        assert_eq!(id, 42);
    }

    #[tokio::test]
    async fn test_query_selector_not_found() {
        let mock = MockTransport::new();
        mock.expect("DOM.querySelector", json!({"nodeId": 0})).await;

        let id = query_selector(&mock, 1, "div.nonexistent").await.unwrap();
        assert_eq!(id, 0);
    }

    #[tokio::test]
    async fn test_query_selector_all() {
        let mock = MockTransport::new();
        mock.expect("DOM.querySelectorAll", json!({"nodeIds": [10, 20, 30]})).await;

        let ids = query_selector_all(&mock, 1, "li").await.unwrap();
        assert_eq!(ids, vec![10, 20, 30]);
    }

    #[tokio::test]
    async fn test_get_outer_html() {
        let mock = MockTransport::new();
        mock.expect("DOM.getOuterHTML", json!({"outerHTML": "<div>hello</div>"})).await;

        let html = get_outer_html(&mock, Some(5), None, None).await.unwrap();
        assert_eq!(html, "<div>hello</div>");
    }

    #[tokio::test]
    async fn test_get_box_model() {
        let mock = MockTransport::new();
        mock.expect("DOM.getBoxModel", json!({
            "model": {
                "content": [10.0, 10.0, 110.0, 10.0, 110.0, 60.0, 10.0, 60.0],
                "padding": [8.0, 8.0, 112.0, 8.0, 112.0, 62.0, 8.0, 62.0],
                "border": [7.0, 7.0, 113.0, 7.0, 113.0, 63.0, 7.0, 63.0],
                "margin": [0.0, 0.0, 120.0, 0.0, 120.0, 70.0, 0.0, 70.0],
                "width": 100,
                "height": 50
            }
        })).await;

        let model = get_box_model(&mock, Some(5), None, None).await.unwrap();
        assert_eq!(model.width, 100);
        assert_eq!(model.height, 50);
        assert_eq!(model.content.len(), 8);
    }

    #[tokio::test]
    async fn test_get_content_quads() {
        let mock = MockTransport::new();
        mock.expect("DOM.getContentQuads", json!({
            "quads": [
                [10.0, 20.0, 110.0, 20.0, 110.0, 70.0, 10.0, 70.0],
                [10.0, 80.0, 110.0, 80.0, 110.0, 130.0, 10.0, 130.0]
            ]
        })).await;

        let quads = get_content_quads(&mock, Some(5), None, None).await.unwrap();
        assert_eq!(quads.len(), 2);
        assert_eq!(quads[0].0.len(), 8);
    }

    #[tokio::test]
    async fn test_quad_center() {
        let quad = Quad(vec![10.0, 20.0, 110.0, 20.0, 110.0, 70.0, 10.0, 70.0]);
        let (cx, cy) = quad_center(&quad);
        assert!((cx - 60.0).abs() < 0.001);
        assert!((cy - 45.0).abs() < 0.001);
    }

    #[tokio::test]
    async fn test_set_file_input_files() {
        let mock = MockTransport::new();
        mock.expect("DOM.setFileInputFiles", json!({})).await;

        set_file_input_files(&mock, &["/tmp/a.txt", "/tmp/b.txt"], Some(5), None, None)
            .await
            .unwrap();

        let params = mock.call_params("DOM.setFileInputFiles", 0).await.unwrap();
        assert_eq!(params["files"], json!(["/tmp/a.txt", "/tmp/b.txt"]));
        assert_eq!(params["nodeId"], 5);
    }

    #[tokio::test]
    async fn test_set_attribute_value() {
        let mock = MockTransport::new();
        mock.expect("DOM.setAttributeValue", json!({})).await;

        set_attribute_value(&mock, 5, "class", "active").await.unwrap();

        let params = mock.call_params("DOM.setAttributeValue", 0).await.unwrap();
        assert_eq!(params["name"], "class");
        assert_eq!(params["value"], "active");
    }

    #[tokio::test]
    async fn test_remove_node() {
        let mock = MockTransport::new();
        mock.expect("DOM.removeNode", json!({})).await;

        remove_node(&mock, 42).await.unwrap();

        let params = mock.call_params("DOM.removeNode", 0).await.unwrap();
        assert_eq!(params["nodeId"], 42);
    }

    #[tokio::test]
    async fn test_scroll_into_view() {
        let mock = MockTransport::new();
        mock.expect("DOM.scrollIntoViewIfNeeded", json!({})).await;

        scroll_into_view_if_needed(&mock, Some(7), None, None, None).await.unwrap();

        let params = mock.call_params("DOM.scrollIntoViewIfNeeded", 0).await.unwrap();
        assert_eq!(params["nodeId"], 7);
    }

    #[tokio::test]
    async fn test_focus() {
        let mock = MockTransport::new();
        mock.expect("DOM.focus", json!({})).await;

        focus(&mock, Some(9), None, None).await.unwrap();

        let params = mock.call_params("DOM.focus", 0).await.unwrap();
        assert_eq!(params["nodeId"], 9);
    }

    #[tokio::test]
    async fn test_describe_node() {
        let mock = MockTransport::new();
        mock.expect("DOM.describeNode", json!({
            "node": {
                "nodeId": 0,
                "backendNodeId": 55,
                "nodeType": 1,
                "nodeName": "DIV",
                "localName": "div",
                "nodeValue": "",
                "childNodeCount": 3
            }
        })).await;

        let node = describe_node(&mock, None, Some(55), None, Some(-1), Some(true)).await.unwrap();
        assert_eq!(node.backend_node_id, 55);
        assert_eq!(node.node_name, "DIV");

        let params = mock.call_params("DOM.describeNode", 0).await.unwrap();
        assert_eq!(params["pierce"], true);
        assert_eq!(params["depth"], -1);
    }

    #[tokio::test]
    async fn test_perform_search() {
        let mock = MockTransport::new();
        mock.expect("DOM.performSearch", json!({"searchId": "search-1", "resultCount": 5})).await;

        let (search_id, count) = perform_search(&mock, "div.item", None).await.unwrap();
        assert_eq!(search_id, "search-1");
        assert_eq!(count, 5);
    }

    #[tokio::test]
    async fn test_get_search_results() {
        let mock = MockTransport::new();
        mock.expect("DOM.getSearchResults", json!({"nodeIds": [10, 11, 12]})).await;

        let ids = get_search_results(&mock, "search-1", 0, 3).await.unwrap();
        assert_eq!(ids, vec![10, 11, 12]);
    }
}
