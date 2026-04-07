//! CDP IndexedDB domain — inspect and manipulate IndexedDB stores.

use super::{CdpResult, CdpTransport};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// ── Types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseWithObjectStores {
    pub name: String,
    pub version: f64,
    pub object_stores: Vec<ObjectStore>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectStore {
    pub name: String,
    pub key_path: KeyPath,
    pub auto_increment: bool,
    pub indexes: Vec<ObjectStoreIndex>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectStoreIndex {
    pub name: String,
    pub key_path: KeyPath,
    pub unique: bool,
    pub multi_entry: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyPath {
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub string: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub array: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataEntry {
    pub key: Value,
    pub primary_key: Value,
    pub value: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyRange {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lower: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upper: Option<Value>,
    pub lower_open: bool,
    pub upper_open: bool,
}

// ── Helper: build origin params ────────────────────────────────────

fn origin_params(
    security_origin: Option<&str>,
    storage_key: Option<&str>,
    storage_bucket: Option<Value>,
) -> Value {
    let mut params = json!({});
    if let Some(origin) = security_origin {
        params["securityOrigin"] = json!(origin);
    }
    if let Some(key) = storage_key {
        params["storageKey"] = json!(key);
    }
    if let Some(bucket) = storage_bucket {
        params["storageBucket"] = bucket;
    }
    params
}

// ── Methods ────────────────────────────────────────────────────────

pub async fn enable(transport: &dyn CdpTransport) -> CdpResult<()> {
    transport.send("IndexedDB.enable", json!({})).await?;
    Ok(())
}

pub async fn disable(transport: &dyn CdpTransport) -> CdpResult<()> {
    transport.send("IndexedDB.disable", json!({})).await?;
    Ok(())
}

pub async fn request_database_names(
    transport: &dyn CdpTransport,
    security_origin: Option<&str>,
    storage_key: Option<&str>,
    storage_bucket: Option<Value>,
) -> CdpResult<Vec<String>> {
    let params = origin_params(security_origin, storage_key, storage_bucket);
    let raw = transport
        .send("IndexedDB.requestDatabaseNames", params)
        .await?;
    let names: Vec<String> = serde_json::from_value(raw["databaseNames"].clone())?;
    Ok(names)
}

pub async fn request_database(
    transport: &dyn CdpTransport,
    security_origin: Option<&str>,
    storage_key: Option<&str>,
    database_name: &str,
    storage_bucket: Option<Value>,
) -> CdpResult<DatabaseWithObjectStores> {
    let mut params = origin_params(security_origin, storage_key, storage_bucket);
    params["databaseName"] = json!(database_name);
    let raw = transport
        .send("IndexedDB.requestDatabase", params)
        .await?;
    let db: DatabaseWithObjectStores =
        serde_json::from_value(raw["databaseWithObjectStores"].clone())?;
    Ok(db)
}

pub async fn request_data(
    transport: &dyn CdpTransport,
    security_origin: Option<&str>,
    storage_key: Option<&str>,
    database_name: &str,
    object_store_name: &str,
    index_name: &str,
    skip_count: i32,
    page_size: i32,
    key_range: Option<KeyRange>,
) -> CdpResult<(Vec<DataEntry>, bool)> {
    let mut params = origin_params(security_origin, storage_key, None);
    params["databaseName"] = json!(database_name);
    params["objectStoreName"] = json!(object_store_name);
    params["indexName"] = json!(index_name);
    params["skipCount"] = json!(skip_count);
    params["pageSize"] = json!(page_size);
    if let Some(kr) = key_range {
        params["keyRange"] = serde_json::to_value(kr)?;
    }
    let raw = transport
        .send("IndexedDB.requestData", params)
        .await?;
    let entries: Vec<DataEntry> = serde_json::from_value(raw["objectStoreDataEntries"].clone())?;
    let has_more = raw["hasMore"].as_bool().unwrap_or(false);
    Ok((entries, has_more))
}

pub async fn get_metadata(
    transport: &dyn CdpTransport,
    security_origin: Option<&str>,
    storage_key: Option<&str>,
    database_name: &str,
    object_store_name: &str,
) -> CdpResult<(f64, f64)> {
    let mut params = origin_params(security_origin, storage_key, None);
    params["databaseName"] = json!(database_name);
    params["objectStoreName"] = json!(object_store_name);
    let raw = transport
        .send("IndexedDB.getMetadata", params)
        .await?;
    let entries_count = raw["entriesCount"].as_f64().unwrap_or(0.0);
    let key_generator_value = raw["keyGeneratorValue"].as_f64().unwrap_or(0.0);
    Ok((entries_count, key_generator_value))
}

pub async fn clear_object_store(
    transport: &dyn CdpTransport,
    security_origin: Option<&str>,
    storage_key: Option<&str>,
    database_name: &str,
    object_store_name: &str,
) -> CdpResult<()> {
    let mut params = origin_params(security_origin, storage_key, None);
    params["databaseName"] = json!(database_name);
    params["objectStoreName"] = json!(object_store_name);
    transport
        .send("IndexedDB.clearObjectStore", params)
        .await?;
    Ok(())
}

pub async fn delete_database(
    transport: &dyn CdpTransport,
    security_origin: Option<&str>,
    storage_key: Option<&str>,
    database_name: &str,
) -> CdpResult<()> {
    let mut params = origin_params(security_origin, storage_key, None);
    params["databaseName"] = json!(database_name);
    transport
        .send("IndexedDB.deleteDatabase", params)
        .await?;
    Ok(())
}

pub async fn delete_object_store_entries(
    transport: &dyn CdpTransport,
    security_origin: Option<&str>,
    storage_key: Option<&str>,
    database_name: &str,
    object_store_name: &str,
    key_range: KeyRange,
) -> CdpResult<()> {
    let mut params = origin_params(security_origin, storage_key, None);
    params["databaseName"] = json!(database_name);
    params["objectStoreName"] = json!(object_store_name);
    params["keyRange"] = serde_json::to_value(key_range)?;
    transport
        .send("IndexedDB.deleteObjectStoreEntries", params)
        .await?;
    Ok(())
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
        mock.expect("IndexedDB.enable", json!({})).await;

        enable(&mock).await.unwrap();
        mock.assert_called_once("IndexedDB.enable").await;
    }

    #[tokio::test]
    async fn test_request_database_names() {
        let mock = MockTransport::new();
        mock.expect(
            "IndexedDB.requestDatabaseNames",
            json!({ "databaseNames": ["mydb", "cache-v1"] }),
        )
        .await;

        let names = request_database_names(&mock, Some("https://example.com"), None, None)
            .await
            .unwrap();
        assert_eq!(names, vec!["mydb", "cache-v1"]);

        let params = mock
            .call_params("IndexedDB.requestDatabaseNames", 0)
            .await
            .unwrap();
        assert_eq!(params["securityOrigin"], "https://example.com");
    }

    #[tokio::test]
    async fn test_request_database() {
        let mock = MockTransport::new();
        mock.expect(
            "IndexedDB.requestDatabase",
            json!({
                "databaseWithObjectStores": {
                    "name": "mydb",
                    "version": 3.0,
                    "objectStores": [{
                        "name": "users",
                        "keyPath": { "type": "string", "string": "id" },
                        "autoIncrement": false,
                        "indexes": [{
                            "name": "by_email",
                            "keyPath": { "type": "string", "string": "email" },
                            "unique": true,
                            "multiEntry": false
                        }]
                    }]
                }
            }),
        )
        .await;

        let db = request_database(&mock, Some("https://example.com"), None, "mydb", None)
            .await
            .unwrap();
        assert_eq!(db.name, "mydb");
        assert_eq!(db.version, 3.0);
        assert_eq!(db.object_stores.len(), 1);
        assert_eq!(db.object_stores[0].name, "users");
        assert_eq!(db.object_stores[0].indexes[0].name, "by_email");
        assert!(db.object_stores[0].indexes[0].unique);
    }

    #[tokio::test]
    async fn test_request_data() {
        let mock = MockTransport::new();
        mock.expect(
            "IndexedDB.requestData",
            json!({
                "objectStoreDataEntries": [
                    { "key": { "type": "number", "value": 1 }, "primaryKey": { "type": "number", "value": 1 }, "value": { "type": "object", "value": "alice" } },
                    { "key": { "type": "number", "value": 2 }, "primaryKey": { "type": "number", "value": 2 }, "value": { "type": "object", "value": "bob" } }
                ],
                "hasMore": true
            }),
        )
        .await;

        let (entries, has_more) = request_data(
            &mock,
            Some("https://example.com"),
            None,
            "mydb",
            "users",
            "",
            0,
            10,
            None,
        )
        .await
        .unwrap();

        assert_eq!(entries.len(), 2);
        assert!(has_more);

        let params = mock
            .call_params("IndexedDB.requestData", 0)
            .await
            .unwrap();
        assert_eq!(params["databaseName"], "mydb");
        assert_eq!(params["objectStoreName"], "users");
        assert_eq!(params["pageSize"], 10);
    }

    #[tokio::test]
    async fn test_get_metadata() {
        let mock = MockTransport::new();
        mock.expect(
            "IndexedDB.getMetadata",
            json!({ "entriesCount": 42.0, "keyGeneratorValue": 43.0 }),
        )
        .await;

        let (count, key_gen) = get_metadata(
            &mock,
            Some("https://example.com"),
            None,
            "mydb",
            "users",
        )
        .await
        .unwrap();

        assert_eq!(count, 42.0);
        assert_eq!(key_gen, 43.0);
    }

    #[tokio::test]
    async fn test_clear_object_store() {
        let mock = MockTransport::new();
        mock.expect("IndexedDB.clearObjectStore", json!({})).await;

        clear_object_store(&mock, Some("https://example.com"), None, "mydb", "users")
            .await
            .unwrap();

        let params = mock
            .call_params("IndexedDB.clearObjectStore", 0)
            .await
            .unwrap();
        assert_eq!(params["databaseName"], "mydb");
        assert_eq!(params["objectStoreName"], "users");
    }

    #[tokio::test]
    async fn test_delete_database() {
        let mock = MockTransport::new();
        mock.expect("IndexedDB.deleteDatabase", json!({})).await;

        delete_database(&mock, Some("https://example.com"), None, "mydb")
            .await
            .unwrap();

        let params = mock
            .call_params("IndexedDB.deleteDatabase", 0)
            .await
            .unwrap();
        assert_eq!(params["databaseName"], "mydb");
    }
}
