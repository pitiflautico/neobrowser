//! CDP Browser domain — version info, window management, downloads, permissions.

use super::{CdpResult, CdpTransport};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// ── Types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WindowBounds {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_state: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserVersion {
    pub protocol_version: String,
    pub product: String,
    pub revision: String,
    pub user_agent: String,
    pub js_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Histogram {
    pub name: String,
    pub sum: i64,
    pub count: i64,
    pub buckets: Vec<Bucket>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bucket {
    pub low: i32,
    pub high: i32,
    pub count: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PermissionDescriptor {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sysex: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_visible_only: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_without_sanitization: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pan_tilt_zoom: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    pub guid: String,
    pub total_bytes: f64,
    pub received_bytes: f64,
    pub state: String,
}

// ── Methods ─────────────────────────────────────────────────────────

pub async fn get_version(transport: &dyn CdpTransport) -> CdpResult<BrowserVersion> {
    let raw = transport.send("Browser.getVersion", json!({})).await?;
    let version: BrowserVersion = serde_json::from_value(raw)?;
    Ok(version)
}

pub async fn close(transport: &dyn CdpTransport) -> CdpResult<()> {
    transport.send("Browser.close", json!({})).await?;
    Ok(())
}

pub async fn get_window_for_target(
    transport: &dyn CdpTransport,
    target_id: Option<&str>,
) -> CdpResult<(i64, WindowBounds)> {
    let mut params = json!({});
    if let Some(id) = target_id {
        params["targetId"] = json!(id);
    }
    let raw = transport
        .send("Browser.getWindowForTarget", params)
        .await?;
    let window_id = raw["windowId"]
        .as_i64()
        .ok_or("Missing windowId")?;
    let bounds: WindowBounds = serde_json::from_value(raw["bounds"].clone())?;
    Ok((window_id, bounds))
}

pub async fn set_window_bounds(
    transport: &dyn CdpTransport,
    window_id: i64,
    bounds: WindowBounds,
) -> CdpResult<()> {
    let params = json!({
        "windowId": window_id,
        "bounds": serde_json::to_value(&bounds)?,
    });
    transport.send("Browser.setWindowBounds", params).await?;
    Ok(())
}

pub async fn get_window_bounds(
    transport: &dyn CdpTransport,
    window_id: i64,
) -> CdpResult<WindowBounds> {
    let raw = transport
        .send("Browser.getWindowBounds", json!({ "windowId": window_id }))
        .await?;
    let bounds: WindowBounds = serde_json::from_value(raw["bounds"].clone())?;
    Ok(bounds)
}

pub async fn set_download_behavior(
    transport: &dyn CdpTransport,
    behavior: &str,
    download_path: Option<&str>,
    events_enabled: Option<bool>,
) -> CdpResult<()> {
    let mut params = json!({ "behavior": behavior });
    if let Some(path) = download_path {
        params["downloadPath"] = json!(path);
    }
    if let Some(enabled) = events_enabled {
        params["eventsEnabled"] = json!(enabled);
    }
    transport
        .send("Browser.setDownloadBehavior", params)
        .await?;
    Ok(())
}

pub async fn grant_permissions(
    transport: &dyn CdpTransport,
    permissions: &[&str],
    origin: Option<&str>,
    browser_context_id: Option<&str>,
) -> CdpResult<()> {
    let mut params = json!({ "permissions": permissions });
    if let Some(o) = origin {
        params["origin"] = json!(o);
    }
    if let Some(ctx) = browser_context_id {
        params["browserContextId"] = json!(ctx);
    }
    transport
        .send("Browser.grantPermissions", params)
        .await?;
    Ok(())
}

pub async fn reset_permissions(
    transport: &dyn CdpTransport,
    browser_context_id: Option<&str>,
) -> CdpResult<()> {
    let mut params = json!({});
    if let Some(ctx) = browser_context_id {
        params["browserContextId"] = json!(ctx);
    }
    transport
        .send("Browser.resetPermissions", params)
        .await?;
    Ok(())
}

pub async fn crash(transport: &dyn CdpTransport) -> CdpResult<()> {
    transport.send("Browser.crash", json!({})).await?;
    Ok(())
}

pub async fn get_histogram(
    transport: &dyn CdpTransport,
    name: &str,
    delta: Option<bool>,
) -> CdpResult<Histogram> {
    let mut params = json!({ "name": name });
    if let Some(d) = delta {
        params["delta"] = json!(d);
    }
    let raw = transport
        .send("Browser.getHistogram", params)
        .await?;
    let histogram: Histogram = serde_json::from_value(raw["histogram"].clone())?;
    Ok(histogram)
}

pub async fn get_histograms(
    transport: &dyn CdpTransport,
    query: Option<&str>,
    delta: Option<bool>,
) -> CdpResult<Vec<Histogram>> {
    let mut params = json!({});
    if let Some(q) = query {
        params["query"] = json!(q);
    }
    if let Some(d) = delta {
        params["delta"] = json!(d);
    }
    let raw = transport
        .send("Browser.getHistograms", params)
        .await?;
    let histograms: Vec<Histogram> = serde_json::from_value(raw["histograms"].clone())?;
    Ok(histograms)
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdp::MockTransport;
    use serde_json::json;

    #[tokio::test]
    async fn test_get_version() {
        let mock = MockTransport::new();
        mock.expect(
            "Browser.getVersion",
            json!({
                "protocolVersion": "1.3",
                "product": "Chrome/120.0.6099.109",
                "revision": "@abc123",
                "userAgent": "Mozilla/5.0 Chrome/120",
                "jsVersion": "12.0.267"
            }),
        )
        .await;

        let v = get_version(&mock).await.unwrap();
        assert_eq!(v.protocol_version, "1.3");
        assert_eq!(v.product, "Chrome/120.0.6099.109");
        assert_eq!(v.revision, "@abc123");
        assert_eq!(v.user_agent, "Mozilla/5.0 Chrome/120");
        assert_eq!(v.js_version, "12.0.267");
        mock.assert_called_once("Browser.getVersion").await;
    }

    #[tokio::test]
    async fn test_close() {
        let mock = MockTransport::new();
        mock.expect("Browser.close", json!({})).await;

        close(&mock).await.unwrap();
        mock.assert_called_once("Browser.close").await;
    }

    #[tokio::test]
    async fn test_get_window_for_target() {
        let mock = MockTransport::new();
        mock.expect(
            "Browser.getWindowForTarget",
            json!({
                "windowId": 42,
                "bounds": {
                    "left": 100,
                    "top": 200,
                    "width": 1024,
                    "height": 768,
                    "windowState": "normal"
                }
            }),
        )
        .await;

        let (wid, bounds) = get_window_for_target(&mock, Some("target-1")).await.unwrap();
        assert_eq!(wid, 42);
        assert_eq!(bounds.left, Some(100));
        assert_eq!(bounds.top, Some(200));
        assert_eq!(bounds.width, Some(1024));
        assert_eq!(bounds.height, Some(768));
        assert_eq!(bounds.window_state.as_deref(), Some("normal"));

        let params = mock.call_params("Browser.getWindowForTarget", 0).await.unwrap();
        assert_eq!(params["targetId"], "target-1");
    }

    #[tokio::test]
    async fn test_set_window_bounds() {
        let mock = MockTransport::new();
        mock.expect("Browser.setWindowBounds", json!({})).await;

        set_window_bounds(
            &mock,
            42,
            WindowBounds {
                window_state: Some("maximized".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let params = mock.call_params("Browser.setWindowBounds", 0).await.unwrap();
        assert_eq!(params["windowId"], 42);
        assert_eq!(params["bounds"]["windowState"], "maximized");
    }

    #[tokio::test]
    async fn test_get_window_bounds() {
        let mock = MockTransport::new();
        mock.expect(
            "Browser.getWindowBounds",
            json!({
                "bounds": {
                    "left": 0,
                    "top": 0,
                    "width": 1920,
                    "height": 1080,
                    "windowState": "fullscreen"
                }
            }),
        )
        .await;

        let bounds = get_window_bounds(&mock, 7).await.unwrap();
        assert_eq!(bounds.width, Some(1920));
        assert_eq!(bounds.height, Some(1080));
        assert_eq!(bounds.window_state.as_deref(), Some("fullscreen"));

        let params = mock.call_params("Browser.getWindowBounds", 0).await.unwrap();
        assert_eq!(params["windowId"], 7);
    }

    #[tokio::test]
    async fn test_set_download_behavior_allow() {
        let mock = MockTransport::new();
        mock.expect("Browser.setDownloadBehavior", json!({})).await;

        set_download_behavior(&mock, "allow", Some("/tmp/downloads"), Some(true))
            .await
            .unwrap();

        let params = mock.call_params("Browser.setDownloadBehavior", 0).await.unwrap();
        assert_eq!(params["behavior"], "allow");
        assert_eq!(params["downloadPath"], "/tmp/downloads");
        assert_eq!(params["eventsEnabled"], true);
    }

    #[tokio::test]
    async fn test_set_download_behavior_deny() {
        let mock = MockTransport::new();
        mock.expect("Browser.setDownloadBehavior", json!({})).await;

        set_download_behavior(&mock, "deny", None, None)
            .await
            .unwrap();

        let params = mock.call_params("Browser.setDownloadBehavior", 0).await.unwrap();
        assert_eq!(params["behavior"], "deny");
        assert!(params.get("downloadPath").is_none());
        assert!(params.get("eventsEnabled").is_none());
    }

    #[tokio::test]
    async fn test_grant_permissions() {
        let mock = MockTransport::new();
        mock.expect("Browser.grantPermissions", json!({})).await;

        grant_permissions(
            &mock,
            &["geolocation", "notifications", "midi"],
            Some("https://example.com"),
            None,
        )
        .await
        .unwrap();

        let params = mock.call_params("Browser.grantPermissions", 0).await.unwrap();
        let perms = params["permissions"].as_array().unwrap();
        assert_eq!(perms.len(), 3);
        assert_eq!(perms[0], "geolocation");
        assert_eq!(perms[1], "notifications");
        assert_eq!(perms[2], "midi");
        assert_eq!(params["origin"], "https://example.com");
    }

    #[tokio::test]
    async fn test_reset_permissions() {
        let mock = MockTransport::new();
        mock.expect("Browser.resetPermissions", json!({})).await;

        reset_permissions(&mock, Some("ctx-abc")).await.unwrap();

        let params = mock.call_params("Browser.resetPermissions", 0).await.unwrap();
        assert_eq!(params["browserContextId"], "ctx-abc");
    }

    #[tokio::test]
    async fn test_get_histogram() {
        let mock = MockTransport::new();
        mock.expect(
            "Browser.getHistogram",
            json!({
                "histogram": {
                    "name": "V8.GCScavenger",
                    "sum": 12345,
                    "count": 100,
                    "buckets": [
                        { "low": 0, "high": 10, "count": 50 },
                        { "low": 10, "high": 100, "count": 30 },
                        { "low": 100, "high": 1000, "count": 20 }
                    ]
                }
            }),
        )
        .await;

        let h = get_histogram(&mock, "V8.GCScavenger", Some(true)).await.unwrap();
        assert_eq!(h.name, "V8.GCScavenger");
        assert_eq!(h.sum, 12345);
        assert_eq!(h.count, 100);
        assert_eq!(h.buckets.len(), 3);
        assert_eq!(h.buckets[0].low, 0);
        assert_eq!(h.buckets[0].high, 10);
        assert_eq!(h.buckets[0].count, 50);

        let params = mock.call_params("Browser.getHistogram", 0).await.unwrap();
        assert_eq!(params["name"], "V8.GCScavenger");
        assert_eq!(params["delta"], true);
    }
}
