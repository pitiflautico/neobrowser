//! Navigation state machine.
//!
//! Tracks page lifecycle: Idle -> Navigating -> Loading -> Interactive ->
//! Settled -> Complete. Every transition is traced.

use neo_trace::Tracer;
use neo_types::PageState;

/// Navigation lifecycle state machine.
///
/// Enforces valid transitions and records every state change
/// through the tracer for AI observability.
pub struct Lifecycle {
    state: PageState,
    tracer: Box<dyn Tracer>,
}

impl Lifecycle {
    /// Create a new lifecycle in `Idle` state.
    pub fn new(tracer: Box<dyn Tracer>) -> Self {
        Self {
            state: PageState::Idle,
            tracer,
        }
    }

    /// Transition to a new state with a human-readable reason.
    ///
    /// Records the transition via the tracer.
    pub fn transition(&mut self, to: PageState, reason: &str) {
        let from = self.state;
        self.tracer.state_change(from, to, reason);
        self.state = to;
    }

    /// Current lifecycle state.
    pub fn current(&self) -> PageState {
        self.state
    }

    /// Shared reference to the tracer.
    pub fn tracer(&self) -> &dyn Tracer {
        self.tracer.as_ref()
    }
}
