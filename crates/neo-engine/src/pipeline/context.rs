//! Pipeline context — shared mutable state across phases.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use super::{PhaseBudgets, PipelineDecision, PipelinePhase};

/// Monotonic counter for trace IDs within this process.
static TRACE_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Generate a unique trace ID (timestamp + counter).
fn generate_trace_id() -> String {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let seq = TRACE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("t-{ts:x}-{seq:04x}")
}

/// Shared state that flows through all pipeline phases.
///
/// Created at the start of each navigation, collects decisions
/// and tracks timing for the entire pipeline.
#[derive(Debug)]
pub struct PipelineContext {
    /// URL being navigated to.
    pub url: String,
    /// Unique identifier for this pipeline run.
    pub trace_id: String,
    /// Time and resource budgets per phase.
    pub budgets: PhaseBudgets,
    /// Decisions made during pipeline execution.
    pub decisions: Vec<PipelineDecision>,
    /// Current phase.
    pub current_phase: PipelinePhase,
    /// Pipeline start time.
    start: Instant,
}

impl PipelineContext {
    /// Create a new context for navigating to `url`.
    pub fn new(url: &str) -> Self {
        Self {
            url: url.to_string(),
            trace_id: generate_trace_id(),
            budgets: PhaseBudgets::default(),
            decisions: Vec::new(),
            current_phase: PipelinePhase::Fetch,
            start: Instant::now(),
        }
    }

    /// Create a context with custom budgets.
    pub fn with_budgets(url: &str, budgets: PhaseBudgets) -> Self {
        Self {
            url: url.to_string(),
            trace_id: generate_trace_id(),
            budgets,
            decisions: Vec::new(),
            current_phase: PipelinePhase::Fetch,
            start: Instant::now(),
        }
    }

    /// Record a decision made during the pipeline.
    pub fn record(&mut self, decision: PipelineDecision) {
        self.decisions.push(decision);
    }

    /// Advance to the next phase.
    pub fn enter_phase(&mut self, phase: PipelinePhase) {
        self.current_phase = phase;
    }

    /// Elapsed time since the pipeline started (ms).
    pub fn elapsed_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    /// Check whether the total budget has been exceeded.
    pub fn is_over_budget(&self) -> bool {
        self.elapsed_ms() > self.budgets.total_ms
    }

    /// Number of decisions recorded so far.
    pub fn decision_count(&self) -> usize {
        self.decisions.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pipeline_context_created() {
        let ctx = PipelineContext::new("https://example.com");
        assert_eq!(ctx.url, "https://example.com");
        assert!(!ctx.trace_id.is_empty());
        assert!(ctx.trace_id.starts_with("t-"));
        assert_eq!(ctx.current_phase, PipelinePhase::Fetch);
        assert_eq!(ctx.decisions.len(), 0);
    }

    #[test]
    fn test_unique_trace_ids() {
        let a = PipelineContext::new("https://a.com");
        let b = PipelineContext::new("https://b.com");
        assert_ne!(a.trace_id, b.trace_id);
    }

    #[test]
    fn test_pipeline_decision_collected() {
        let mut ctx = PipelineContext::new("https://example.com");
        ctx.record(PipelineDecision::ModuleStubbed {
            url: "https://cdn.example.com/big.js".into(),
            size_bytes: 1_500_000,
        });
        assert_eq!(ctx.decision_count(), 1);
        match &ctx.decisions[0] {
            PipelineDecision::ModuleStubbed { url, size_bytes } => {
                assert!(url.contains("big.js"));
                assert_eq!(*size_bytes, 1_500_000);
            }
            other => panic!("expected ModuleStubbed, got {other:?}"),
        }
    }

    #[test]
    fn test_enter_phase() {
        let mut ctx = PipelineContext::new("https://example.com");
        assert_eq!(ctx.current_phase, PipelinePhase::Fetch);
        ctx.enter_phase(PipelinePhase::Parse);
        assert_eq!(ctx.current_phase, PipelinePhase::Parse);
    }

    #[test]
    fn test_elapsed_and_budget() {
        let ctx = PipelineContext::new("https://example.com");
        // Just created — should not be over budget.
        assert!(!ctx.is_over_budget());
        assert!(ctx.elapsed_ms() < 1000);
    }

    #[test]
    fn test_with_custom_budgets() {
        let budgets = PhaseBudgets {
            total_ms: 10_000,
            fetch_ms: 2_000,
            execute_ms: 3_000,
            prefetch_ms: 4_000,
            max_scripts: 20,
            max_modules: 100,
        };
        let ctx = PipelineContext::with_budgets("https://x.com", budgets);
        assert_eq!(ctx.budgets.total_ms, 10_000);
        assert_eq!(ctx.budgets.max_scripts, 20);
    }
}
