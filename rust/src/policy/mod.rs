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
//!
//! Split into [`class`] (what kind of action a tool performs), [`profile`] (the three
//! profiles and what a decision can be), [`engine`] (the single decision point) and
//! [`host`] (label-aware domain matching).

pub mod class;
pub mod engine;
pub mod host;
pub mod profile;

pub use class::{classify, ActionClass};
pub use engine::Policy;
pub use host::target_host;
pub use profile::{Decision, Profile};

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Map, Value};

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
