//! CDP Target domain — target discovery, attach/detach, browser contexts.

use super::{CdpResult, CdpTransport};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// ── Types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetInfo {
    pub target_id: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub title: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attached: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opener_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub can_access_opener: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub browser_context_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteLocation {
    pub host: String,
    pub port: i32,
}

// ── Methods ────────────────────────────────────────────────────────

/// Enable target discovery via Target.setDiscoverTargets.
pub async fn enable(transport: &dyn CdpTransport) -> CdpResult<()> {
    set_discover_targets(transport, true, None).await
}

/// Get list of available targets.
pub async fn get_targets(
    transport: &dyn CdpTransport,
    filter: Option<Value>,
) -> CdpResult<Vec<TargetInfo>> {
    let mut params = json!({});
    if let Some(f) = filter {
        params["filter"] = f;
    }
    let raw = transport.send("Target.getTargets", params).await?;
    let targets: Vec<TargetInfo> = serde_json::from_value(raw["targetInfos"].clone())?;
    Ok(targets)
}

/// Attach to a target. Returns the session ID.
pub async fn attach_to_target(
    transport: &dyn CdpTransport,
    target_id: &str,
    flatten: Option<bool>,
) -> CdpResult<String> {
    let mut params = json!({ "targetId": target_id });
    if let Some(f) = flatten {
        params["flatten"] = json!(f);
    }
    let raw = transport.send("Target.attachToTarget", params).await?;
    let session_id = raw["sessionId"]
        .as_str()
        .ok_or("missing sessionId")?
        .to_string();
    Ok(session_id)
}

/// Detach from a target.
pub async fn detach_from_target(
    transport: &dyn CdpTransport,
    session_id: Option<&str>,
    target_id: Option<&str>,
) -> CdpResult<()> {
    let mut params = json!({});
    if let Some(s) = session_id {
        params["sessionId"] = json!(s);
    }
    if let Some(t) = target_id {
        params["targetId"] = json!(t);
    }
    transport.send("Target.detachFromTarget", params).await?;
    Ok(())
}

/// Create a new page target. Returns the target ID.
pub async fn create_target(
    transport: &dyn CdpTransport,
    url: &str,
    width: Option<i32>,
    height: Option<i32>,
    browser_context_id: Option<&str>,
    enable_begin_frame_control: Option<bool>,
    new_window: Option<bool>,
    background: Option<bool>,
) -> CdpResult<String> {
    let mut params = json!({ "url": url });
    if let Some(w) = width {
        params["width"] = json!(w);
    }
    if let Some(h) = height {
        params["height"] = json!(h);
    }
    if let Some(ctx) = browser_context_id {
        params["browserContextId"] = json!(ctx);
    }
    if let Some(b) = enable_begin_frame_control {
        params["enableBeginFrameControl"] = json!(b);
    }
    if let Some(nw) = new_window {
        params["newWindow"] = json!(nw);
    }
    if let Some(bg) = background {
        params["background"] = json!(bg);
    }
    let raw = transport.send("Target.createTarget", params).await?;
    let target_id = raw["targetId"]
        .as_str()
        .ok_or("missing targetId")?
        .to_string();
    Ok(target_id)
}

/// Close a target. Returns whether it was successfully closed.
pub async fn close_target(
    transport: &dyn CdpTransport,
    target_id: &str,
) -> CdpResult<bool> {
    let raw = transport
        .send("Target.closeTarget", json!({ "targetId": target_id }))
        .await?;
    let success = raw["success"].as_bool().unwrap_or(false);
    Ok(success)
}

/// Activate (bring to front) a target.
pub async fn activate_target(
    transport: &dyn CdpTransport,
    target_id: &str,
) -> CdpResult<()> {
    transport
        .send("Target.activateTarget", json!({ "targetId": target_id }))
        .await?;
    Ok(())
}

/// Create a new browser context. Returns the browser context ID.
pub async fn create_browser_context(
    transport: &dyn CdpTransport,
    dispose_on_detach: Option<bool>,
    proxy_server: Option<&str>,
    proxy_bypass_list: Option<&str>,
) -> CdpResult<String> {
    let mut params = json!({});
    if let Some(d) = dispose_on_detach {
        params["disposeOnDetach"] = json!(d);
    }
    if let Some(p) = proxy_server {
        params["proxyServer"] = json!(p);
    }
    if let Some(b) = proxy_bypass_list {
        params["proxyBypassList"] = json!(b);
    }
    let raw = transport
        .send("Target.createBrowserContext", params)
        .await?;
    let ctx_id = raw["browserContextId"]
        .as_str()
        .ok_or("missing browserContextId")?
        .to_string();
    Ok(ctx_id)
}

/// Dispose a browser context.
pub async fn dispose_browser_context(
    transport: &dyn CdpTransport,
    browser_context_id: &str,
) -> CdpResult<()> {
    transport
        .send(
            "Target.disposeBrowserContext",
            json!({ "browserContextId": browser_context_id }),
        )
        .await?;
    Ok(())
}

/// Get all existing browser contexts.
pub async fn get_browser_contexts(
    transport: &dyn CdpTransport,
) -> CdpResult<Vec<String>> {
    let raw = transport
        .send("Target.getBrowserContexts", json!({}))
        .await?;
    let ids: Vec<String> = serde_json::from_value(raw["browserContextIds"].clone())?;
    Ok(ids)
}

/// Set auto-attach for new targets.
pub async fn set_auto_attach(
    transport: &dyn CdpTransport,
    auto_attach: bool,
    wait_for_debugger_on_start: bool,
    flatten: Option<bool>,
    filter: Option<Value>,
) -> CdpResult<()> {
    let mut params = json!({
        "autoAttach": auto_attach,
        "waitForDebuggerOnStart": wait_for_debugger_on_start,
    });
    if let Some(f) = flatten {
        params["flatten"] = json!(f);
    }
    if let Some(flt) = filter {
        params["filter"] = flt;
    }
    transport.send("Target.setAutoAttach", params).await?;
    Ok(())
}

/// Enable/disable target discovery.
pub async fn set_discover_targets(
    transport: &dyn CdpTransport,
    discover: bool,
    filter: Option<Value>,
) -> CdpResult<()> {
    let mut params = json!({ "discover": discover });
    if let Some(f) = filter {
        params["filter"] = f;
    }
    transport
        .send("Target.setDiscoverTargets", params)
        .await?;
    Ok(())
}

/// Get information about a target.
pub async fn get_target_info(
    transport: &dyn CdpTransport,
    target_id: Option<&str>,
) -> CdpResult<TargetInfo> {
    let mut params = json!({});
    if let Some(t) = target_id {
        params["targetId"] = json!(t);
    }
    let raw = transport.send("Target.getTargetInfo", params).await?;
    let info: TargetInfo = serde_json::from_value(raw["targetInfo"].clone())?;
    Ok(info)
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdp::MockTransport;
    use serde_json::json;

    #[tokio::test]
    async fn test_get_targets() {
        let mock = MockTransport::new();
        mock.expect(
            "Target.getTargets",
            json!({
                "targetInfos": [{
                    "targetId": "T1",
                    "type": "page",
                    "title": "Example",
                    "url": "https://example.com",
                    "attached": true
                }]
            }),
        )
        .await;

        let targets = get_targets(&mock, None).await.unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].target_id, "T1");
        assert_eq!(targets[0].type_, "page");
        assert_eq!(targets[0].title, "Example");
        assert_eq!(targets[0].url, "https://example.com");
        assert_eq!(targets[0].attached, Some(true));
        mock.assert_called_once("Target.getTargets").await;
    }

    #[tokio::test]
    async fn test_attach_to_target() {
        let mock = MockTransport::new();
        mock.expect(
            "Target.attachToTarget",
            json!({ "sessionId": "S1" }),
        )
        .await;

        let session_id = attach_to_target(&mock, "T1", None).await.unwrap();
        assert_eq!(session_id, "S1");

        let params = mock.call_params("Target.attachToTarget", 0).await.unwrap();
        assert_eq!(params["targetId"], "T1");
        assert!(params.get("flatten").is_none());
    }

    #[tokio::test]
    async fn test_attach_with_flatten() {
        let mock = MockTransport::new();
        mock.expect(
            "Target.attachToTarget",
            json!({ "sessionId": "S2" }),
        )
        .await;

        let session_id = attach_to_target(&mock, "T2", Some(true)).await.unwrap();
        assert_eq!(session_id, "S2");

        let params = mock.call_params("Target.attachToTarget", 0).await.unwrap();
        assert_eq!(params["targetId"], "T2");
        assert_eq!(params["flatten"], true);
    }

    #[tokio::test]
    async fn test_detach_from_target() {
        let mock = MockTransport::new();
        mock.expect("Target.detachFromTarget", json!({})).await;

        detach_from_target(&mock, Some("S1"), None).await.unwrap();

        let params = mock
            .call_params("Target.detachFromTarget", 0)
            .await
            .unwrap();
        assert_eq!(params["sessionId"], "S1");
    }

    #[tokio::test]
    async fn test_create_target() {
        let mock = MockTransport::new();
        mock.expect(
            "Target.createTarget",
            json!({ "targetId": "T_NEW" }),
        )
        .await;

        let target_id = create_target(
            &mock,
            "about:blank",
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(target_id, "T_NEW");

        let params = mock.call_params("Target.createTarget", 0).await.unwrap();
        assert_eq!(params["url"], "about:blank");
    }

    #[tokio::test]
    async fn test_create_target_with_options() {
        let mock = MockTransport::new();
        mock.expect(
            "Target.createTarget",
            json!({ "targetId": "T_OPT" }),
        )
        .await;

        let target_id = create_target(
            &mock,
            "https://example.com",
            Some(1280),
            Some(720),
            None,
            None,
            Some(true),
            None,
        )
        .await
        .unwrap();
        assert_eq!(target_id, "T_OPT");

        let params = mock.call_params("Target.createTarget", 0).await.unwrap();
        assert_eq!(params["url"], "https://example.com");
        assert_eq!(params["width"], 1280);
        assert_eq!(params["height"], 720);
        assert_eq!(params["newWindow"], true);
    }

    #[tokio::test]
    async fn test_close_target() {
        let mock = MockTransport::new();
        mock.expect(
            "Target.closeTarget",
            json!({ "success": true }),
        )
        .await;

        let success = close_target(&mock, "T1").await.unwrap();
        assert!(success);

        let params = mock.call_params("Target.closeTarget", 0).await.unwrap();
        assert_eq!(params["targetId"], "T1");
    }

    #[tokio::test]
    async fn test_activate_target() {
        let mock = MockTransport::new();
        mock.expect("Target.activateTarget", json!({})).await;

        activate_target(&mock, "T1").await.unwrap();

        let params = mock
            .call_params("Target.activateTarget", 0)
            .await
            .unwrap();
        assert_eq!(params["targetId"], "T1");
    }

    #[tokio::test]
    async fn test_create_browser_context() {
        let mock = MockTransport::new();
        mock.expect(
            "Target.createBrowserContext",
            json!({ "browserContextId": "CTX1" }),
        )
        .await;

        let ctx_id =
            create_browser_context(&mock, Some(true), None, None)
                .await
                .unwrap();
        assert_eq!(ctx_id, "CTX1");

        let params = mock
            .call_params("Target.createBrowserContext", 0)
            .await
            .unwrap();
        assert_eq!(params["disposeOnDetach"], true);
    }

    #[tokio::test]
    async fn test_dispose_browser_context() {
        let mock = MockTransport::new();
        mock.expect("Target.disposeBrowserContext", json!({})).await;

        dispose_browser_context(&mock, "CTX1").await.unwrap();

        let params = mock
            .call_params("Target.disposeBrowserContext", 0)
            .await
            .unwrap();
        assert_eq!(params["browserContextId"], "CTX1");
    }

    #[tokio::test]
    async fn test_get_browser_contexts() {
        let mock = MockTransport::new();
        mock.expect(
            "Target.getBrowserContexts",
            json!({ "browserContextIds": ["CTX1", "CTX2"] }),
        )
        .await;

        let ids = get_browser_contexts(&mock).await.unwrap();
        assert_eq!(ids, vec!["CTX1", "CTX2"]);
    }

    #[tokio::test]
    async fn test_set_auto_attach() {
        let mock = MockTransport::new();
        mock.expect("Target.setAutoAttach", json!({})).await;

        set_auto_attach(&mock, true, false, Some(true), None)
            .await
            .unwrap();

        let params = mock
            .call_params("Target.setAutoAttach", 0)
            .await
            .unwrap();
        assert_eq!(params["autoAttach"], true);
        assert_eq!(params["waitForDebuggerOnStart"], false);
        assert_eq!(params["flatten"], true);
    }

    #[tokio::test]
    async fn test_set_discover_targets() {
        let mock = MockTransport::new();
        mock.expect("Target.setDiscoverTargets", json!({})).await;

        set_discover_targets(&mock, true, None).await.unwrap();

        let params = mock
            .call_params("Target.setDiscoverTargets", 0)
            .await
            .unwrap();
        assert_eq!(params["discover"], true);
    }
}
