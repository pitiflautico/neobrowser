//! CDP DOMStorage domain — localStorage and sessionStorage access.

use super::{CdpResult, CdpTransport};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// ── Types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageId {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security_origin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage_key: Option<String>,
    pub is_local_storage: bool,
}

// ── Events ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomStorageItemAddedEvent {
    pub storage_id: StorageId,
    pub key: String,
    pub new_value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomStorageItemRemovedEvent {
    pub storage_id: StorageId,
    pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomStorageItemUpdatedEvent {
    pub storage_id: StorageId,
    pub key: String,
    pub old_value: String,
    pub new_value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomStorageItemsClearedEvent {
    pub storage_id: StorageId,
}

// ── Methods ────────────────────────────────────────────────────────

pub async fn enable(transport: &dyn CdpTransport) -> CdpResult<()> {
    transport.send("DOMStorage.enable", json!({})).await?;
    Ok(())
}

pub async fn disable(transport: &dyn CdpTransport) -> CdpResult<()> {
    transport.send("DOMStorage.disable", json!({})).await?;
    Ok(())
}

pub async fn get_dom_storage_items(
    transport: &dyn CdpTransport,
    storage_id: &StorageId,
) -> CdpResult<Vec<Vec<String>>> {
    let raw = transport
        .send(
            "DOMStorage.getDOMStorageItems",
            json!({ "storageId": serde_json::to_value(storage_id)? }),
        )
        .await?;
    let entries: Vec<Vec<String>> = serde_json::from_value(raw["entries"].clone())?;
    Ok(entries)
}

pub async fn set_dom_storage_item(
    transport: &dyn CdpTransport,
    storage_id: &StorageId,
    key: &str,
    value: &str,
) -> CdpResult<()> {
    transport
        .send(
            "DOMStorage.setDOMStorageItem",
            json!({
                "storageId": serde_json::to_value(storage_id)?,
                "key": key,
                "value": value,
            }),
        )
        .await?;
    Ok(())
}

pub async fn remove_dom_storage_item(
    transport: &dyn CdpTransport,
    storage_id: &StorageId,
    key: &str,
) -> CdpResult<()> {
    transport
        .send(
            "DOMStorage.removeDOMStorageItem",
            json!({
                "storageId": serde_json::to_value(storage_id)?,
                "key": key,
            }),
        )
        .await?;
    Ok(())
}

pub async fn clear(
    transport: &dyn CdpTransport,
    storage_id: &StorageId,
) -> CdpResult<()> {
    transport
        .send(
            "DOMStorage.clear",
            json!({ "storageId": serde_json::to_value(storage_id)? }),
        )
        .await?;
    Ok(())
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdp::MockTransport;
    use serde_json::json;

    fn local_storage() -> StorageId {
        StorageId {
            security_origin: Some("https://example.com".into()),
            storage_key: None,
            is_local_storage: true,
        }
    }

    fn session_storage() -> StorageId {
        StorageId {
            security_origin: Some("https://example.com".into()),
            storage_key: None,
            is_local_storage: false,
        }
    }

    #[tokio::test]
    async fn test_enable() {
        let mock = MockTransport::new();
        mock.expect("DOMStorage.enable", json!({})).await;

        enable(&mock).await.unwrap();
        mock.assert_called_once("DOMStorage.enable").await;
    }

    #[tokio::test]
    async fn test_get_items() {
        let mock = MockTransport::new();
        mock.expect(
            "DOMStorage.getDOMStorageItems",
            json!({
                "entries": [
                    ["theme", "dark"],
                    ["lang", "en"],
                ]
            }),
        )
        .await;

        let items = get_dom_storage_items(&mock, &local_storage()).await.unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0], vec!["theme", "dark"]);
        assert_eq!(items[1], vec!["lang", "en"]);

        let params = mock
            .call_params("DOMStorage.getDOMStorageItems", 0)
            .await
            .unwrap();
        assert_eq!(params["storageId"]["isLocalStorage"], true);
        assert_eq!(params["storageId"]["securityOrigin"], "https://example.com");
    }

    #[tokio::test]
    async fn test_set_item() {
        let mock = MockTransport::new();
        mock.expect("DOMStorage.setDOMStorageItem", json!({})).await;

        set_dom_storage_item(&mock, &local_storage(), "token", "abc123")
            .await
            .unwrap();

        let params = mock
            .call_params("DOMStorage.setDOMStorageItem", 0)
            .await
            .unwrap();
        assert_eq!(params["storageId"]["isLocalStorage"], true);
        assert_eq!(params["key"], "token");
        assert_eq!(params["value"], "abc123");
    }

    #[tokio::test]
    async fn test_remove_item() {
        let mock = MockTransport::new();
        mock.expect("DOMStorage.removeDOMStorageItem", json!({})).await;

        remove_dom_storage_item(&mock, &local_storage(), "token")
            .await
            .unwrap();

        let params = mock
            .call_params("DOMStorage.removeDOMStorageItem", 0)
            .await
            .unwrap();
        assert_eq!(params["key"], "token");
    }

    #[tokio::test]
    async fn test_clear() {
        let mock = MockTransport::new();
        mock.expect("DOMStorage.clear", json!({})).await;

        clear(&mock, &local_storage()).await.unwrap();

        let params = mock.call_params("DOMStorage.clear", 0).await.unwrap();
        assert_eq!(params["storageId"]["isLocalStorage"], true);
    }

    #[tokio::test]
    async fn test_local_vs_session() {
        let mock = MockTransport::new();
        mock.expect("DOMStorage.getDOMStorageItems", json!({ "entries": [["a", "1"]] }))
            .await;
        mock.expect("DOMStorage.getDOMStorageItems", json!({ "entries": [["b", "2"]] }))
            .await;

        let local_items = get_dom_storage_items(&mock, &local_storage()).await.unwrap();
        let session_items = get_dom_storage_items(&mock, &session_storage()).await.unwrap();

        assert_eq!(local_items[0], vec!["a", "1"]);
        assert_eq!(session_items[0], vec!["b", "2"]);

        // Verify different storageId.isLocalStorage values were sent
        let p1 = mock
            .call_params("DOMStorage.getDOMStorageItems", 0)
            .await
            .unwrap();
        let p2 = mock
            .call_params("DOMStorage.getDOMStorageItems", 1)
            .await
            .unwrap();
        assert_eq!(p1["storageId"]["isLocalStorage"], true);
        assert_eq!(p2["storageId"]["isLocalStorage"], false);
    }
}
