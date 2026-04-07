//! CDP Log domain — browser log entries, violation reporting.

use super::{CdpResult, CdpTransport};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// ── Types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    /// "xml", "javascript", "network", "storage", "appcache", "rendering",
    /// "security", "deprecation", "worker", "violation", "intervention",
    /// "recommendation", "other"
    pub source: String,
    /// "verbose", "info", "warning", "error"
    pub level: String,
    pub text: String,
    pub timestamp: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_number: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stack_trace: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViolationSetting {
    /// "longTask", "longLayout", "blockedEvent", "blockedParser",
    /// "discouragedAPIUse", "handler", "recurringHandler"
    pub name: String,
    pub threshold: f64,
}

// ── Methods ────────────────────────────────────────────────────────

pub async fn enable(transport: &dyn CdpTransport) -> CdpResult<()> {
    transport.send("Log.enable", json!({})).await?;
    Ok(())
}

pub async fn disable(transport: &dyn CdpTransport) -> CdpResult<()> {
    transport.send("Log.disable", json!({})).await?;
    Ok(())
}

pub async fn clear(transport: &dyn CdpTransport) -> CdpResult<()> {
    transport.send("Log.clear", json!({})).await?;
    Ok(())
}

pub async fn start_violations_report(
    transport: &dyn CdpTransport,
    config: Vec<ViolationSetting>,
) -> CdpResult<()> {
    transport
        .send(
            "Log.startViolationsReport",
            json!({ "config": config }),
        )
        .await?;
    Ok(())
}

pub async fn stop_violations_report(transport: &dyn CdpTransport) -> CdpResult<()> {
    transport.send("Log.stopViolationsReport", json!({})).await?;
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
        mock.expect("Log.enable", json!({})).await;

        enable(&mock).await.unwrap();
        mock.assert_called_once("Log.enable").await;
    }

    #[tokio::test]
    async fn test_disable() {
        let mock = MockTransport::new();
        mock.expect("Log.disable", json!({})).await;

        disable(&mock).await.unwrap();
        mock.assert_called_once("Log.disable").await;
    }

    #[tokio::test]
    async fn test_clear() {
        let mock = MockTransport::new();
        mock.expect("Log.clear", json!({})).await;

        clear(&mock).await.unwrap();
        mock.assert_called_once("Log.clear").await;
    }

    #[tokio::test]
    async fn test_start_violations_report() {
        let mock = MockTransport::new();
        mock.expect("Log.startViolationsReport", json!({})).await;

        let config = vec![
            ViolationSetting {
                name: "longTask".into(),
                threshold: 200.0,
            },
            ViolationSetting {
                name: "blockedEvent".into(),
                threshold: 100.0,
            },
            ViolationSetting {
                name: "handler".into(),
                threshold: 150.0,
            },
        ];

        start_violations_report(&mock, config).await.unwrap();
        mock.assert_called_once("Log.startViolationsReport").await;

        let params = mock.call_params("Log.startViolationsReport", 0).await.unwrap();
        let config_sent = params["config"].as_array().unwrap();
        assert_eq!(config_sent.len(), 3);
        assert_eq!(config_sent[0]["name"], "longTask");
        assert_eq!(config_sent[0]["threshold"], 200.0);
        assert_eq!(config_sent[1]["name"], "blockedEvent");
        assert_eq!(config_sent[2]["threshold"], 150.0);
    }

    #[tokio::test]
    async fn test_stop_violations_report() {
        let mock = MockTransport::new();
        mock.expect("Log.stopViolationsReport", json!({})).await;

        stop_violations_report(&mock).await.unwrap();
        mock.assert_called_once("Log.stopViolationsReport").await;
    }
}
