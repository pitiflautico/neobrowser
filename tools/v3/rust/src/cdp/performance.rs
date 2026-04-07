//! CDP Performance domain — runtime metrics collection.

use super::{CdpResult, CdpTransport};
use serde::{Deserialize, Serialize};
use serde_json::json;

// ── Types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Metric {
    pub name: String,
    pub value: f64,
}

// ── Metric name constants ──────────────────────────────────────────

pub const METRIC_TIMESTAMP: &str = "Timestamp";
pub const METRIC_DOCUMENTS: &str = "Documents";
pub const METRIC_FRAMES: &str = "Frames";
pub const METRIC_JS_EVENT_LISTENERS: &str = "JSEventListeners";
pub const METRIC_NODES: &str = "Nodes";
pub const METRIC_LAYOUT_COUNT: &str = "LayoutCount";
pub const METRIC_RECALC_STYLE_COUNT: &str = "RecalcStyleCount";
pub const METRIC_LAYOUT_DURATION: &str = "LayoutDuration";
pub const METRIC_RECALC_STYLE_DURATION: &str = "RecalcStyleDuration";
pub const METRIC_SCRIPT_DURATION: &str = "ScriptDuration";
pub const METRIC_TASK_DURATION: &str = "TaskDuration";
pub const METRIC_JS_HEAP_USED_SIZE: &str = "JSHeapUsedSize";
pub const METRIC_JS_HEAP_TOTAL_SIZE: &str = "JSHeapTotalSize";

// ── Methods ────────────────────────────────────────────────────────

pub async fn enable(
    transport: &dyn CdpTransport,
    time_domain: Option<&str>,
) -> CdpResult<()> {
    let mut params = json!({});
    if let Some(td) = time_domain {
        params["timeDomain"] = json!(td);
    }
    transport.send("Performance.enable", params).await?;
    Ok(())
}

pub async fn disable(transport: &dyn CdpTransport) -> CdpResult<()> {
    transport.send("Performance.disable", json!({})).await?;
    Ok(())
}

pub async fn get_metrics(transport: &dyn CdpTransport) -> CdpResult<Vec<Metric>> {
    let raw = transport.send("Performance.getMetrics", json!({})).await?;
    let metrics: Vec<Metric> = serde_json::from_value(raw["metrics"].clone())?;
    Ok(metrics)
}

pub async fn set_time_domain(
    transport: &dyn CdpTransport,
    time_domain: &str,
) -> CdpResult<()> {
    transport
        .send("Performance.setTimeDomain", json!({ "timeDomain": time_domain }))
        .await?;
    Ok(())
}

/// Get a specific metric by name from the metrics list.
pub fn find_metric(metrics: &[Metric], name: &str) -> Option<f64> {
    metrics.iter().find(|m| m.name == name).map(|m| m.value)
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
        mock.expect("Performance.enable", json!({})).await;

        enable(&mock, None).await.unwrap();
        mock.assert_called_once("Performance.enable").await;

        let params = mock.call_params("Performance.enable", 0).await.unwrap();
        assert!(params.get("timeDomain").is_none());
    }

    #[tokio::test]
    async fn test_enable_with_time_domain() {
        let mock = MockTransport::new();
        mock.expect("Performance.enable", json!({})).await;

        enable(&mock, Some("timeTicks")).await.unwrap();
        mock.assert_called_once("Performance.enable").await;

        let params = mock.call_params("Performance.enable", 0).await.unwrap();
        assert_eq!(params["timeDomain"], "timeTicks");
    }

    #[tokio::test]
    async fn test_get_metrics() {
        let mock = MockTransport::new();
        mock.expect(
            "Performance.getMetrics",
            json!({
                "metrics": [
                    { "name": "Timestamp", "value": 1234567.89 },
                    { "name": "Documents", "value": 3.0 },
                    { "name": "Nodes", "value": 1542.0 },
                    { "name": "JSHeapUsedSize", "value": 8388608.0 },
                    { "name": "LayoutDuration", "value": 0.0123 },
                    { "name": "ScriptDuration", "value": 0.456 }
                ]
            }),
        )
        .await;

        let metrics = get_metrics(&mock).await.unwrap();
        assert_eq!(metrics.len(), 6);
        assert_eq!(metrics[0].name, "Timestamp");
        assert_eq!(metrics[0].value, 1234567.89);
        assert_eq!(metrics[2].name, "Nodes");
        assert_eq!(metrics[2].value, 1542.0);
        assert_eq!(metrics[3].name, "JSHeapUsedSize");
        assert_eq!(metrics[3].value, 8388608.0);
    }

    #[tokio::test]
    async fn test_find_metric() {
        let metrics = vec![
            Metric { name: "Timestamp".into(), value: 100.0 },
            Metric { name: "Nodes".into(), value: 500.0 },
            Metric { name: "JSHeapUsedSize".into(), value: 1024.0 },
        ];

        assert_eq!(find_metric(&metrics, "Nodes"), Some(500.0));
        assert_eq!(find_metric(&metrics, METRIC_JS_HEAP_USED_SIZE), Some(1024.0));
        assert_eq!(find_metric(&metrics, "NonExistent"), None);
        assert_eq!(find_metric(&metrics, METRIC_LAYOUT_DURATION), None);
    }

    #[tokio::test]
    async fn test_disable() {
        let mock = MockTransport::new();
        mock.expect("Performance.disable", json!({})).await;

        disable(&mock).await.unwrap();
        mock.assert_called_once("Performance.disable").await;
    }
}
