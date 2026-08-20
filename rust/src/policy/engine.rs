//! The engine: one decision point every call passes through.
//!
//! Centralised on purpose. A policy scattered across tool implementations is a policy with
//! holes in it, and the holes are invisible — each tool looks correct on its own. Here, a
//! tool that forgets to consult the policy cannot, because dispatch consults it before the
//! tool runs.

use std::collections::BTreeSet;

use super::class::ActionClass;
use super::host::{domain_set, matches_suffix};
use super::profile::{Decision, Profile};

/// The active policy.
#[derive(Debug, Clone)]
pub struct Policy {
    pub profile: Profile,
    /// Registrable-domain suffixes that are permitted. Empty means "no allowlist".
    pub(super) allow: BTreeSet<String>,
    /// Suffixes that are refused. Evaluated before the allowlist.
    pub(super) deny: BTreeSet<String>,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            profile: Profile::Developer,
            allow: BTreeSet::new(),
            deny: BTreeSet::new(),
        }
    }
}

impl Policy {
    /// Read the policy from the environment.
    ///
    /// `NEOBROWSER_POLICY` selects the profile; an unrecognised value falls back to
    /// `Developer` rather than erroring, so a typo degrades instead of bricking the
    /// session. `NEOBROWSER_ALLOW_DOMAINS` / `NEOBROWSER_DENY_DOMAINS` are
    /// comma-separated host suffixes.
    pub fn from_env() -> Self {
        let profile = std::env::var("NEOBROWSER_POLICY")
            .ok()
            .and_then(|v| Profile::from_env_value(&v))
            .unwrap_or(Profile::Developer);
        Self {
            profile,
            allow: domain_set("NEOBROWSER_ALLOW_DOMAINS"),
            deny: domain_set("NEOBROWSER_DENY_DOMAINS"),
        }
    }

    /// Is a domain list configured at all? Reported by `doctor`.
    pub fn has_domain_rules(&self) -> bool {
        !self.allow.is_empty() || !self.deny.is_empty()
    }

    pub fn allow_list(&self) -> Vec<&str> {
        self.allow.iter().map(String::as_str).collect()
    }

    pub fn deny_list(&self) -> Vec<&str> {
        self.deny.iter().map(String::as_str).collect()
    }

    /// Evaluate one call. `target` is the host the call is aimed at, when the
    /// arguments name one.
    pub fn evaluate(&self, class: ActionClass, target: Option<&str>) -> Decision {
        // Domain rules first: a blocked destination is blocked whatever the class,
        // including a plain read. Reading is how data gets exfiltrated.
        if let Some(host) = target {
            let host = host.trim().to_ascii_lowercase();
            if matches_suffix(&self.deny, &host) {
                return Decision::Deny {
                    reason: format!("{host} is on NEOBROWSER_DENY_DOMAINS"),
                    remedy: "Choose a different destination, or remove the entry from \
                             NEOBROWSER_DENY_DOMAINS."
                        .into(),
                };
            }
            if !self.allow.is_empty() && !matches_suffix(&self.allow, &host) {
                return Decision::Deny {
                    reason: format!(
                        "{host} is not on NEOBROWSER_ALLOW_DOMAINS, which is set and \
                         therefore exclusive"
                    ),
                    remedy: "Add the host to NEOBROWSER_ALLOW_DOMAINS, or unset it to \
                             allow any destination."
                        .into(),
                };
            }
        }

        match self.profile {
            Profile::Developer => Decision::Allow,

            Profile::Safe if class.is_elevated() => Decision::RequiresConfirmation {
                reason: format!(
                    "the `{}` policy profile requires confirmation for {} actions",
                    self.profile.label(),
                    class.label()
                ),
            },
            Profile::Safe => Decision::Allow,

            // An unattended agent with no allowlist has no boundary at all, so the
            // absence of a list is treated as "nothing is permitted" rather than
            // "everything is". This is the one place where forgetting to configure
            // is loud instead of silent.
            Profile::Autonomous => {
                if self.allow.is_empty() {
                    return Decision::Deny {
                        reason: "the `autonomous` policy profile requires an explicit \
                                 destination allowlist, and NEOBROWSER_ALLOW_DOMAINS is \
                                 empty"
                            .into(),
                        remedy: "Set NEOBROWSER_ALLOW_DOMAINS to the hosts this agent may \
                                 reach, or use NEOBROWSER_POLICY=developer for \
                                 interactive work."
                            .into(),
                    };
                }
                // A call with no nameable destination cannot be checked against the
                // allowlist, so under this profile it is refused unless it is a
                // read. `js` on an already-allowed page is the notable casualty;
                // that is the intended trade for unattended operation.
                if target.is_none() && class != ActionClass::Read {
                    return Decision::Deny {
                        reason: format!(
                            "under the `autonomous` profile a {} action must name its \
                             destination so it can be checked against the allowlist",
                            class.label()
                        ),
                        remedy: "Use a tool that takes an explicit url, or switch to \
                                 NEOBROWSER_POLICY=developer."
                            .into(),
                    };
                }
                Decision::Allow
            }
        }
    }
}
