//! The three profiles, and what a decision can be.
//!
//! `RequiresConfirmation` exists as a distinct outcome from `Deny` because they mean
//! different things to a caller: one is "ask the human", the other is "this will not happen".
//! Collapsing them would make an agent retry something that can never succeed, or worse,
//! treat a refusal as a transient error.

use super::class::ActionClass;
use serde_json::json;

/// How strict the engine is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// Domain lists enforced; elevated actions logged but allowed. The default, so
    /// that adding the engine does not change what already-working setups can do.
    Developer,
    /// Elevated actions require confirmation; a deny-list miss is still a denial.
    Safe,
    /// For unattended agents: nothing outside an explicit allowlist is reachable,
    /// and an empty allowlist denies everything rather than allowing everything.
    Autonomous,
}

impl Profile {
    pub(super) fn from_env_value(v: &str) -> Option<Self> {
        match v.trim().to_ascii_lowercase().as_str() {
            "developer" | "dev" => Some(Profile::Developer),
            "safe" => Some(Profile::Safe),
            "autonomous" | "auto" => Some(Profile::Autonomous),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Profile::Developer => "developer",
            Profile::Safe => "safe",
            Profile::Autonomous => "autonomous",
        }
    }
}

/// The outcome of evaluating one call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    /// Refused. `reason` says which rule fired; `remedy` says what would satisfy it.
    Deny {
        reason: String,
        remedy: String,
    },
    /// Allowed in principle, but a human has to agree first.
    RequiresConfirmation {
        reason: String,
    },
}

impl Decision {
    /// Render a denial or confirmation as the structured payload a caller receives.
    ///
    /// Typed and self-describing on purpose: `status` lets a client branch without
    /// parsing prose, while `reason`/`remedy` give a model enough to change course
    /// instead of retrying the same call.
    pub fn to_payload(&self, tool: &str, class: ActionClass) -> Option<String> {
        match self {
            Decision::Allow => None,
            Decision::Deny { reason, remedy } => Some(
                json!({
                    "ok": false,
                    "status": "blocked",
                    "tool": tool,
                    "action_class": class.label(),
                    "reason": reason,
                    "remedy": remedy,
                })
                .to_string(),
            ),
            Decision::RequiresConfirmation { reason } => Some(
                json!({
                    "ok": false,
                    "status": "requires_confirmation",
                    "tool": tool,
                    "action_class": class.label(),
                    "reason": reason,
                    "remedy": "Re-issue this call after the user approves it, or switch \
                               NEOBROWSER_POLICY to `developer` if this session is \
                               interactive and you accept the risk.",
                })
                .to_string(),
            ),
        }
    }
}
