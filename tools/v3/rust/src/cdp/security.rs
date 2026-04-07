//! CDP Security domain — certificate errors, HTTPS state, overrides.

use super::{CdpResult, CdpTransport};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// ── Types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityStateEvent {
    pub security_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheme_is_cryptographic: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explanations: Option<Vec<SecurityStateExplanation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insecure_content_status: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityStateExplanation {
    pub security_state: String,
    pub title: String,
    pub summary: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mixed_content_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub certificate: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CertificateErrorEvent {
    pub event_id: i64,
    pub error_type: String,
    pub request_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibleSecurityState {
    pub security_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub certificate_security_state: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safety_tip_info: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security_state_issue_ids: Option<Vec<String>>,
}

// ── Methods ────────────────────────────────────────────────────────

pub async fn enable(transport: &dyn CdpTransport) -> CdpResult<()> {
    transport.send("Security.enable", json!({})).await?;
    Ok(())
}

pub async fn disable(transport: &dyn CdpTransport) -> CdpResult<()> {
    transport.send("Security.disable", json!({})).await?;
    Ok(())
}

pub async fn set_ignore_certificate_errors(
    transport: &dyn CdpTransport,
    ignore: bool,
) -> CdpResult<()> {
    transport
        .send(
            "Security.setIgnoreCertificateErrors",
            json!({ "ignore": ignore }),
        )
        .await?;
    Ok(())
}

pub async fn handle_certificate_error(
    transport: &dyn CdpTransport,
    event_id: i64,
    action: &str,
) -> CdpResult<()> {
    transport
        .send(
            "Security.handleCertificateError",
            json!({ "eventId": event_id, "action": action }),
        )
        .await?;
    Ok(())
}

pub async fn set_override_certificate_errors(
    transport: &dyn CdpTransport,
    override_: bool,
) -> CdpResult<()> {
    transport
        .send(
            "Security.setOverrideCertificateErrors",
            json!({ "override": override_ }),
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
        mock.expect("Security.enable", json!({})).await;

        enable(&mock).await.unwrap();
        mock.assert_called_once("Security.enable").await;
    }

    #[tokio::test]
    async fn test_disable() {
        let mock = MockTransport::new();
        mock.expect("Security.disable", json!({})).await;

        disable(&mock).await.unwrap();
        mock.assert_called_once("Security.disable").await;
    }

    #[tokio::test]
    async fn test_set_ignore_certificate_errors_true() {
        let mock = MockTransport::new();
        mock.expect("Security.setIgnoreCertificateErrors", json!({}))
            .await;

        set_ignore_certificate_errors(&mock, true).await.unwrap();

        let params = mock
            .call_params("Security.setIgnoreCertificateErrors", 0)
            .await
            .unwrap();
        assert_eq!(params["ignore"], true);
    }

    #[tokio::test]
    async fn test_set_ignore_certificate_errors_false() {
        let mock = MockTransport::new();
        mock.expect("Security.setIgnoreCertificateErrors", json!({}))
            .await;

        set_ignore_certificate_errors(&mock, false).await.unwrap();

        let params = mock
            .call_params("Security.setIgnoreCertificateErrors", 0)
            .await
            .unwrap();
        assert_eq!(params["ignore"], false);
    }

    #[tokio::test]
    async fn test_handle_certificate_error() {
        let mock = MockTransport::new();
        mock.expect("Security.handleCertificateError", json!({}))
            .await;

        handle_certificate_error(&mock, 42, "continue").await.unwrap();

        let params = mock
            .call_params("Security.handleCertificateError", 0)
            .await
            .unwrap();
        assert_eq!(params["eventId"], 42);
        assert_eq!(params["action"], "continue");
    }
}
