//! CDP performance and wait tools — tracing, audits, and element waits.
//!
//! Extends `ChromeSession` with methods for performance tracing,
//! lightweight Lighthouse-style audits, and waiting for page content.

use crate::session::ChromeSession;
use crate::{ChromeError, Result};
use serde_json::json;

// ─── Types ───

/// Configuration for starting a performance trace.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TraceConfig {
    /// Automatically stop the trace when the page finishes loading.
    pub auto_stop: bool,
    /// Reload the page after starting the trace.
    pub reload: bool,
    /// Optional file path to save the trace JSON.
    pub file_path: Option<String>,
}

impl Default for TraceConfig {
    fn default() -> Self {
        Self {
            auto_stop: false,
            reload: false,
            file_path: None,
        }
    }
}

/// Result of a Lighthouse-style audit.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuditResult {
    /// Numeric scores (0.0–1.0) for each category.
    pub scores: AuditScores,
    /// Individual findings from the audit.
    pub findings: Vec<AuditFinding>,
}

/// Scores for the lightweight audit categories.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuditScores {
    /// Accessibility score (0.0–1.0).
    pub accessibility: f64,
    /// SEO score (0.0–1.0).
    pub seo: f64,
    /// Best practices score (0.0–1.0).
    pub best_practices: f64,
}

/// A single audit finding.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuditFinding {
    /// Category: "accessibility", "seo", or "best-practices".
    pub category: String,
    /// Severity: "error", "warning", or "info".
    pub severity: String,
    /// Human-readable description of the finding.
    pub message: String,
}

/// Result of waiting for text on the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaitResult {
    /// The given text was found on the page.
    Found(String),
    /// The timeout was reached without finding any text.
    Timeout,
}

/// Categories used for CDP Tracing.start.
const TRACE_CATEGORIES: &str =
    "devtools.timeline,v8.execute,disabled-by-default-devtools.timeline";

/// Parsed performance metrics from a trace or the Performance API.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PerformanceInsight {
    /// Largest Contentful Paint in ms (if available).
    pub lcp_ms: Option<f64>,
    /// First Contentful Paint in ms (if available).
    pub fcp_ms: Option<f64>,
    /// Cumulative Layout Shift score (if available).
    pub cls: Option<f64>,
    /// Long tasks (>50ms) count.
    pub long_tasks: usize,
    /// Raw entries for further analysis.
    pub entries: serde_json::Value,
}

// ─── Implementation ───

impl ChromeSession {
    // ─── 1. performance_start_trace ───

    /// Start a performance trace via CDP `Tracing.start`.
    ///
    /// If `config.reload` is true, reloads the page after starting.
    /// If `config.auto_stop` is true, waits for load then stops automatically.
    /// If `config.file_path` is set, saves the trace to that path on stop.
    pub async fn performance_start_trace(&self, config: &TraceConfig) -> Result<()> {
        // Enable Tracing domain.
        self.cdp
            .send_to(
                &self.page_session_id,
                "Tracing.start",
                Some(json!({
                    "categories": TRACE_CATEGORIES,
                    "transferMode": "ReturnAsStream",
                })),
            )
            .await?;

        if config.reload {
            self.cdp
                .send_to(
                    &self.page_session_id,
                    "Page.reload",
                    Some(json!({ "ignoreCache": false })),
                )
                .await?;
        }

        if config.auto_stop {
            // Wait for load then stop.
            self.wait_for_load_internal(15_000).await?;
            self.performance_stop_trace(config.file_path.as_deref())
                .await?;
        }

        Ok(())
    }

    // ─── 2. performance_stop_trace ───

    /// Stop an active performance trace and optionally save to file.
    ///
    /// Uses `Runtime.evaluate` with `performance.getEntries()` to collect
    /// timing data (simpler than streaming Tracing events).
    /// Calls `Tracing.end` to stop CDP-level tracing.
    pub async fn performance_stop_trace(&self, file_path: Option<&str>) -> Result<serde_json::Value> {
        // Stop CDP tracing.
        let _ = self
            .cdp
            .send_to(&self.page_session_id, "Tracing.end", None)
            .await;

        // Collect performance entries via the Performance API.
        let entries_json = self
            .eval("JSON.stringify(performance.getEntries())")
            .await?;

        let entries: serde_json::Value =
            serde_json::from_str(&entries_json).unwrap_or(serde_json::Value::Array(vec![]));

        // Save to file if requested.
        if let Some(path) = file_path {
            let data = serde_json::to_string_pretty(&entries)
                .map_err(ChromeError::Json)?;
            tokio::fs::write(path, data).await.map_err(ChromeError::Io)?;
        }

        Ok(entries)
    }

    // ─── 3. performance_analyze_insight ───

    /// Analyze performance entries and extract key web vitals.
    ///
    /// Collects metrics via the Performance API and parses out:
    /// - LCP (Largest Contentful Paint)
    /// - FCP (First Contentful Paint)
    /// - CLS (Cumulative Layout Shift)
    /// - Long task count (>50ms)
    pub async fn performance_analyze_insight(&self) -> Result<PerformanceInsight> {
        let js = r#"JSON.stringify({
            entries: performance.getEntries(),
            lcp: (function() {
                var e = performance.getEntriesByType('largest-contentful-paint');
                return e.length > 0 ? e[e.length - 1].startTime : null;
            })(),
            fcp: (function() {
                var e = performance.getEntriesByType('paint')
                    .filter(function(p) { return p.name === 'first-contentful-paint'; });
                return e.length > 0 ? e[0].startTime : null;
            })(),
            cls: (function() {
                var e = performance.getEntriesByType('layout-shift');
                var sum = 0;
                for (var i = 0; i < e.length; i++) {
                    if (!e[i].hadRecentInput) sum += e[i].value;
                }
                return e.length > 0 ? sum : null;
            })(),
            longTasks: performance.getEntriesByType('longtask').length
        })"#;

        let result_str = self.eval(js).await?;
        let parsed: serde_json::Value =
            serde_json::from_str(&result_str).unwrap_or(json!({}));

        Ok(PerformanceInsight {
            lcp_ms: parsed.get("lcp").and_then(|v| v.as_f64()),
            fcp_ms: parsed.get("fcp").and_then(|v| v.as_f64()),
            cls: parsed.get("cls").and_then(|v| v.as_f64()),
            long_tasks: parsed
                .get("longTasks")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize,
            entries: parsed.get("entries").cloned().unwrap_or(json!([])),
        })
    }

    // ─── 4. lighthouse_audit ───

    /// Run a lightweight Lighthouse-style audit on the current page.
    ///
    /// Checks:
    /// - Navigation timing for performance
    /// - Meta viewport for mobile-friendliness
    /// - Image alt attributes for accessibility
    /// - Title and meta description for SEO
    pub async fn lighthouse_audit(&self) -> Result<AuditResult> {
        let js = r#"JSON.stringify({
            hasViewport: !!document.querySelector('meta[name="viewport"]'),
            hasTitle: document.title.length > 0,
            hasMetaDesc: !!document.querySelector('meta[name="description"]'),
            totalImages: document.querySelectorAll('img').length,
            imagesWithoutAlt: document.querySelectorAll('img:not([alt])').length,
            hasDoctype: document.doctype !== null,
            hasLangAttr: document.documentElement.hasAttribute('lang'),
            hasCharset: !!document.querySelector('meta[charset]') ||
                        !!document.querySelector('meta[http-equiv="Content-Type"]'),
            navTiming: (function() {
                var e = performance.getEntriesByType('navigation');
                if (e.length === 0) return null;
                var n = e[0];
                return {
                    domContentLoaded: n.domContentLoadedEventEnd,
                    load: n.loadEventEnd,
                    ttfb: n.responseStart
                };
            })()
        })"#;

        let result_str = self.eval(js).await?;
        let data: serde_json::Value =
            serde_json::from_str(&result_str).unwrap_or(json!({}));

        let mut findings = Vec::new();

        // ─── Accessibility checks ───
        let total_images = data.get("totalImages").and_then(|v| v.as_u64()).unwrap_or(0);
        let images_no_alt = data.get("imagesWithoutAlt").and_then(|v| v.as_u64()).unwrap_or(0);

        if images_no_alt > 0 {
            findings.push(AuditFinding {
                category: "accessibility".into(),
                severity: "error".into(),
                message: format!(
                    "{images_no_alt} of {total_images} images missing alt attribute"
                ),
            });
        }

        let has_lang = data.get("hasLangAttr").and_then(|v| v.as_bool()).unwrap_or(false);
        if !has_lang {
            findings.push(AuditFinding {
                category: "accessibility".into(),
                severity: "warning".into(),
                message: "Document missing lang attribute on <html>".into(),
            });
        }

        // ─── SEO checks ───
        let has_title = data.get("hasTitle").and_then(|v| v.as_bool()).unwrap_or(false);
        if !has_title {
            findings.push(AuditFinding {
                category: "seo".into(),
                severity: "error".into(),
                message: "Page has no title".into(),
            });
        }

        let has_meta_desc = data.get("hasMetaDesc").and_then(|v| v.as_bool()).unwrap_or(false);
        if !has_meta_desc {
            findings.push(AuditFinding {
                category: "seo".into(),
                severity: "warning".into(),
                message: "Missing meta description".into(),
            });
        }

        let has_viewport = data.get("hasViewport").and_then(|v| v.as_bool()).unwrap_or(false);
        if !has_viewport {
            findings.push(AuditFinding {
                category: "seo".into(),
                severity: "error".into(),
                message: "Missing viewport meta tag (not mobile-friendly)".into(),
            });
        }

        // ─── Best practices checks ───
        let has_doctype = data.get("hasDoctype").and_then(|v| v.as_bool()).unwrap_or(false);
        if !has_doctype {
            findings.push(AuditFinding {
                category: "best-practices".into(),
                severity: "warning".into(),
                message: "Document missing DOCTYPE".into(),
            });
        }

        let has_charset = data.get("hasCharset").and_then(|v| v.as_bool()).unwrap_or(false);
        if !has_charset {
            findings.push(AuditFinding {
                category: "best-practices".into(),
                severity: "warning".into(),
                message: "Missing charset declaration".into(),
            });
        }

        // ─── Calculate scores ───
        let scores = calculate_scores(&findings);

        Ok(AuditResult { scores, findings })
    }

    // ─── 5. wait_for ───

    /// Wait for any of the given texts to appear in the page body.
    ///
    /// Polls every 100ms until one of the texts is found or the timeout
    /// is reached. Default timeout: 30000ms.
    pub async fn wait_for(
        &self,
        texts: &[&str],
        timeout_ms: Option<u64>,
    ) -> Result<WaitResult> {
        let timeout = timeout_ms.unwrap_or(30_000);
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout);

        // Build a JS expression that checks all texts.
        let checks: Vec<String> = texts
            .iter()
            .map(|t| {
                let escaped = t.replace('\\', "\\\\").replace('\'', "\\'");
                format!("document.body.innerText.includes('{escaped}')")
            })
            .collect();

        // Returns the index of the first matching text, or -1.
        let js = format!(
            "(function() {{ var checks = [{}]; for (var i = 0; i < checks.length; i++) {{ if (checks[i]) return i; }} return -1; }})()",
            checks.join(",")
        );

        loop {
            if std::time::Instant::now() > deadline {
                return Ok(WaitResult::Timeout);
            }

            let result = self.eval(&js).await?;
            if let Ok(idx) = result.parse::<i64>() {
                if idx >= 0 && (idx as usize) < texts.len() {
                    return Ok(WaitResult::Found(texts[idx as usize].to_string()));
                }
            }

            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    // ─── Internal helpers ───

    /// Internal wait-for-load used by trace auto_stop.
    async fn wait_for_load_internal(&self, timeout_ms: u64) -> Result<()> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            if std::time::Instant::now() > deadline {
                return Err(ChromeError::Timeout("Page load timed out".into()));
            }
            let state = self.eval("document.readyState").await?;
            if state == "complete" || state == "\"complete\"" {
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }
}

// ─── Score calculation ───

/// Calculate scores from audit findings.
///
/// Each category starts at 1.0. Errors deduct 0.3, warnings deduct 0.1.
/// Scores are clamped to [0.0, 1.0].
pub fn calculate_scores(findings: &[AuditFinding]) -> AuditScores {
    let mut accessibility = 1.0_f64;
    let mut seo = 1.0_f64;
    let mut best_practices = 1.0_f64;

    for f in findings {
        let penalty = match f.severity.as_str() {
            "error" => 0.3,
            "warning" => 0.1,
            _ => 0.0,
        };

        match f.category.as_str() {
            "accessibility" => accessibility -= penalty,
            "seo" => seo -= penalty,
            "best-practices" => best_practices -= penalty,
            _ => {}
        }
    }

    AuditScores {
        accessibility: accessibility.clamp(0.0, 1.0),
        seo: seo.clamp(0.0, 1.0),
        best_practices: best_practices.clamp(0.0, 1.0),
    }
}
