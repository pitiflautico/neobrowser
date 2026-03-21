//! Pipeline contract — phases, errors, budgets, and decisions.
//!
//! Every navigation goes through a sequence of [`PipelinePhase`]s.
//! A [`PipelineContext`] is created at navigate() start and collects
//! [`PipelineDecision`]s as the pipeline progresses.

mod context;

pub use context::PipelineContext;

use std::fmt;

/// Ordered phases of the navigation pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum PipelinePhase {
    /// HTTP request for the page.
    Fetch,
    /// HTML to DOM tree.
    Parse,
    /// Identify inline, external, and module scripts.
    ClassifyScripts,
    /// Pre-fetch ES module imports.
    Prefetch,
    /// Replace heavy modules with lightweight stubs.
    Stub,
    /// Source-level transforms (Promise.allSettled polyfill, etc.).
    Rewrite,
    /// V8 script execution.
    Execute,
    /// React/Vue hydration patches.
    Hydrate,
    /// WOM + structured data extraction.
    Extract,
}

impl fmt::Display for PipelinePhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

/// Errors that can occur within a pipeline phase.
#[derive(Debug, thiserror::Error)]
pub enum PhaseError {
    /// Unrecoverable failure in a phase.
    #[error("phase {phase} failed: {message}")]
    Fatal {
        /// Which phase failed.
        phase: PipelinePhase,
        /// Human-readable description.
        message: String,
    },
    /// Transient failure that may succeed on retry.
    #[error("phase {phase} retryable: {message}")]
    Retryable {
        /// Which phase failed.
        phase: PipelinePhase,
        /// Human-readable description.
        message: String,
        /// How many retries remain.
        retries_left: u32,
    },
    /// Phase exceeded its time budget.
    #[error("phase {phase} timeout after {timeout_ms}ms")]
    Timeout {
        /// Which phase timed out.
        phase: PipelinePhase,
        /// Elapsed milliseconds.
        timeout_ms: u64,
    },
}

/// Per-phase time and resource budgets.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PhaseBudgets {
    /// Global timeout for the entire pipeline (ms). Default: 30 000.
    pub total_ms: u64,
    /// Per-fetch timeout (ms). Default: 5 000.
    pub fetch_ms: u64,
    /// Total JS execution budget (ms). Default: 6 000.
    pub execute_ms: u64,
    /// Module prefetch budget (ms). Default: 8 000.
    pub prefetch_ms: u64,
    /// Maximum scripts to execute. Default: 50.
    pub max_scripts: usize,
    /// Maximum modules to prefetch. Default: 500.
    pub max_modules: usize,
}

impl Default for PhaseBudgets {
    fn default() -> Self {
        Self {
            total_ms: 30_000,
            fetch_ms: 5_000,
            execute_ms: 6_000,
            prefetch_ms: 8_000,
            max_scripts: 50,
            max_modules: 500,
        }
    }
}

/// A recorded decision made during pipeline execution.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum PipelineDecision {
    /// Script was skipped (e.g. telemetry, over budget).
    ScriptSkipped {
        /// Script URL.
        url: String,
        /// Why it was skipped.
        reason: String,
    },
    /// Module was replaced with a lightweight stub.
    ModuleStubbed {
        /// Module URL.
        url: String,
        /// Original size in bytes.
        size_bytes: usize,
    },
    /// A source-level rewrite was applied.
    RewriteApplied {
        /// Script URL.
        url: String,
        /// Name of the transform.
        transform: String,
    },
    /// HTTP cache returned a fresh response.
    CacheHit {
        /// Request URL.
        url: String,
        /// Cache type (disk, memory, etc.).
        cache_type: String,
    },
    /// Cache did not have a usable response.
    CacheMiss {
        /// Request URL.
        url: String,
    },
    /// A hydration patch was considered.
    HydrationPatch {
        /// Patch name.
        patch: String,
        /// Whether it was applied.
        applied: bool,
    },
    /// A phase exceeded its budget.
    Timeout {
        /// Which phase timed out.
        phase: PipelinePhase,
        /// How long it ran (ms).
        elapsed_ms: u64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_phase_budgets_defaults() {
        let b = PhaseBudgets::default();
        assert_eq!(b.total_ms, 30_000);
        assert_eq!(b.fetch_ms, 5_000);
        assert_eq!(b.execute_ms, 6_000);
        assert_eq!(b.prefetch_ms, 8_000);
        assert_eq!(b.max_scripts, 50);
        assert_eq!(b.max_modules, 500);
    }

    #[test]
    fn test_phase_display() {
        assert_eq!(PipelinePhase::Fetch.to_string(), "Fetch");
        assert_eq!(PipelinePhase::Extract.to_string(), "Extract");
    }

    #[test]
    fn test_phase_error_messages() {
        let fatal = PhaseError::Fatal {
            phase: PipelinePhase::Parse,
            message: "bad html".into(),
        };
        assert!(fatal.to_string().contains("Parse"));
        assert!(fatal.to_string().contains("bad html"));

        let retry = PhaseError::Retryable {
            phase: PipelinePhase::Fetch,
            message: "503".into(),
            retries_left: 2,
        };
        assert!(retry.to_string().contains("retryable"));

        let timeout = PhaseError::Timeout {
            phase: PipelinePhase::Execute,
            timeout_ms: 6000,
        };
        assert!(timeout.to_string().contains("6000"));
    }

    #[test]
    fn test_pipeline_decision_variants() {
        let d = PipelineDecision::ScriptSkipped {
            url: "https://analytics.js".into(),
            reason: "telemetry".into(),
        };
        assert!(format!("{d:?}").contains("ScriptSkipped"));

        let d2 = PipelineDecision::ModuleStubbed {
            url: "https://big.js".into(),
            size_bytes: 2_000_000,
        };
        assert!(format!("{d2:?}").contains("2000000"));
    }
}
