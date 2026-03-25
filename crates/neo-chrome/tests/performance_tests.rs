//! Unit tests for performance types and scoring logic.
//!
//! These tests validate the data structures, score calculation, and
//! text matching used by the performance module without requiring
//! a live Chrome instance.

use neo_chrome::performance::{
    calculate_scores, AuditFinding, AuditResult, AuditScores, PerformanceInsight, TraceConfig,
    WaitResult,
};

// ─── TraceConfig ───

#[test]
fn trace_config_default() {
    let config = TraceConfig::default();
    assert!(!config.auto_stop);
    assert!(!config.reload);
    assert!(config.file_path.is_none());
}

#[test]
fn trace_config_serialize_roundtrip() {
    let config = TraceConfig {
        auto_stop: true,
        reload: true,
        file_path: Some("/tmp/trace.json".to_string()),
    };

    let json = serde_json::to_string(&config).unwrap();
    let deser: TraceConfig = serde_json::from_str(&json).unwrap();

    assert!(deser.auto_stop);
    assert!(deser.reload);
    assert_eq!(deser.file_path.unwrap(), "/tmp/trace.json");
}

#[test]
fn trace_config_serialize_no_file_path() {
    let config = TraceConfig {
        auto_stop: false,
        reload: false,
        file_path: None,
    };

    let json = serde_json::to_string(&config).unwrap();
    assert!(json.contains("\"file_path\":null"));
}

// ─── AuditScores ───

#[test]
fn audit_scores_serialize_roundtrip() {
    let scores = AuditScores {
        accessibility: 0.8,
        seo: 0.9,
        best_practices: 1.0,
    };

    let json = serde_json::to_string(&scores).unwrap();
    let deser: AuditScores = serde_json::from_str(&json).unwrap();

    assert!((deser.accessibility - 0.8).abs() < f64::EPSILON);
    assert!((deser.seo - 0.9).abs() < f64::EPSILON);
    assert!((deser.best_practices - 1.0).abs() < f64::EPSILON);
}

// ─── AuditFinding ───

#[test]
fn audit_finding_serialize_roundtrip() {
    let finding = AuditFinding {
        category: "seo".into(),
        severity: "error".into(),
        message: "Missing title".into(),
    };

    let json = serde_json::to_string(&finding).unwrap();
    let deser: AuditFinding = serde_json::from_str(&json).unwrap();

    assert_eq!(deser.category, "seo");
    assert_eq!(deser.severity, "error");
    assert_eq!(deser.message, "Missing title");
}

// ─── AuditResult ───

#[test]
fn audit_result_serialize_roundtrip() {
    let result = AuditResult {
        scores: AuditScores {
            accessibility: 0.7,
            seo: 1.0,
            best_practices: 0.9,
        },
        findings: vec![AuditFinding {
            category: "accessibility".into(),
            severity: "error".into(),
            message: "Images without alt".into(),
        }],
    };

    let json = serde_json::to_string(&result).unwrap();
    let deser: AuditResult = serde_json::from_str(&json).unwrap();

    assert!((deser.scores.accessibility - 0.7).abs() < f64::EPSILON);
    assert_eq!(deser.findings.len(), 1);
    assert_eq!(deser.findings[0].message, "Images without alt");
}

// ─── Score calculation ───

#[test]
fn scores_perfect_with_no_findings() {
    let scores = calculate_scores(&[]);
    assert!((scores.accessibility - 1.0).abs() < f64::EPSILON);
    assert!((scores.seo - 1.0).abs() < f64::EPSILON);
    assert!((scores.best_practices - 1.0).abs() < f64::EPSILON);
}

#[test]
fn scores_error_deducts_0_3() {
    let findings = vec![AuditFinding {
        category: "seo".into(),
        severity: "error".into(),
        message: "Missing title".into(),
    }];

    let scores = calculate_scores(&findings);
    assert!((scores.seo - 0.7).abs() < f64::EPSILON);
    assert!((scores.accessibility - 1.0).abs() < f64::EPSILON);
    assert!((scores.best_practices - 1.0).abs() < f64::EPSILON);
}

#[test]
fn scores_warning_deducts_0_1() {
    let findings = vec![AuditFinding {
        category: "accessibility".into(),
        severity: "warning".into(),
        message: "Missing lang".into(),
    }];

    let scores = calculate_scores(&findings);
    assert!((scores.accessibility - 0.9).abs() < f64::EPSILON);
}

#[test]
fn scores_clamp_to_zero() {
    let findings: Vec<AuditFinding> = (0..5)
        .map(|i| AuditFinding {
            category: "seo".into(),
            severity: "error".into(),
            message: format!("Error {i}"),
        })
        .collect();

    let scores = calculate_scores(&findings);
    assert!((scores.seo - 0.0).abs() < f64::EPSILON);
}

#[test]
fn scores_multiple_categories() {
    let findings = vec![
        AuditFinding {
            category: "accessibility".into(),
            severity: "error".into(),
            message: "Images without alt".into(),
        },
        AuditFinding {
            category: "accessibility".into(),
            severity: "warning".into(),
            message: "Missing lang".into(),
        },
        AuditFinding {
            category: "seo".into(),
            severity: "error".into(),
            message: "Missing title".into(),
        },
        AuditFinding {
            category: "best-practices".into(),
            severity: "warning".into(),
            message: "Missing DOCTYPE".into(),
        },
    ];

    let scores = calculate_scores(&findings);
    // accessibility: 1.0 - 0.3 - 0.1 = 0.6
    assert!((scores.accessibility - 0.6).abs() < f64::EPSILON);
    // seo: 1.0 - 0.3 = 0.7
    assert!((scores.seo - 0.7).abs() < f64::EPSILON);
    // best-practices: 1.0 - 0.1 = 0.9
    assert!((scores.best_practices - 0.9).abs() < f64::EPSILON);
}

#[test]
fn scores_info_severity_no_deduction() {
    let findings = vec![AuditFinding {
        category: "seo".into(),
        severity: "info".into(),
        message: "Just a note".into(),
    }];

    let scores = calculate_scores(&findings);
    assert!((scores.seo - 1.0).abs() < f64::EPSILON);
}

// ─── WaitResult ───

#[test]
fn wait_result_found_equality() {
    let a = WaitResult::Found("hello".into());
    let b = WaitResult::Found("hello".into());
    assert_eq!(a, b);
}

#[test]
fn wait_result_timeout_equality() {
    assert_eq!(WaitResult::Timeout, WaitResult::Timeout);
}

#[test]
fn wait_result_found_vs_timeout() {
    assert_ne!(WaitResult::Found("x".into()), WaitResult::Timeout);
}

#[test]
fn wait_result_different_texts() {
    assert_ne!(
        WaitResult::Found("hello".into()),
        WaitResult::Found("world".into())
    );
}

#[test]
fn wait_result_debug() {
    let found = WaitResult::Found("test".into());
    let dbg = format!("{found:?}");
    assert!(dbg.contains("Found"));
    assert!(dbg.contains("test"));

    let timeout = WaitResult::Timeout;
    let dbg = format!("{timeout:?}");
    assert!(dbg.contains("Timeout"));
}

// ─── PerformanceInsight ───

#[test]
fn performance_insight_serialize_roundtrip() {
    let insight = PerformanceInsight {
        lcp_ms: Some(1500.0),
        fcp_ms: Some(800.0),
        cls: Some(0.05),
        long_tasks: 3,
        entries: serde_json::json!([]),
    };

    let json = serde_json::to_string(&insight).unwrap();
    let deser: PerformanceInsight = serde_json::from_str(&json).unwrap();

    assert!((deser.lcp_ms.unwrap() - 1500.0).abs() < f64::EPSILON);
    assert!((deser.fcp_ms.unwrap() - 800.0).abs() < f64::EPSILON);
    assert!((deser.cls.unwrap() - 0.05).abs() < f64::EPSILON);
    assert_eq!(deser.long_tasks, 3);
}

#[test]
fn performance_insight_null_metrics() {
    let insight = PerformanceInsight {
        lcp_ms: None,
        fcp_ms: None,
        cls: None,
        long_tasks: 0,
        entries: serde_json::json!([]),
    };

    let json = serde_json::to_string(&insight).unwrap();
    let deser: PerformanceInsight = serde_json::from_str(&json).unwrap();

    assert!(deser.lcp_ms.is_none());
    assert!(deser.fcp_ms.is_none());
    assert!(deser.cls.is_none());
    assert_eq!(deser.long_tasks, 0);
}

// ─── Text matching logic (unit-testable JS generation) ───

#[test]
fn text_escape_single_quotes() {
    // Verify that single quotes in search text would be escaped.
    let text = "it's a test";
    let escaped = text.replace('\\', "\\\\").replace('\'', "\\'");
    assert_eq!(escaped, "it\\'s a test");
}

#[test]
fn text_escape_backslash() {
    let text = r"path\to\file";
    let escaped = text.replace('\\', "\\\\").replace('\'', "\\'");
    assert_eq!(escaped, "path\\\\to\\\\file");
}
