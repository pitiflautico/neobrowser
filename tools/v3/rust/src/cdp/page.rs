//! CDP Page domain — navigation, screenshots, lifecycle, dialogs.

use super::{CdpResult, CdpTransport};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// ── Params ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NavigateParams {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub referrer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transition_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clip: Option<Viewport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_surface: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture_beyond_viewport: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Viewport {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub scale: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PrintToPdfParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub landscape: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_header_footer: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub print_background: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paper_width: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paper_height: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin_top: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin_bottom: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin_left: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin_right: Option<f64>,
}

// ── Results ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NavigateResult {
    pub frame_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loader_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenshotResult {
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameTree {
    pub frame: Frame,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub child_frames: Option<Vec<FrameTree>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Frame {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub loader_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security_origin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

// ── Events ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameNavigatedEvent {
    pub frame: Frame,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadEventFiredEvent {
    pub timestamp: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomContentEventFiredEvent {
    pub timestamp: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleEvent {
    pub frame_id: String,
    pub loader_id: String,
    pub name: String,
    pub timestamp: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JavascriptDialogOpeningEvent {
    pub url: String,
    pub message: String,
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_browser_handler: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_prompt: Option<String>,
}

// ── Methods ─────────────────────────────────────────────────────────

pub async fn enable(transport: &dyn CdpTransport) -> CdpResult<()> {
    transport.send("Page.enable", json!({})).await?;
    Ok(())
}

pub async fn disable(transport: &dyn CdpTransport) -> CdpResult<()> {
    transport.send("Page.disable", json!({})).await?;
    Ok(())
}

pub async fn navigate(
    transport: &dyn CdpTransport,
    params: NavigateParams,
) -> CdpResult<NavigateResult> {
    let raw = transport
        .send("Page.navigate", serde_json::to_value(&params)?)
        .await?;
    let result: NavigateResult = serde_json::from_value(raw)?;
    if let Some(ref err) = result.error_text {
        return Err(format!("Navigation error: {}", err).into());
    }
    Ok(result)
}

pub async fn reload(
    transport: &dyn CdpTransport,
    ignore_cache: Option<bool>,
    script_to_evaluate_on_load: Option<String>,
) -> CdpResult<()> {
    let mut params = json!({});
    if let Some(ic) = ignore_cache {
        params["ignoreCache"] = json!(ic);
    }
    if let Some(ref script) = script_to_evaluate_on_load {
        params["scriptToEvaluateOnLoad"] = json!(script);
    }
    transport.send("Page.reload", params).await?;
    Ok(())
}

pub async fn capture_screenshot(
    transport: &dyn CdpTransport,
    params: ScreenshotParams,
) -> CdpResult<ScreenshotResult> {
    let raw = transport
        .send("Page.captureScreenshot", serde_json::to_value(&params)?)
        .await?;
    let result: ScreenshotResult = serde_json::from_value(raw)?;
    Ok(result)
}

pub async fn get_frame_tree(transport: &dyn CdpTransport) -> CdpResult<FrameTree> {
    let raw = transport.send("Page.getFrameTree", json!({})).await?;
    let tree: FrameTree = serde_json::from_value(raw["frameTree"].clone())?;
    Ok(tree)
}

pub async fn create_isolated_world(
    transport: &dyn CdpTransport,
    frame_id: &str,
    world_name: Option<&str>,
    grant_universal_access: Option<bool>,
) -> CdpResult<i64> {
    let mut params = json!({ "frameId": frame_id });
    if let Some(name) = world_name {
        params["worldName"] = json!(name);
    }
    if let Some(grant) = grant_universal_access {
        params["grantUniveralAccess"] = json!(grant);
    }
    let raw = transport
        .send("Page.createIsolatedWorld", params)
        .await?;
    let ctx_id = raw["executionContextId"]
        .as_i64()
        .ok_or("Missing executionContextId")?;
    Ok(ctx_id)
}

pub async fn handle_javascript_dialog(
    transport: &dyn CdpTransport,
    accept: bool,
    prompt_text: Option<&str>,
) -> CdpResult<()> {
    let mut params = json!({ "accept": accept });
    if let Some(text) = prompt_text {
        params["promptText"] = json!(text);
    }
    transport
        .send("Page.handleJavaScriptDialog", params)
        .await?;
    Ok(())
}

pub async fn set_lifecycle_events_enabled(
    transport: &dyn CdpTransport,
    enabled: bool,
) -> CdpResult<()> {
    transport
        .send("Page.setLifecycleEventsEnabled", json!({ "enabled": enabled }))
        .await?;
    Ok(())
}

pub async fn print_to_pdf(
    transport: &dyn CdpTransport,
    params: PrintToPdfParams,
) -> CdpResult<String> {
    let raw = transport
        .send("Page.printToPDF", serde_json::to_value(&params)?)
        .await?;
    let data = raw["data"]
        .as_str()
        .ok_or("Missing data in printToPDF response")?
        .to_string();
    Ok(data)
}

pub async fn stop_loading(transport: &dyn CdpTransport) -> CdpResult<()> {
    transport.send("Page.stopLoading", json!({})).await?;
    Ok(())
}

pub async fn bring_to_front(transport: &dyn CdpTransport) -> CdpResult<()> {
    transport.send("Page.bringToFront", json!({})).await?;
    Ok(())
}

pub async fn add_script_to_evaluate_on_new_document(
    transport: &dyn CdpTransport,
    source: &str,
    world_name: Option<&str>,
) -> CdpResult<String> {
    let mut params = json!({ "source": source });
    if let Some(name) = world_name {
        params["worldName"] = json!(name);
    }
    let raw = transport
        .send("Page.addScriptToEvaluateOnNewDocument", params)
        .await?;
    let identifier = raw["identifier"]
        .as_str()
        .ok_or("Missing identifier")?
        .to_string();
    Ok(identifier)
}

pub async fn remove_script_to_evaluate_on_new_document(
    transport: &dyn CdpTransport,
    identifier: &str,
) -> CdpResult<()> {
    transport
        .send(
            "Page.removeScriptToEvaluateOnNewDocument",
            json!({ "identifier": identifier }),
        )
        .await?;
    Ok(())
}

pub async fn set_intercept_file_chooser_dialog(
    transport: &dyn CdpTransport,
    enabled: bool,
) -> CdpResult<()> {
    transport
        .send(
            "Page.setInterceptFileChooserDialog",
            json!({ "enabled": enabled }),
        )
        .await?;
    Ok(())
}

pub async fn close(transport: &dyn CdpTransport) -> CdpResult<()> {
    transport.send("Page.close", json!({})).await?;
    Ok(())
}

pub async fn get_navigation_history(transport: &dyn CdpTransport) -> CdpResult<Value> {
    let raw = transport
        .send("Page.getNavigationHistory", json!({}))
        .await?;
    Ok(raw)
}

pub async fn navigate_to_history_entry(
    transport: &dyn CdpTransport,
    entry_id: i32,
) -> CdpResult<()> {
    transport
        .send(
            "Page.navigateToHistoryEntry",
            json!({ "entryId": entry_id }),
        )
        .await?;
    Ok(())
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdp::MockTransport;
    use serde_json::json;

    #[tokio::test]
    async fn test_enable() {
        let mock = MockTransport::new();
        mock.expect("Page.enable", json!({})).await;

        enable(&mock).await.unwrap();
        mock.assert_called_once("Page.enable").await;
    }

    #[tokio::test]
    async fn test_navigate() {
        let mock = MockTransport::new();
        mock.expect(
            "Page.navigate",
            json!({
                "frameId": "F1",
                "loaderId": "L1"
            }),
        )
        .await;

        let result = navigate(
            &mock,
            NavigateParams {
                url: "https://example.com".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert_eq!(result.frame_id, "F1");
        assert_eq!(result.loader_id, Some("L1".into()));
        assert!(result.error_text.is_none());

        let params = mock.call_params("Page.navigate", 0).await.unwrap();
        assert_eq!(params["url"], "https://example.com");
    }

    #[tokio::test]
    async fn test_navigate_error() {
        let mock = MockTransport::new();
        mock.expect(
            "Page.navigate",
            json!({
                "frameId": "F1",
                "errorText": "net::ERR_NAME_NOT_RESOLVED"
            }),
        )
        .await;

        let result = navigate(
            &mock,
            NavigateParams {
                url: "https://doesnotexist.invalid".into(),
                ..Default::default()
            },
        )
        .await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("net::ERR_NAME_NOT_RESOLVED"));
    }

    #[tokio::test]
    async fn test_capture_screenshot_png() {
        let mock = MockTransport::new();
        mock.expect(
            "Page.captureScreenshot",
            json!({ "data": "iVBORw0KGgoAAAANSUhEUgAA..." }),
        )
        .await;

        let result = capture_screenshot(
            &mock,
            ScreenshotParams {
                format: Some("png".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert!(result.data.starts_with("iVBOR"));
        let params = mock.call_params("Page.captureScreenshot", 0).await.unwrap();
        assert_eq!(params["format"], "png");
    }

    #[tokio::test]
    async fn test_capture_screenshot_jpeg() {
        let mock = MockTransport::new();
        mock.expect(
            "Page.captureScreenshot",
            json!({ "data": "/9j/4AAQSkZJRg..." }),
        )
        .await;

        let result = capture_screenshot(
            &mock,
            ScreenshotParams {
                format: Some("jpeg".into()),
                quality: Some(80),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert!(result.data.starts_with("/9j/"));
        let params = mock.call_params("Page.captureScreenshot", 0).await.unwrap();
        assert_eq!(params["format"], "jpeg");
        assert_eq!(params["quality"], 80);
    }

    #[tokio::test]
    async fn test_get_frame_tree() {
        let mock = MockTransport::new();
        mock.expect(
            "Page.getFrameTree",
            json!({
                "frameTree": {
                    "frame": {
                        "id": "main",
                        "loaderId": "L1",
                        "url": "https://example.com",
                        "mimeType": "text/html"
                    },
                    "childFrames": [{
                        "frame": {
                            "id": "iframe1",
                            "parentId": "main",
                            "loaderId": "L2",
                            "url": "https://ads.example.com",
                            "name": "ad-frame"
                        }
                    }]
                }
            }),
        )
        .await;

        let tree = get_frame_tree(&mock).await.unwrap();
        assert_eq!(tree.frame.id, "main");
        assert_eq!(tree.frame.url, "https://example.com");

        let children = tree.child_frames.unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].frame.id, "iframe1");
        assert_eq!(children[0].frame.parent_id, Some("main".into()));
        assert_eq!(children[0].frame.name, Some("ad-frame".into()));
    }

    #[tokio::test]
    async fn test_create_isolated_world() {
        let mock = MockTransport::new();
        mock.expect(
            "Page.createIsolatedWorld",
            json!({ "executionContextId": 42 }),
        )
        .await;

        let ctx_id =
            create_isolated_world(&mock, "main-frame", Some("myWorld"), Some(true))
                .await
                .unwrap();

        assert_eq!(ctx_id, 42);
        let params = mock
            .call_params("Page.createIsolatedWorld", 0)
            .await
            .unwrap();
        assert_eq!(params["frameId"], "main-frame");
        assert_eq!(params["worldName"], "myWorld");
    }

    #[tokio::test]
    async fn test_handle_dialog_accept() {
        let mock = MockTransport::new();
        mock.expect("Page.handleJavaScriptDialog", json!({})).await;

        handle_javascript_dialog(&mock, true, None).await.unwrap();

        let params = mock
            .call_params("Page.handleJavaScriptDialog", 0)
            .await
            .unwrap();
        assert_eq!(params["accept"], true);
        assert!(params.get("promptText").is_none());
    }

    #[tokio::test]
    async fn test_handle_dialog_dismiss() {
        let mock = MockTransport::new();
        mock.expect("Page.handleJavaScriptDialog", json!({})).await;

        handle_javascript_dialog(&mock, false, Some("my answer"))
            .await
            .unwrap();

        let params = mock
            .call_params("Page.handleJavaScriptDialog", 0)
            .await
            .unwrap();
        assert_eq!(params["accept"], false);
        assert_eq!(params["promptText"], "my answer");
    }

    #[tokio::test]
    async fn test_reload() {
        let mock = MockTransport::new();
        mock.expect("Page.reload", json!({})).await;
        mock.expect("Page.reload", json!({})).await;

        // Without ignore_cache
        reload(&mock, None, None).await.unwrap();
        let params = mock.call_params("Page.reload", 0).await.unwrap();
        assert!(params.get("ignoreCache").is_none());

        // With ignore_cache
        reload(&mock, Some(true), None).await.unwrap();
        let params = mock.call_params("Page.reload", 1).await.unwrap();
        assert_eq!(params["ignoreCache"], true);
    }

    #[tokio::test]
    async fn test_lifecycle_events_enabled() {
        let mock = MockTransport::new();
        mock.expect("Page.setLifecycleEventsEnabled", json!({}))
            .await;

        set_lifecycle_events_enabled(&mock, true).await.unwrap();

        let params = mock
            .call_params("Page.setLifecycleEventsEnabled", 0)
            .await
            .unwrap();
        assert_eq!(params["enabled"], true);
    }

    #[tokio::test]
    async fn test_stop_loading() {
        let mock = MockTransport::new();
        mock.expect("Page.stopLoading", json!({})).await;

        stop_loading(&mock).await.unwrap();
        mock.assert_called_once("Page.stopLoading").await;
    }

    #[tokio::test]
    async fn test_bring_to_front() {
        let mock = MockTransport::new();
        mock.expect("Page.bringToFront", json!({})).await;

        bring_to_front(&mock).await.unwrap();
        mock.assert_called_once("Page.bringToFront").await;
    }

    #[tokio::test]
    async fn test_add_script_on_new_document() {
        let mock = MockTransport::new();
        mock.expect(
            "Page.addScriptToEvaluateOnNewDocument",
            json!({ "identifier": "script-1" }),
        )
        .await;

        let id = add_script_to_evaluate_on_new_document(
            &mock,
            "console.log('injected')",
            Some("isolated"),
        )
        .await
        .unwrap();

        assert_eq!(id, "script-1");
        let params = mock
            .call_params("Page.addScriptToEvaluateOnNewDocument", 0)
            .await
            .unwrap();
        assert_eq!(params["source"], "console.log('injected')");
        assert_eq!(params["worldName"], "isolated");
    }

    #[tokio::test]
    async fn test_print_to_pdf() {
        let mock = MockTransport::new();
        mock.expect(
            "Page.printToPDF",
            json!({ "data": "JVBERi0xLjQK..." }),
        )
        .await;

        let data = print_to_pdf(
            &mock,
            PrintToPdfParams {
                landscape: Some(true),
                print_background: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert!(data.starts_with("JVBERi"));
        let params = mock.call_params("Page.printToPDF", 0).await.unwrap();
        assert_eq!(params["landscape"], true);
        assert_eq!(params["printBackground"], true);
    }
}
