//! Central policy engine: every tool call is classified and evaluated before it runs.
//!
//! Before this existed, security lived in scattered point-checks — an SSRF guard in
//! `reach`, an https requirement in `login`, an upload-root check in `reach`. Each is
//! correct, but there was no single place that could answer "is this call allowed?",
//! so every new tool had to remember to re-implement the relevant guard.
//!
//! The engine sits in the MCP dispatch path (see `mcp::handle_tools_call`) between
//! argument validation and execution. It does not replace the point-checks — defence
//! in depth — it decides whether the call happens at all.
//!
//! Design constraints worth stating, because they shaped the defaults:
//!
//! - **A denial must be legible.** A model that gets an opaque failure retries; one
//!   that is told which rule fired and what would satisfy it can adapt or ask.
//! - **`ask` is only useful if someone can answer.** Most MCP clients do not
//!   implement elicitation, so a profile that asks for confirmation on ordinary
//!   actions would simply be a profile that fails. Confirmation is therefore
//!   reserved for the `Safe` profile's elevated classes, and returns a structured
//!   `requires_confirmation` the caller can act on rather than a dead end.
//! - **The default profile does not break existing setups.** `Developer` enforces
//!   the domain lists and logs elevated actions but does not gate them. Users who
//!   want gating opt into `Safe`; unattended agents should use `Autonomous`, which
//!   demands an explicit allowlist.

use std::collections::BTreeSet;

use serde_json::{json, Map, Value};

/// What a tool call actually does, for policy purposes.
///
/// Grouped by consequence rather than by implementation: `js` sits alone because
/// arbitrary script in the page context can impersonate every other class, and
/// `Replay` is elevated because a recorded playbook can contain any mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ActionClass {
    /// Observation only: no page state changes, nothing leaves the machine.
    Read,
    /// Moves the browser somewhere, including tab management.
    Navigate,
    /// Mutates page state — clicks, typing, form submission.
    Interact,
    /// Outbound query to a third-party search provider.
    Search,
    /// Server-side fetch or file write (`browse`, `download`).
    Fetch,
    /// Reads a local file and hands it to a page.
    Upload,
    /// Touches credentials or session material.
    Auth,
    /// Arbitrary script execution, or replay of a recorded mutation sequence.
    Script,
}

impl ActionClass {
    /// Human label used in denial and confirmation messages.
    pub fn label(self) -> &'static str {
        match self {
            ActionClass::Read => "read",
            ActionClass::Navigate => "navigate",
            ActionClass::Interact => "interact",
            ActionClass::Search => "search",
            ActionClass::Fetch => "fetch",
            ActionClass::Upload => "upload",
            ActionClass::Auth => "auth",
            ActionClass::Script => "script",
        }
    }

    /// Classes whose blast radius reaches past the current page: local files,
    /// credentials, arbitrary code. These are what `Safe` gates.
    pub fn is_elevated(self) -> bool {
        matches!(
            self,
            ActionClass::Upload | ActionClass::Auth | ActionClass::Script | ActionClass::Fetch
        )
    }
}

/// Classify a tool by name.
///
/// An unrecognised name resolves to `Script`, the most restrictive class. A tool
/// added without being classified here therefore becomes *more* guarded, not less —
/// the failure mode of forgetting is a denial, never a silent bypass.
pub fn classify(tool: &str) -> ActionClass {
    match tool {
        "status" | "read" | "screenshot" | "find" | "list_tabs" | "page_info" | "console_logs"
        | "network_log" | "metrics" | "debug" | "analyze" | "extract" | "extract_table"
        | "session_info" | "wait" | "observe" | "perf_trace" | "computed_style" | "har_export"
        | "trace_bundle" | "profile_mode" | "list_frames" | "bridge_status" | "cpu_profile"
        | "heap_stats" | "source_map" => ActionClass::Read,

        "navigate" | "new_tab" | "switch_tab" | "close_tab" | "scroll" | "paginate" => {
            ActionClass::Navigate
        }

        "click" | "type" | "fill" | "form_fill" | "submit" | "find_and_click"
        | "dismiss_overlay" | "press" | "hover" | "click_variant" | "set_control" | "drag" => {
            ActionClass::Interact
        }

        "search" | "search_images" | "search_videos" | "search_twitter_videos" => {
            ActionClass::Search
        }

        "browse" | "download" => ActionClass::Fetch,

        // Reads a local file, so it belongs with upload rather than with the read-only
        // debugging tools.
        "har_import" => ActionClass::Upload,

        "upload" => ActionClass::Upload,

        // Composites inherit the worst class of what they do internally.
        "login_flow" => ActionClass::Auth,
        "extract_paginated" => ActionClass::Interact,
        "pierce" | "dialog" | "emulate" => ActionClass::Interact,

        // Raw CDP into the user's real browser is the most powerful thing here.
        "bridge_cdp" => ActionClass::Script,

        "login" | "save_cookies" | "restore_cookies" | "save_session" | "revoke_session" => {
            ActionClass::Auth
        }

        // `js` is arbitrary code. `replay` re-dispatches recorded steps, so it
        // inherits the worst class its playbook could contain. Recording controls
        // themselves change what gets persisted, which can capture credentials.
        "js" | "replay" | "record_task" | "stop_recording" => ActionClass::Script,

        _ => ActionClass::Script,
    }
}

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
    fn from_env_value(v: &str) -> Option<Self> {
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

/// The active policy.
#[derive(Debug, Clone)]
pub struct Policy {
    pub profile: Profile,
    /// Registrable-domain suffixes that are permitted. Empty means "no allowlist".
    allow: BTreeSet<String>,
    /// Suffixes that are refused. Evaluated before the allowlist.
    deny: BTreeSet<String>,
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

/// Extract the destination host from a tool's arguments, if they name one.
///
/// Only a real URL counts. A bare string is not treated as a host: guessing would
/// let a selector like `div.example.com` be read as a destination and either grant
/// or deny access on nonsense.
pub fn target_host(args: &Map<String, Value>) -> Option<String> {
    let raw = args.get("url").and_then(Value::as_str)?;
    reqwest::Url::parse(raw)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
}

/// Comma-separated env var to a set of lowercase host suffixes.
fn domain_set(var: &str) -> BTreeSet<String> {
    std::env::var(var)
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Does `host` equal or sit under any suffix in `set`?
///
/// Matching is label-aware: `evil-example.com` must not match `example.com`, which
/// a plain `ends_with` would wrongly accept.
fn matches_suffix(set: &BTreeSet<String>, host: &str) -> bool {
    set.iter()
        .any(|s| host == s.as_str() || host.ends_with(&format!(".{s}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(profile: Profile, allow: &[&str], deny: &[&str]) -> Policy {
        Policy {
            profile,
            allow: allow.iter().map(|s| s.to_string()).collect(),
            deny: deny.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// Every registered tool must have a deliberate class. This test fails when a
    /// tool is added without classifying it, which is the moment to decide.
    #[test]
    fn every_registered_tool_is_classified() {
        let unclassified: Vec<String> = crate::tool_impls::tool_list()
            .iter()
            .map(|t| t.spec().name.to_string())
            .filter(|name| {
                // `classify` falls back to Script, so an unlisted tool is
                // indistinguishable from a deliberately-Script one by return value
                // alone. Compare against the explicit match arms instead.
                classify(name) == ActionClass::Script
                    && !matches!(
                        name.as_str(),
                        // Deliberately Script: arbitrary code, or replay of arbitrary
                        // recorded mutations, or raw CDP into the user's real browser.
                        "js" | "replay" | "record_task" | "stop_recording" | "bridge_cdp"
                    )
            })
            .collect();
        assert!(
            unclassified.is_empty(),
            "tools missing a policy class (they default to Script): {unclassified:?}"
        );
    }

    #[test]
    fn unknown_tool_defaults_to_the_most_restrictive_class() {
        assert_eq!(classify("some_future_tool"), ActionClass::Script);
        assert!(ActionClass::Script.is_elevated());
    }

    #[test]
    fn developer_profile_allows_everything_without_domain_rules() {
        let p = policy(Profile::Developer, &[], &[]);
        for class in [
            ActionClass::Read,
            ActionClass::Interact,
            ActionClass::Script,
            ActionClass::Auth,
            ActionClass::Upload,
        ] {
            assert_eq!(p.evaluate(class, Some("example.com")), Decision::Allow);
            assert_eq!(p.evaluate(class, None), Decision::Allow);
        }
    }

    #[test]
    fn deny_list_blocks_even_a_read() {
        let p = policy(Profile::Developer, &[], &["internal.corp"]);
        assert!(matches!(
            p.evaluate(ActionClass::Read, Some("internal.corp")),
            Decision::Deny { .. }
        ));
        // Subdomains too.
        assert!(matches!(
            p.evaluate(ActionClass::Read, Some("wiki.internal.corp")),
            Decision::Deny { .. }
        ));
        assert_eq!(
            p.evaluate(ActionClass::Read, Some("example.com")),
            Decision::Allow
        );
    }

    /// The classic suffix-matching bug: `evil-example.com` ends with
    /// `example.com` as a substring but is a different registrable domain.
    #[test]
    fn suffix_matching_respects_label_boundaries() {
        let p = policy(Profile::Developer, &["example.com"], &[]);
        assert_eq!(
            p.evaluate(ActionClass::Read, Some("api.example.com")),
            Decision::Allow
        );
        assert!(matches!(
            p.evaluate(ActionClass::Read, Some("evil-example.com")),
            Decision::Deny { .. }
        ));
        assert!(matches!(
            p.evaluate(ActionClass::Read, Some("examplex.com")),
            Decision::Deny { .. }
        ));
    }

    #[test]
    fn allowlist_is_exclusive_once_set() {
        let p = policy(Profile::Developer, &["example.com"], &[]);
        assert!(matches!(
            p.evaluate(ActionClass::Navigate, Some("other.test")),
            Decision::Deny { .. }
        ));
    }

    #[test]
    fn deny_wins_over_allow() {
        let p = policy(
            Profile::Developer,
            &["example.com"],
            &["secret.example.com"],
        );
        assert!(matches!(
            p.evaluate(ActionClass::Read, Some("secret.example.com")),
            Decision::Deny { .. }
        ));
    }

    #[test]
    fn safe_profile_gates_only_elevated_classes() {
        let p = policy(Profile::Safe, &[], &[]);
        assert_eq!(
            p.evaluate(ActionClass::Read, Some("example.com")),
            Decision::Allow
        );
        assert_eq!(
            p.evaluate(ActionClass::Interact, Some("example.com")),
            Decision::Allow
        );
        for class in [
            ActionClass::Script,
            ActionClass::Auth,
            ActionClass::Upload,
            ActionClass::Fetch,
        ] {
            assert!(
                matches!(
                    p.evaluate(class, Some("example.com")),
                    Decision::RequiresConfirmation { .. }
                ),
                "{class:?} should need confirmation under safe"
            );
        }
    }

    /// An unattended agent with no allowlist has no boundary, so the empty case
    /// must fail closed. Getting this backwards is the whole point of the profile.
    #[test]
    fn autonomous_without_an_allowlist_denies_everything() {
        let p = policy(Profile::Autonomous, &[], &[]);
        assert!(matches!(
            p.evaluate(ActionClass::Read, Some("example.com")),
            Decision::Deny { .. }
        ));
    }

    #[test]
    fn autonomous_allows_listed_destinations_and_refuses_untargetable_mutations() {
        let p = policy(Profile::Autonomous, &["example.com"], &[]);
        assert_eq!(
            p.evaluate(ActionClass::Navigate, Some("example.com")),
            Decision::Allow
        );
        // Reads with no nameable destination stay usable, so an agent can still
        // observe the page it was allowed to open.
        assert_eq!(p.evaluate(ActionClass::Read, None), Decision::Allow);
        // A mutation that names no destination cannot be checked, so it is refused.
        assert!(matches!(
            p.evaluate(ActionClass::Script, None),
            Decision::Deny { .. }
        ));
    }

    #[test]
    fn target_host_only_accepts_real_urls() {
        let mut args = Map::new();
        args.insert("url".into(), json!("https://Example.COM/path?q=1"));
        assert_eq!(target_host(&args).as_deref(), Some("example.com"));

        // A selector that merely looks domain-ish must not be read as a host.
        let mut args = Map::new();
        args.insert("url".into(), json!("div.example.com"));
        assert_eq!(target_host(&args), None);

        assert_eq!(target_host(&Map::new()), None);
    }

    #[test]
    fn payloads_are_typed_and_carry_a_remedy() {
        let deny = Decision::Deny {
            reason: "r".into(),
            remedy: "do x".into(),
        };
        let text = deny.to_payload("navigate", ActionClass::Navigate).unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["status"], "blocked");
        assert_eq!(v["ok"], false);
        assert_eq!(v["tool"], "navigate");
        assert_eq!(v["action_class"], "navigate");
        assert!(v["remedy"].as_str().unwrap().contains("do x"));

        let confirm = Decision::RequiresConfirmation { reason: "r".into() };
        let v: Value =
            serde_json::from_str(&confirm.to_payload("upload", ActionClass::Upload).unwrap())
                .unwrap();
        assert_eq!(v["status"], "requires_confirmation");

        // Allow produces nothing to send: the call simply proceeds.
        assert!(Decision::Allow
            .to_payload("read", ActionClass::Read)
            .is_none());
    }

    #[test]
    fn profile_parsing_falls_back_to_developer_on_nonsense() {
        assert_eq!(Profile::from_env_value("safe"), Some(Profile::Safe));
        assert_eq!(
            Profile::from_env_value("AUTONOMOUS"),
            Some(Profile::Autonomous)
        );
        assert_eq!(Profile::from_env_value("banana"), None);
    }
}
