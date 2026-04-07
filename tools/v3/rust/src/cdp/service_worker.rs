//! CDP ServiceWorker domain — registration, lifecycle, push/sync events.

use super::{CdpResult, CdpTransport};
use serde::{Deserialize, Serialize};
use serde_json::json;

// ── Types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceWorkerRegistration {
    pub registration_id: String,
    pub scope_url: String,
    pub is_deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceWorkerVersion {
    pub version_id: String,
    pub registration_id: String,
    pub script_url: String,
    /// "stopped", "starting", "running", "stopping"
    pub running_status: String,
    /// "new", "installing", "installed", "activating", "activated", "redundant"
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script_last_modified: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script_response_time: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub controlled_clients: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub router_rules: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceWorkerErrorMessage {
    pub error_message: String,
    pub registration_id: String,
    pub version_id: String,
    pub source_url: String,
    pub line_number: i32,
    pub column_number: i32,
}

// ── Methods ────────────────────────────────────────────────────────

pub async fn enable(transport: &dyn CdpTransport) -> CdpResult<()> {
    transport.send("ServiceWorker.enable", json!({})).await?;
    Ok(())
}

pub async fn disable(transport: &dyn CdpTransport) -> CdpResult<()> {
    transport.send("ServiceWorker.disable", json!({})).await?;
    Ok(())
}

pub async fn start_worker(transport: &dyn CdpTransport, scope_url: &str) -> CdpResult<()> {
    transport
        .send("ServiceWorker.startWorker", json!({ "scopeURL": scope_url }))
        .await?;
    Ok(())
}

pub async fn stop_worker(transport: &dyn CdpTransport, version_id: &str) -> CdpResult<()> {
    transport
        .send("ServiceWorker.stopWorker", json!({ "versionId": version_id }))
        .await?;
    Ok(())
}

pub async fn stop_all_workers(transport: &dyn CdpTransport) -> CdpResult<()> {
    transport
        .send("ServiceWorker.stopAllWorkers", json!({}))
        .await?;
    Ok(())
}

pub async fn unregister(transport: &dyn CdpTransport, scope_url: &str) -> CdpResult<()> {
    transport
        .send("ServiceWorker.unregister", json!({ "scopeURL": scope_url }))
        .await?;
    Ok(())
}

pub async fn update_registration(
    transport: &dyn CdpTransport,
    scope_url: &str,
) -> CdpResult<()> {
    transport
        .send(
            "ServiceWorker.updateRegistration",
            json!({ "scopeURL": scope_url }),
        )
        .await?;
    Ok(())
}

pub async fn skip_waiting(transport: &dyn CdpTransport, scope_url: &str) -> CdpResult<()> {
    transport
        .send("ServiceWorker.skipWaiting", json!({ "scopeURL": scope_url }))
        .await?;
    Ok(())
}

pub async fn set_force_update_on_page_load(
    transport: &dyn CdpTransport,
    force_update: bool,
) -> CdpResult<()> {
    transport
        .send(
            "ServiceWorker.setForceUpdateOnPageLoad",
            json!({ "forceUpdateOnPageLoad": force_update }),
        )
        .await?;
    Ok(())
}

pub async fn deliver_push_message(
    transport: &dyn CdpTransport,
    origin: &str,
    registration_id: &str,
    data: &str,
) -> CdpResult<()> {
    transport
        .send(
            "ServiceWorker.deliverPushMessage",
            json!({
                "origin": origin,
                "registrationId": registration_id,
                "data": data,
            }),
        )
        .await?;
    Ok(())
}

pub async fn dispatch_sync_event(
    transport: &dyn CdpTransport,
    origin: &str,
    registration_id: &str,
    tag: &str,
    last_chance: bool,
) -> CdpResult<()> {
    transport
        .send(
            "ServiceWorker.dispatchSyncEvent",
            json!({
                "origin": origin,
                "registrationId": registration_id,
                "tag": tag,
                "lastChance": last_chance,
            }),
        )
        .await?;
    Ok(())
}

pub async fn dispatch_periodic_sync_event(
    transport: &dyn CdpTransport,
    origin: &str,
    registration_id: &str,
    tag: &str,
) -> CdpResult<()> {
    transport
        .send(
            "ServiceWorker.dispatchPeriodicSyncEvent",
            json!({
                "origin": origin,
                "registrationId": registration_id,
                "tag": tag,
            }),
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

    #[tokio::test]
    async fn test_enable() {
        let mock = MockTransport::new();
        mock.expect("ServiceWorker.enable", json!({})).await;

        enable(&mock).await.unwrap();
        mock.assert_called_once("ServiceWorker.enable").await;
    }

    #[tokio::test]
    async fn test_disable() {
        let mock = MockTransport::new();
        mock.expect("ServiceWorker.disable", json!({})).await;

        disable(&mock).await.unwrap();
        mock.assert_called_once("ServiceWorker.disable").await;
    }

    #[tokio::test]
    async fn test_start_worker() {
        let mock = MockTransport::new();
        mock.expect("ServiceWorker.startWorker", json!({})).await;

        start_worker(&mock, "https://example.com/app/").await.unwrap();

        let params = mock
            .call_params("ServiceWorker.startWorker", 0)
            .await
            .unwrap();
        assert_eq!(params["scopeURL"], "https://example.com/app/");
    }

    #[tokio::test]
    async fn test_stop_worker() {
        let mock = MockTransport::new();
        mock.expect("ServiceWorker.stopWorker", json!({})).await;

        stop_worker(&mock, "v-123").await.unwrap();

        let params = mock
            .call_params("ServiceWorker.stopWorker", 0)
            .await
            .unwrap();
        assert_eq!(params["versionId"], "v-123");
    }

    #[tokio::test]
    async fn test_stop_all_workers() {
        let mock = MockTransport::new();
        mock.expect("ServiceWorker.stopAllWorkers", json!({})).await;

        stop_all_workers(&mock).await.unwrap();
        mock.assert_called_once("ServiceWorker.stopAllWorkers").await;
    }

    #[tokio::test]
    async fn test_unregister() {
        let mock = MockTransport::new();
        mock.expect("ServiceWorker.unregister", json!({})).await;

        unregister(&mock, "https://example.com/").await.unwrap();

        let params = mock
            .call_params("ServiceWorker.unregister", 0)
            .await
            .unwrap();
        assert_eq!(params["scopeURL"], "https://example.com/");
    }

    #[tokio::test]
    async fn test_skip_waiting() {
        let mock = MockTransport::new();
        mock.expect("ServiceWorker.skipWaiting", json!({})).await;

        skip_waiting(&mock, "https://example.com/").await.unwrap();

        let params = mock
            .call_params("ServiceWorker.skipWaiting", 0)
            .await
            .unwrap();
        assert_eq!(params["scopeURL"], "https://example.com/");
    }

    #[tokio::test]
    async fn test_set_force_update() {
        let mock = MockTransport::new();
        mock.expect("ServiceWorker.setForceUpdateOnPageLoad", json!({}))
            .await;

        set_force_update_on_page_load(&mock, true).await.unwrap();

        let params = mock
            .call_params("ServiceWorker.setForceUpdateOnPageLoad", 0)
            .await
            .unwrap();
        assert_eq!(params["forceUpdateOnPageLoad"], true);
    }
}
