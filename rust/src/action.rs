//! Verified actions: the universal result envelope, time budgets, and the
//! observe → act → verify loop.
//!
//! The problem this exists to solve: `click` used to return `Clicked` the moment it
//! finished dispatching `mouseReleased`. That is a report about NeoBrowser's own
//! plumbing, not about the page. A model reading it cannot distinguish "the button
//! submitted the form" from "the event landed on a dead element", so it moves on and
//! builds on a step that never happened.
//!
//! So every action here reports what *changed*, and when nothing observable changed
//! it says [`ActionStatus::Uncertain`] rather than success. `Uncertain` is never
//! silently promoted — that promotion is precisely the false success being hunted.
//!
//! Budgets exist for the same honesty reason. The old `navigate` polled
//! `document.readyState` against a hardcoded 15s and then slept a fixed buffer, so a
//! slow site cost the full 15s whether or not it had finished, and a caller had no
//! way to say "I only have 3 seconds". A [`Budget`] is passed in, checked, and
//! reported when it runs out, which turns an opaque stall into a stated deadline.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::cdp::{CdpClient, CdpError};
use crate::page;

/// Terminal state of an action. Mirrors the PRD contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionStatus {
    /// The intended effect was observed.
    Succeeded,
    /// The action could not be performed, and we know why.
    Failed,
    /// A policy or a site wall stopped it.
    Blocked,
    /// Only a person can move this forward (captcha, MFA, an interactive challenge).
    NeedsHuman,
    /// Permitted in principle, awaiting explicit approval.
    RequiresConfirmation,
    /// It was dispatched, but nothing observable changed. **Not** a success.
    Uncertain,
}

impl ActionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ActionStatus::Succeeded => "succeeded",
            ActionStatus::Failed => "failed",
            ActionStatus::Blocked => "blocked",
            ActionStatus::NeedsHuman => "needs_human",
            ActionStatus::RequiresConfirmation => "requires_confirmation",
            ActionStatus::Uncertain => "uncertain",
        }
    }

    /// Only `Succeeded` is success. Spelled out as its own method so no call site
    /// can drift into treating `Uncertain` as good enough.
    pub fn is_ok(self) -> bool {
        matches!(self, ActionStatus::Succeeded)
    }
}

/// Process-wide shutdown flag, consulted by every budget.
///
/// This is how "cancel the active action on SIGTERM" is implemented without threading a
/// cancellation token through forty call sites. Every wait loop in the codebase bounds
/// itself with [`Budget`], so making `Budget::expired` also mean "we are shutting down"
/// cancels all of them at once — cooperatively, at the next poll, rather than by killing
/// a task mid-CDP-call and leaving Chrome in an unknown state.
static SHUTTING_DOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Signal that the process is shutting down. Called from the signal handler.
pub fn begin_shutdown() {
    SHUTTING_DOWN.store(true, Ordering::Relaxed);
}

pub fn is_shutting_down() -> bool {
    SHUTTING_DOWN.load(Ordering::Relaxed)
}

/// Reset the flag.
///
/// For tests only: a real process never un-shuts-down. Exposed (rather than `#[cfg(test)]`)
/// because the fault-injection suite is an integration test in a separate binary, and a
/// test that sets a process-global flag must be able to clear it or it poisons every test
/// that runs after it in the same binary.
pub fn end_shutdown_for_tests() {
    SHUTTING_DOWN.store(false, Ordering::Relaxed);
}

#[cfg(test)]
fn clear_shutdown() {
    end_shutdown_for_tests();
}

/// A time budget for one action, with a deadline callers can propagate.
///
/// Deliberately not a fixed timeout constant: a task with two seconds left should
/// not spend fifteen inside one navigation. `remaining` is what every wait loop
/// bounds itself by, so a slow page yields a stated `budget_exhausted` warning
/// instead of an unexplained stall.
#[derive(Debug, Clone, Copy)]
pub struct Budget {
    deadline: Instant,
}

impl Budget {
    pub fn from_secs(secs: f64) -> Self {
        Self {
            deadline: Instant::now() + Duration::from_secs_f64(secs.max(0.0)),
        }
    }

    /// Default per-action budget when a caller does not specify one.
    pub fn default_action() -> Self {
        Self::from_secs(15.0)
    }

    pub fn remaining(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }

    /// Has the budget run out — or is the process shutting down?
    ///
    /// Folding shutdown in here is deliberate: it means an in-flight action with a 30s
    /// budget stops waiting the moment a SIGTERM arrives, instead of holding the process
    /// open for another half minute. The action then reports `uncertain`, which is the
    /// honest outcome — it was interrupted, so nobody knows whether it took effect.
    pub fn expired(&self) -> bool {
        self.remaining().is_zero() || is_shutting_down()
    }

    /// The shorter of `self` and `d` — for a sub-step that must not outlive the
    /// action, and must not extend it either.
    ///
    /// Returns zero while shutting down, so a `sleep(capped_at(..))` in a poll loop
    /// returns immediately rather than adding one more interval of delay to shutdown.
    pub fn capped_at(&self, d: Duration) -> Duration {
        if is_shutting_down() {
            return Duration::ZERO;
        }
        self.remaining().min(d)
    }
}

/// An observation of the page, cheap enough to take before and after every action.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PageState {
    pub url: String,
    pub title: String,
    /// Structural digest: `elements|text_hash|controls_hash` — element count, a
    /// salted hash of the visible text, and a hash over the interactive elements'
    /// identity and state.
    pub state_hash: String,
}

/// Per-process salt for the state digest.
///
/// Field values are hashed, not stored, but a bare 32-bit hash of a low-entropy
/// value (a 4-digit code in a plain text input) is brute-forceable. Salting with a
/// value that is random per process makes the digest useful only for the
/// before/after comparison it exists for, and useless for recovering content or for
/// matching against a precomputed dictionary.
///
/// `RandomState` is seeded from the OS, which gets randomness without adding a
/// dependency just for this.
fn state_salt() -> u32 {
    use std::hash::{BuildHasher, Hasher};
    static SALT: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *SALT.get_or_init(|| {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write(b"neobrowser-state-digest");
        // Fold to 32 bits: the JS side does FNV-1a in 32-bit arithmetic.
        (h.finish() ^ (h.finish() >> 32)) as u32
    })
}

/// Take a [`PageState`] snapshot. A failure to observe is not fatal — it yields an
/// empty state, which the change detector reads as "cannot tell", producing
/// `Uncertain` rather than a false success.
pub async fn observe(client: &CdpClient) -> PageState {
    // `Snippet::returning` guarantees the expression follows `return` on the same line —
    // the ASI hazard that once made every action report `uncertain`. See `crate::js`.
    let snippet = crate::js::state_digest().with("SALT", &state_salt().to_string());
    let raw = match page::js(client, &snippet.returning()).await {
        Ok(v) => v,
        Err(_) => return PageState::default(),
    };
    let parsed: Value = match &raw {
        Value::String(s) => serde_json::from_str(s).unwrap_or(Value::Null),
        other => other.clone(),
    };
    PageState {
        url: parsed
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        title: parsed
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        state_hash: parsed
            .get("hash")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    }
}

/// What the element being acted on was, for the evidence record.
#[derive(Debug, Clone, Default)]
pub struct TargetDesc {
    pub reference: String,
    pub role: String,
    pub name: String,
}

impl TargetDesc {
    pub fn new(
        reference: impl Into<String>,
        role: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            reference: reference.into(),
            role: role.into(),
            name: name.into(),
        }
    }
}

/// The universal envelope every action returns.
#[derive(Debug, Clone)]
pub struct ActionResult {
    pub status: ActionStatus,
    pub action_id: String,
    pub trace_id: String,
    pub action: String,
    pub target: Option<TargetDesc>,
    pub before: PageState,
    pub after: PageState,
    pub changes: Vec<String>,
    pub retryable: bool,
    pub warnings: Vec<String>,
    /// Human-facing detail — the old free-text result, kept so a person reading a
    /// transcript still gets a sentence rather than only a digest.
    pub detail: String,
}

impl ActionResult {
    pub fn new(action: &str, status: ActionStatus) -> Self {
        Self {
            status,
            action_id: next_id("act"),
            trace_id: next_id("trace"),
            action: action.to_string(),
            target: None,
            before: PageState::default(),
            after: PageState::default(),
            changes: Vec::new(),
            retryable: false,
            warnings: Vec::new(),
            detail: String::new(),
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
    }

    pub fn with_target(mut self, target: TargetDesc) -> Self {
        self.target = Some(target);
        self
    }

    pub fn retryable(mut self, yes: bool) -> Self {
        self.retryable = yes;
        self
    }

    pub fn warn(mut self, w: impl Into<String>) -> Self {
        self.warnings.push(w.into());
        self
    }

    /// Serialize to the wire envelope.
    ///
    /// `ok` is derived from `status` rather than stored, so the two can never
    /// disagree — an envelope claiming `ok: true, status: "uncertain"` is not
    /// representable.
    pub fn to_json(&self) -> Value {
        let mut evidence = json!({
            "before": { "url": self.before.url, "state_hash": self.before.state_hash },
            "after": { "url": self.after.url, "state_hash": self.after.state_hash },
            "changes": self.changes,
        });
        if self.before.title != self.after.title {
            evidence["title_changed"] = json!({
                "before": self.before.title,
                "after": self.after.title,
            });
        }
        let mut out = json!({
            "ok": self.status.is_ok(),
            "status": self.status.as_str(),
            "action": self.action,
            "action_id": self.action_id,
            "trace_id": self.trace_id,
            "evidence": evidence,
            "retryable": self.retryable,
            "warnings": self.warnings,
        });
        if let Some(t) = &self.target {
            out["target"] = json!({ "ref": t.reference, "role": t.role, "name": t.name });
        }
        if !self.detail.is_empty() {
            out["detail"] = json!(self.detail);
        }
        out
    }

    pub fn to_string_pretty(&self) -> String {
        self.to_json().to_string()
    }
}

/// Monotonic, process-unique identifiers.
///
/// A counter rather than a random value: it needs to be unique within a session and
/// cheap, not unguessable, and a counter also makes the ordering of actions in a log
/// readable at a glance.
fn next_id(prefix: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{n:06}")
}

/// Compare two observations and name what changed.
///
/// The vocabulary is deliberately small and mechanical. It is evidence, not
/// interpretation: "navigation" and "dom" are things we measured, whereas "the form
/// was submitted" would be a guess dressed as a fact.
pub fn detect_changes(before: &PageState, after: &PageState) -> Vec<String> {
    let mut changes = Vec::new();
    // An unobservable page (empty digest on either side) yields no changes, which
    // the caller turns into `Uncertain`.
    if before.state_hash.is_empty() || after.state_hash.is_empty() {
        return changes;
    }
    if before.url != after.url {
        changes.push("navigation".to_string());
    }
    if before.title != after.title {
        changes.push("title".to_string());
    }
    let (b, a) = (
        before.state_hash.split('|').collect::<Vec<_>>(),
        after.state_hash.split('|').collect::<Vec<_>>(),
    );
    if b.len() == 3 && a.len() == 3 {
        if b[0] != a[0] {
            changes.push("dom_nodes".to_string());
        }
        if b[1] != a[1] {
            changes.push("text".to_string());
        }
        if b[2] != a[2] {
            changes.push("control_state".to_string());
        }
    } else if before.state_hash != after.state_hash {
        changes.push("dom".to_string());
    }
    changes
}

/// Wait until the page differs from `before`, or the budget runs out.
///
/// Polling with a bounded, growing interval rather than one fixed sleep: a fast page
/// is detected in ~50ms instead of always paying the worst case, and a slow one is
/// not hammered. Returns the final observation and whether a change was seen.
pub async fn wait_for_change(
    client: &CdpClient,
    before: &PageState,
    budget: &Budget,
) -> (PageState, bool) {
    let mut interval = Duration::from_millis(50);
    let mut last = observe(client).await;
    loop {
        if !detect_changes(before, &last).is_empty() {
            return (last, true);
        }
        if budget.expired() {
            return (last, false);
        }
        tokio::time::sleep(budget.capped_at(interval)).await;
        // Bounded backoff: never longer than 400ms, so the deadline stays the thing
        // that ends this loop rather than the sleep granularity.
        interval = (interval * 2).min(Duration::from_millis(400));
        last = observe(client).await;
    }
}

/// The observe → act → verify loop.
///
/// `act` performs the raw effect and returns a human-facing detail string. Whatever
/// it reports, the status here is decided by what the page did:
/// - a change within budget      -> `Succeeded`, with the changes as evidence
/// - no change, budget spent     -> `Uncertain` + a `budget_exhausted` warning
/// - no change, page unreadable  -> `Uncertain`
/// - `act` itself errored        -> `Failed`, retryable
pub async fn perform<F, Fut>(
    client: &CdpClient,
    action: &str,
    budget: Budget,
    act: F,
) -> ActionResult
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<String, CdpError>>,
{
    let before = observe(client).await;
    let detail = match act().await {
        Ok(d) => d,
        Err(e) => {
            let after = observe(client).await;
            let mut r = ActionResult::new(action, ActionStatus::Failed)
                .with_detail(e.to_string())
                .retryable(true);
            r.before = before;
            r.after = after;
            return r;
        }
    };

    let (after, changed) = wait_for_change(client, &before, &budget).await;
    let changes = detect_changes(&before, &after);

    let status = if changed {
        ActionStatus::Succeeded
    } else {
        ActionStatus::Uncertain
    };
    let mut r = ActionResult::new(action, status).with_detail(detail);
    r.before = before;
    r.after = after;
    r.changes = changes;
    if !changed {
        // Retryable: nothing observable happened, so trying again is not obviously
        // harmful — unlike a confirmed submit, where a retry could double-post.
        r.retryable = true;
        if is_shutting_down() {
            // Distinguished from a timeout on purpose: "we stopped waiting because the
            // server is shutting down" is a different fact from "this page is slow", and
            // conflating them would send someone debugging the wrong thing.
            r = r.warn(
                "cancelled: the server began shutting down before the page reacted. \
                 Whether the action took effect is unknown",
            );
        } else if budget.expired() {
            r = r.warn(
                "budget_exhausted: no observable change before the deadline; the action \
                 may still be in flight",
            );
        } else {
            r = r.warn(
                "no_observable_change: the event was dispatched but the page did not \
                 change; do not assume it took effect",
            );
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(url: &str, title: &str, hash: &str) -> PageState {
        PageState {
            url: url.into(),
            title: title.into(),
            state_hash: hash.into(),
        }
    }

    #[test]
    fn identical_states_yield_no_changes() {
        let s = state("https://a.test/", "A", "10|20|abc");
        assert!(detect_changes(&s, &s).is_empty());
    }

    #[test]
    fn each_signal_is_reported_separately() {
        let before = state("https://a.test/", "A", "10|20|abc");
        assert_eq!(
            detect_changes(&before, &state("https://b.test/", "A", "10|20|abc")),
            vec!["navigation"]
        );
        assert_eq!(
            detect_changes(&before, &state("https://a.test/", "B", "10|20|abc")),
            vec!["title"]
        );
        assert_eq!(
            detect_changes(&before, &state("https://a.test/", "A", "11|20|abc")),
            vec!["dom_nodes"]
        );
        assert_eq!(
            detect_changes(&before, &state("https://a.test/", "A", "10|21|abc")),
            vec!["text"]
        );
        assert_eq!(
            detect_changes(&before, &state("https://a.test/", "A", "10|20|xyz")),
            vec!["control_state"]
        );
    }

    /// An unreadable page must not look unchanged — "I could not observe" and
    /// "nothing happened" are different, and only the former is silent here so the
    /// caller reports `Uncertain`.
    #[test]
    fn an_unobservable_state_reports_no_changes() {
        let good = state("https://a.test/", "A", "10|20|abc");
        let blank = PageState::default();
        assert!(detect_changes(&good, &blank).is_empty());
        assert!(detect_changes(&blank, &good).is_empty());
    }

    /// The invariant that gives the envelope its value: `ok` is derived, so an
    /// uncertain action can never serialize as a success.
    #[test]
    fn uncertain_never_serializes_as_ok() {
        for status in [
            ActionStatus::Uncertain,
            ActionStatus::Failed,
            ActionStatus::Blocked,
            ActionStatus::NeedsHuman,
            ActionStatus::RequiresConfirmation,
        ] {
            let r = ActionResult::new("click", status);
            let v = r.to_json();
            assert_eq!(v["ok"], false, "{status:?} must not be ok");
            assert_eq!(v["status"], status.as_str());
        }
        let v = ActionResult::new("click", ActionStatus::Succeeded).to_json();
        assert_eq!(v["ok"], true);
    }

    #[test]
    fn envelope_carries_the_contract_fields() {
        let mut r = ActionResult::new("click", ActionStatus::Succeeded)
            .with_target(TargetDesc::new("e42", "button", "Continue"))
            .with_detail("clicked");
        r.before = state("https://a.test/", "A", "1|1|a");
        r.after = state("https://b.test/", "B", "2|2|b");
        r.changes = vec!["navigation".into()];
        let v = r.to_json();
        assert_eq!(v["action"], "click");
        assert!(v["action_id"].as_str().unwrap().starts_with("act_"));
        assert!(v["trace_id"].as_str().unwrap().starts_with("trace_"));
        assert_eq!(v["target"]["ref"], "e42");
        assert_eq!(v["target"]["role"], "button");
        assert_eq!(v["target"]["name"], "Continue");
        assert_eq!(v["evidence"]["before"]["url"], "https://a.test/");
        assert_eq!(v["evidence"]["after"]["url"], "https://b.test/");
        assert_eq!(v["evidence"]["changes"][0], "navigation");
        assert_eq!(v["evidence"]["title_changed"]["before"], "A");
        assert_eq!(v["detail"], "clicked");
    }

    #[test]
    fn action_ids_are_unique_and_ordered() {
        let a = next_id("act");
        let b = next_id("act");
        assert_ne!(a, b);
        assert!(a < b, "ids should sort in creation order: {a} vs {b}");
    }

    /// F1/B5: a SIGTERM must cancel an action that is still waiting, not let it hold the
    /// process open for the rest of its budget.
    #[test]
    fn shutdown_expires_every_budget_immediately() {
        let _g = crate::env_test_guard();
        clear_shutdown();
        let generous = Budget::from_secs(300.0);
        assert!(
            !generous.expired(),
            "a fresh 5-minute budget is not expired"
        );
        assert!(generous.capped_at(Duration::from_millis(200)) > Duration::ZERO);

        begin_shutdown();
        assert!(
            generous.expired(),
            "shutdown must expire a budget with minutes left on it"
        );
        assert_eq!(
            generous.capped_at(Duration::from_millis(200)),
            Duration::ZERO,
            "a poll loop must not sleep one more interval while shutting down"
        );

        clear_shutdown();
        assert!(!generous.expired(), "the flag must be observable both ways");
    }

    #[test]
    fn budget_reports_expiry_and_caps_sub_waits() {
        let b = Budget::from_secs(0.0);
        assert!(b.expired());
        assert_eq!(b.capped_at(Duration::from_secs(5)), Duration::ZERO);

        let b = Budget::from_secs(10.0);
        assert!(!b.expired());
        // A sub-step asking for 200ms gets 200ms, not the whole budget.
        assert_eq!(
            b.capped_at(Duration::from_millis(200)),
            Duration::from_millis(200)
        );
        // A sub-step asking for more than remains is clamped to what remains.
        assert!(b.capped_at(Duration::from_secs(60)) <= Duration::from_secs(10));
    }

    /// The digest must not carry secrets: it ends up in logs and evidence bundles.
    #[test]
    fn state_js_hashes_values_and_skips_passwords() {
        let js = crate::js::state_digest().with("SALT", "12345").expr();
        assert!(
            js.contains("e.type === 'password'") && js.contains("'P1' : 'P0'"),
            "password fields must contribute only empty-vs-filled, never a hash"
        );
        assert!(
            js.contains("fnv(String(e.value), 12345)"),
            "values must be salt-hashed, never emitted"
        );
        // The raw value must never appear in the returned payload.
        assert!(
            !js.contains("value: e.value") && !js.contains("+ e.value"),
            "a field value must not be concatenated into the digest output"
        );
    }

    /// A control inside a web component must be visible to the digest, or a successful
    /// action in there is downgraded to `uncertain`.
    #[test]
    fn the_digest_crosses_open_shadow_boundaries() {
        let js = crate::js::state_digest().expr();
        assert!(
            js.contains("shadowRoot"),
            "digest must descend into shadow roots"
        );
        assert!(js.contains("depth > 8"), "the descent must be bounded");
        assert!(
            js.contains("shadowText"),
            "shadow text must contribute to the text digest"
        );
    }

    /// The salt is what stops the value hashes from being dictionary-attackable, so
    /// it must be stable within a process (before/after must be comparable) while not
    /// being a hardcoded constant.
    #[test]
    fn state_salt_is_stable_within_the_process() {
        assert_eq!(state_salt(), state_salt());
        let js = crate::js::state_digest()
            .with("SALT", &state_salt().to_string())
            .expr();
        assert!(js.contains(&state_salt().to_string()));
    }
}
