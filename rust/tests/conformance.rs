//! Conformance suite for **The Verified Action Contract, version 1.0**
//! (`docs/VERIFIED-ACTIONS.md`).
//!
//! One test per scenario in §6 of the specification, named `cN_…` so a failure maps
//! straight back to the table. Each test states the invariant it enforces and what goes
//! wrong in the real world when an implementation gets it wrong — the suite is meant to be
//! readable by someone implementing the contract in another language, for whom "this tests
//! C3" is worthless and "an agent that believes this click landed will spend the rest of
//! the task reasoning about a page that does not exist" is the whole point.
//!
//! **Several scenarios are defined by the forbidden status, not the required one.** C3 is
//! the heart of the specification: a click that does nothing must report `uncertain`, and
//! an implementation that reports `succeeded` there is exactly the class of tool the
//! contract exists to distinguish itself from. Those assertions come first in their tests
//! and say so in their messages.
//!
//! ## How it drives the implementation
//!
//! Through the registered MCP tools, with a real `ToolCtx`, against real Chrome — so the
//! value asserted on is literally the `status` field of the envelope a caller receives, not
//! an internal helper's return value. Where a scenario cannot be expressed at the tool
//! layer it says so and explains why (C9 and C13 need a session the self-healing browser
//! manager would otherwise silently replace).
//!
//! Every envelope the suite touches also passes [`assert_contract_shape`], which enforces
//! §3 (the status is one of exactly six) and §3.1 (`ok` is derived from the status, so
//! `ok: true` alongside `status: "uncertain"` is unrepresentable) on all of them.
//!
//! ## Skips
//!
//! Self-skips when Chrome is absent, printing a SKIP line. §6.1: a skip is not a pass — a
//! conformance claim requires the run to have executed.
//!
//! ## What it found
//!
//! Recorded because it is the argument for writing a suite against a specification rather
//! than against an implementation: on its first run, C2 and C4 failed. An overlay-obstructed
//! click reported `failed` — with the covering element correctly named, so the diagnosis
//! existed but the status a caller switches on called a removable cookie banner a dead end.
//! And a click on a disabled control reported `uncertain`, withholding a diagnosis that was
//! available before the action was even attempted. Both were fixed in `src` rather than
//! adjusted for here; see `page::ClickOutcome::Disabled`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use neobrowser::action::{self, ActionStatus, Budget};
use neobrowser::browser::Browser;
use neobrowser::tools::{ToolCtx, ToolError, ToolOutput};
use neobrowser::{chrome, observe, page};

// --- harness --------------------------------------------------------------------

/// `NEOBROWSER_HOME` is process-global, and so is the shutdown flag C10 sets. Each test
/// holds this for its whole body and gets its own home — two Chromes on one profile would
/// trip the SingletonLock.
static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The budget for scenarios where the budget is *not* the thing under test.
///
/// Deliberately generous. These scenarios assert what a status means, and a machine slow
/// enough to expire the budget turns them into a measurement of the machine instead. That is
/// not a hypothetical: C1 failed exactly once during development, on a host that was
/// compiling in three other processes, and the tool's answer was `uncertain` — the honest
/// one. The contract held; the *test* had encoded an assumption about hardware.
///
/// Boundedness is still verified, just not here: C8 asserts an action returns within its
/// budget on a page that never settles, and it uses a deliberately tiny one. Raising this
/// constant cannot weaken that.
const AMPLE_BUDGET_S: f64 = 15.0;

/// The budget for scenarios that expect *no* change.
///
/// Small on purpose, and safe for the opposite reason to [`AMPLE_BUDGET_S`]. Where the
/// required answer is `uncertain`, a slow machine cannot flip it — waiting longer only makes
/// `uncertain` more certain, since there is nothing to observe either way. So the whole
/// budget is spent every time, and spending fifteen seconds to reach a conclusion that was
/// available in two buys nothing but a suite people stop running.
const NO_CHANGE_BUDGET_S: f64 = 2.0;

/// The six statuses of §3. A closed set: an implementation adding a seventh has written a
/// new specification version, not a conformant extension.
const CLOSED_STATUS_SET: &[&str] = &[
    "succeeded",
    "failed",
    "blocked",
    "needs_human",
    "requires_confirmation",
    "uncertain",
];

/// One scenario's environment: Chrome, a tool context, and the tab everything runs on.
struct Conformance {
    /// Held for the test's whole body: dropping it terminates Chrome.
    browser: Arc<Browser>,
    ctx: ToolCtx,
    tab: Arc<neobrowser::cdp::CdpClient>,
    /// The `navigate` envelope from loading the fixture. C8 and C11 assert on it; the rest
    /// only need it to have happened.
    arrival: Value,
    /// This scenario's `NEOBROWSER_HOME` leaf, e.g. `nb-conf-c3-40912`.
    home_key: String,
}

impl Drop for Conformance {
    /// Reap this scenario's Chrome and delete its profile, unconditionally.
    ///
    /// `Browser` terminates the child it owns, but a test that panics unwinds through code
    /// that was never expecting to be unwound, and a Chrome that survives keeps an
    /// exclusive lock on its user-data dir. The next run then fails to launch with
    /// "profile already in use" — a failure that has nothing to do with the contract and
    /// points at the wrong scenario. Doing it here means it cannot be forgotten, and the
    /// marker is this scenario's own profile path, so nothing else on the machine is
    /// touched.
    fn drop(&mut self) {
        kill_chrome_for(&self.home_key);
        // Deleting the profile is hygiene, and best-effort on purpose: a Chrome being killed
        // writes into its user-data dir on the way out, and can recreate the directory after
        // this removal has already succeeded. Retrying briefly wins the common race; when it
        // does not, a stray directory is harmless, because the home is pid-suffixed and can
        // never be the one a later run tries to launch into.
        let dir = format!("/tmp/{}", self.home_key);
        for _ in 0..10 {
            if std::fs::remove_dir_all(&dir).is_ok() || !std::path::Path::new(&dir).exists() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

fn chrome_available() -> bool {
    chrome::chrome_bin().exists()
}

/// A fixture page under `tests/fixtures/`, as a `file://` URL.
fn fixture(name: &str) -> String {
    format!(
        "file://{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// Launch Chrome under an isolated home and load `page_url` through the `navigate` tool.
///
/// Returns `None` after printing a SKIP line when there is no browser to drive.
async fn arrive(scenario: &str, page_url: &str) -> Option<Conformance> {
    if !chrome_available() {
        eprintln!(
            "SKIP {scenario}: no Chrome binary found. This scenario did NOT run — per §6.1 a \
             skip is not a pass"
        );
        return None;
    }
    // Test hygiene, not part of any scenario: the shutdown flag is process-global, so a
    // sibling test that panicked between `begin_shutdown` and its reset would make every
    // later action here report `cancelled`.
    action::end_shutdown_for_tests();

    // The process id is part of the home so every run gets a genuinely fresh profile. A
    // fixed path is reused across runs, and a Chrome that leaked from an earlier run leaves
    // a `SingletonLock` naming a pid — which the OS eventually recycles onto an unrelated
    // live process, at which point Chrome refuses the profile and the scenario fails for a
    // reason invented weeks earlier.
    let home_key = format!("nb-conf-{scenario}-{}", std::process::id());
    std::env::set_var("NEOBROWSER_HOME", format!("/tmp/{home_key}"));
    let browser = Arc::new(Browser::new());
    let ctx = ToolCtx {
        browser: browser.clone(),
        registry: Arc::new(neobrowser::tool_impls::build_registry()),
        policy: Arc::new(neobrowser::policy::Policy::from_env()),
        trace: Arc::new(neobrowser::trace::Trace::new(format!(
            "trace_conf_{scenario}"
        ))),
        bridge: None,
    };
    let tab = browser
        .tab()
        .await
        .expect("launch Chrome and attach a CDP tab");
    let mut c = Conformance {
        browser,
        ctx,
        tab,
        arrival: Value::Null,
        home_key,
    };
    c.arrival = c
        .envelope("navigate", json!({ "url": page_url, "budget_s": 10.0 }))
        .await;
    assert_contract_shape(scenario, &c.arrival);
    Some(c)
}

impl Conformance {
    /// Call a registered tool exactly as the MCP layer would.
    async fn try_call(&self, tool: &str, args: Value) -> Result<ToolOutput, ToolError> {
        let handler = self
            .ctx
            .registry
            .get(tool)
            .unwrap_or_else(|| panic!("no tool named {tool:?} is registered"));
        let args = args.as_object().cloned().unwrap_or_default();
        handler.call(&self.ctx, &args).await
    }

    /// A tool's text output, panicking on a tool error.
    async fn text(&self, tool: &str, args: Value) -> String {
        match self.try_call(tool, args).await {
            Ok(ToolOutput::Text(s)) => s,
            Ok(ToolOutput::Image { .. }) => panic!("{tool} returned an image, not an envelope"),
            Err(e) => panic!("{tool} returned a tool error: {e}"),
        }
    }

    /// A tool's verified-action envelope, parsed.
    async fn envelope(&self, tool: &str, args: Value) -> Value {
        let raw = self.text(tool, args).await;
        serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("{tool} must return a JSON envelope ({e}): {raw}"))
    }

    /// Evaluate JavaScript in the fixture, for setting up and inspecting page state.
    async fn js(&self, expr: &str) -> Value {
        page::js(&self.tab, expr)
            .await
            .unwrap_or_else(|e| panic!("page JS failed: {e}\n{expr}"))
    }

    /// The text of an element, for asserting an action's real-world effect rather than
    /// only the status that describes it.
    async fn read(&self, selector: &str) -> String {
        page::read_text(&self.tab, selector)
            .await
            .unwrap_or_else(|e| panic!("read {selector} failed: {e}"))
    }
}

// --- envelope helpers -----------------------------------------------------------

fn status_of(envelope: &Value) -> &str {
    envelope
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("envelope has no `status`: {envelope}"))
}

fn detail_of(envelope: &Value) -> &str {
    envelope.get("detail").and_then(Value::as_str).unwrap_or("")
}

fn changes_of(envelope: &Value) -> Vec<&str> {
    envelope["evidence"]["changes"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

fn warnings_of(envelope: &Value) -> Vec<&str> {
    envelope["warnings"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

/// §3 and §3.1, checked on every envelope this suite sees.
///
/// Two properties, both cheap and both load-bearing. The status must be one of exactly six
/// — a tool that invents `"partial"` has silently forked the contract, and a caller
/// switching on the status will fall through to its default branch. And `ok` must be
/// exactly `status == "succeeded"`: if the two are independently assignable then some code
/// path eventually emits `ok: true` beside `status: "uncertain"`, not maliciously but
/// because a default was applied, and every consumer downstream believes the wrong one.
fn assert_contract_shape(scenario: &str, envelope: &Value) {
    let status = status_of(envelope);
    assert!(
        CLOSED_STATUS_SET.contains(&status),
        "{scenario}: `{status}` is not one of the six statuses in §3 ({CLOSED_STATUS_SET:?}). \
         Adding a status is a new specification version, not an extension: {envelope}"
    );
    assert_eq!(
        envelope["ok"],
        Value::Bool(status == "succeeded"),
        "{scenario}: `ok` must be derived from `status`, and only `succeeded` is success. \
         An envelope where the two disagree is the false success this contract exists to \
         make unrepresentable: {envelope}"
    );
}

/// Kill every Chrome belonging to one scenario's profile.
fn kill_chrome_for(home_key: &str) {
    let marker = format!("/tmp/{home_key}/profiles");
    let _ = std::process::Command::new("pkill")
        .args(["-9", "-f", &marker])
        .output();
}

/// Wait until `tab` stops evaluating JavaScript, up to five seconds.
///
/// A fixed sleep after injecting a fault would make C9 and C13 fail on a loaded machine for
/// reasons that have nothing to do with the contract. What those scenarios require is that
/// the session *becomes* unobservable, not that it does so within some particular
/// millisecond; the deadline exists only so a session that never dies fails the test.
async fn await_dead_session(tab: &neobrowser::cdp::CdpClient) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if page::js(tab, "return 1").await.is_err() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

// --- fixtures expressible as data: URLs ------------------------------------------

/// The page C1, C3, C4, C7, C9 and C10 act on. One inert control, one live one, one whose
/// effect is a same-length text change, and one disabled — identical dispatch, four
/// different truths about what happened.
const CONTRACT_PAGE: &str = "data:text/html,\
<html><head><title>Contract</title></head><body>\
<h1 id='heading'>step 2</h1>\
<button id='live' onclick='document.getElementById(\"heading\").textContent=\"Order placed\"'>\
Place order</button>\
<button id='inert'>Inert</button>\
<button id='samelen' onclick='document.getElementById(\"heading\").textContent=\"step 3\"'>\
Advance</button>\
<button id='off' disabled>Pay now</button>\
</body></html>";

/// C2: a cookie banner sitting on top of the button, exactly as a real one does — absolute,
/// high `z-index`, covering the target's centre.
///
/// Colours are named rather than hexadecimal on purpose: a `#` in a `data:` URL starts the
/// fragment, so `#dddddd` silently truncates the rest of the page and the fixture stops
/// obstructing anything.
const OVERLAY_PAGE: &str = "data:text/html,\
<html><head><title>Overlay</title></head><body style='margin:0'>\
<h1 id='heading'>step 2</h1>\
<button id='target' \
style='position:absolute;top:120px;left:100px;width:220px;height:60px' \
onclick='document.getElementById(\"heading\").textContent=\"Order placed\"'>Submit order</button>\
<div id='banner' class='cookie-banner' \
style='position:absolute;top:100px;left:80px;width:320px;height:120px;z-index:9999;\
background:silver'>We value your privacy</div>\
</body></html>";

/// C11: a human gate. The witness records any interaction with the challenge, so the test
/// can assert the tool reported the gate rather than poking at it.
const HUMAN_GATE_PAGE: &str = "data:text/html,\
<html><head><title>Verify you are human</title></head><body>\
<h1>Verify you are human</h1>\
<div class='cf-turnstile' id='gate' \
style='width:300px;height:65px;border:1px solid gray' \
onclick='document.getElementById(\"witness\").textContent=\"tampered\"'>\
Confirm you are a human</div>\
<p id='witness'>untouched</p>\
</body></html>";

// --- C1 -------------------------------------------------------------------------

/// **C1 — a click with a visible effect reports `succeeded`.** Invariant I2: `succeeded`
/// requires an observation before, an observation after, and a detected difference.
///
/// This is the other half of C3, and without it the contract is trivially satisfiable: an
/// implementation that returns `uncertain` for everything never lies, and is also useless.
/// So the test asserts more than the status — it asserts the *evidence*: both observations
/// are non-empty, they differ, and the named change matches what the page actually did. An
/// implementation that reports `succeeded` with an empty before-state has not compared
/// anything; it has guessed correctly, which is indistinguishable from guessing until the
/// day it guesses wrong.
#[tokio::test]
async fn c1_a_click_with_a_visible_effect_succeeds() {
    let _guard = ENV_LOCK.lock().await;
    let Some(c) = arrive("c1", CONTRACT_PAGE).await else {
        return;
    };

    let env = c
        .envelope(
            "click",
            json!({ "selector": "#live", "budget_s": AMPLE_BUDGET_S }),
        )
        .await;
    assert_contract_shape("C1", &env);

    assert_eq!(
        status_of(&env),
        "succeeded",
        "C1: a click whose handler rewrites the heading must report `succeeded`: {env}"
    );

    // I2, spelled out: two real observations and a difference between them.
    let before = env["evidence"]["before"]["state_hash"]
        .as_str()
        .unwrap_or("");
    let after = env["evidence"]["after"]["state_hash"]
        .as_str()
        .unwrap_or("");
    assert!(
        !before.is_empty() && !after.is_empty(),
        "C1: `succeeded` requires an observation on both sides of the action. An empty \
         digest means the page was never observed, and a `succeeded` derived from that is \
         a guess: {env}"
    );
    assert_ne!(
        before, after,
        "C1: the two observations are identical, so nothing was detected and this \
         `succeeded` did not come from comparing them: {env}"
    );
    assert!(
        changes_of(&env).contains(&"text"),
        "C1: the evidence must name what changed, and this click changed text: {env}"
    );

    // And the effect is real, not merely a digest difference.
    assert_eq!(
        c.read("#heading").await,
        "Order placed",
        "C1: the fixture's handler did not run, so the status is describing something else"
    );
}

// --- C2 -------------------------------------------------------------------------

/// **C2 — a click on a target covered by an overlay reports `blocked`, naming what covers
/// it.** Invariant I5: `blocked` names the obstruction.
///
/// Forbidden: `succeeded`. A cookie banner over a Submit button is the single most common
/// obstruction on the web, and the naive implementation dispatches a mouse event at the
/// button's coordinates, the banner receives it, and the tool reports success. The agent
/// then waits for a confirmation page that will never load.
///
/// `blocked` rather than `failed` because the two demand different responses, and I5 is
/// where the value is: "it failed" makes an agent retry the identical click forever,
/// whereas "a `div.cookie-banner` covers the target" tells it to dismiss the banner first.
/// Without the diagnosis, `blocked` is only a slower `failed`.
#[tokio::test]
async fn c2_a_click_on_a_covered_target_is_blocked_with_the_obstruction_named() {
    let _guard = ENV_LOCK.lock().await;
    let Some(c) = arrive("c2", OVERLAY_PAGE).await else {
        return;
    };

    // The premise: the banner really is over the button's centre. Without this the
    // scenario is untested regardless of what the status says.
    let covered = c
        .js(
            "var b = document.getElementById('target').getBoundingClientRect();\
             var hit = document.elementFromPoint(b.left + b.width / 2, b.top + b.height / 2);\
             return hit ? hit.id : '(none)'",
        )
        .await;
    assert_eq!(
        covered,
        Value::String("banner".into()),
        "C2: the fixture is not actually obstructing the target; the element at its centre \
         is {covered}"
    );

    let env = c
        .envelope(
            "click",
            json!({ "selector": "#target", "budget_s": AMPLE_BUDGET_S }),
        )
        .await;
    assert_contract_shape("C2", &env);

    assert_ne!(
        status_of(&env),
        "succeeded",
        "C2 FORBIDDEN: reported `succeeded` for a click the overlay swallowed. The banner \
         received the mouse events, not the button: {env}"
    );
    assert_eq!(
        c.read("#heading").await,
        "step 2",
        "C2: the target's handler must not have run — if it did, this fixture is not \
         obstructing anything"
    );

    // I5: the obstruction is named, and named usefully enough to act on.
    let detail = detail_of(&env);
    assert!(
        detail.contains("cookie-banner") || detail.contains("div."),
        "C2/I5: `blocked` is worth more than `failed` only because it says WHAT is in the \
         way. The detail names nothing actionable: {env}"
    );

    assert_eq!(
        status_of(&env),
        "blocked",
        "C2 requires `blocked`: an obstruction is removable, so the caller's next move is \
         to dismiss it and retry — which is a different instruction from `failed` (the \
         action did not take place, and will not on retry). Observed `{}`: {env}",
        status_of(&env)
    );
}

// --- C3 -------------------------------------------------------------------------

/// **C3 — a click with no effect reports `uncertain`, never `succeeded`.** Invariants I1
/// (`uncertain` is never promoted) and I2 (observation brackets the action).
///
/// This is the scenario the whole specification exists for. Every browser tool can dispatch
/// two mouse events at some coordinates and report that it did; the question is what it
/// says when the page does not react. An implementation answering `succeeded` here is
/// reporting on its own plumbing, and §6.2 names it exactly: *"an implementation that
/// reports `succeeded` where C3 requires `uncertain` is precisely the tool this
/// specification exists to distinguish itself from."*
///
/// The consequence is worse than an error, because an error stops an agent and a false
/// success makes it continue. It proceeds into a page it never changed, every subsequent
/// step reasons from a state that does not exist, and the final report says the task was
/// completed. There is no recovery from a confident wrong answer — which is why a tool that
/// says "I could not tell" is more useful than one that guesses right most of the time.
///
/// The test also pins down *why* `uncertain` was returned. `uncertain` for the wrong reason
/// — because the page could not be observed at all — passes a naive status check while
/// hiding a broken observer, so both observations are asserted non-empty and equal: the
/// page was genuinely seen, twice, and genuinely did not change.
#[tokio::test]
async fn c3_a_click_with_no_effect_is_uncertain_not_succeeded() {
    let _guard = ENV_LOCK.lock().await;
    let Some(c) = arrive("c3", CONTRACT_PAGE).await else {
        return;
    };

    let env = c
        .envelope(
            "click",
            json!({ "selector": "#inert", "budget_s": NO_CHANGE_BUDGET_S }),
        )
        .await;
    assert_contract_shape("C3", &env);

    assert_ne!(
        status_of(&env),
        "succeeded",
        "C3 FORBIDDEN — this is the assertion the specification is built around. The mouse \
         events were delivered to an element with no handler and nothing on the page \
         changed, and this implementation called that success. An agent reading it will \
         continue into a page it never modified, and will report the task as done: {env}"
    );
    assert_eq!(
        env["ok"],
        Value::Bool(false),
        "C3/§3.1: an unverified click must never serialize as ok: {env}"
    );

    assert_eq!(
        status_of(&env),
        "uncertain",
        "C3 requires `uncertain`: the action was dispatched and its outcome could not be \
         observed. That is not a failure — it is the honest answer, and the one that lets a \
         caller retry or escalate. Observed `{}`: {env}",
        status_of(&env)
    );

    // I2: `uncertain` here must mean "observed twice, no difference" — not "never
    // observed". An implementation whose observer is broken reports `uncertain` for
    // everything and would otherwise pass this test while being useless.
    let before = env["evidence"]["before"]["state_hash"]
        .as_str()
        .unwrap_or("");
    let after = env["evidence"]["after"]["state_hash"]
        .as_str()
        .unwrap_or("");
    assert!(
        !before.is_empty() && !after.is_empty(),
        "C3/I2: both observations must be real. Empty digests mean this `uncertain` came \
         from a blind observer, not from a page that did not react — and such an \
         implementation would report `uncertain` for a successful click too: {env}"
    );
    assert_eq!(
        before, after,
        "C3: the observations differ, so something on the page DID change and this fixture \
         is not inert: {env}"
    );
    assert!(
        changes_of(&env).is_empty(),
        "C3: no change may be named when none was detected: {env}"
    );

    // I1: the honest answer must survive being asked again. A retry loop that promotes
    // `uncertain` to `succeeded` on the second look is the exact failure I1 forbids.
    let again = c
        .envelope(
            "click",
            json!({ "selector": "#inert", "budget_s": NO_CHANGE_BUDGET_S }),
        )
        .await;
    assert_contract_shape("C3", &again);
    assert_eq!(
        status_of(&again),
        "uncertain",
        "C3/I1: a second attempt on the same inert element was promoted to `{}`. \
         `uncertain` is never converted into `succeeded` — not on retry, not on a second \
         observation, not by a heuristic: {again}",
        status_of(&again)
    );

    // And the warning tells the caller what to do with it, rather than leaving `uncertain`
    // to be read as a polite success.
    assert!(
        warnings_of(&env)
            .iter()
            .any(|w| w.contains("no_observable_change")),
        "C3: `uncertain` must come with the reason it is uncertain, or callers learn to \
         ignore the status: {env}"
    );
}

// --- C4 -------------------------------------------------------------------------

/// **C4 — a click on a disabled control reports `blocked` or `failed`, never `succeeded`.**
/// Invariant I5, and §2's definition of an obstruction, which names a disabled control
/// explicitly.
///
/// A disabled control is the commonest reason a real form refuses to move: the submit
/// button stays disabled because a required field is empty or a validation is still
/// pending. The distinction C4 draws is between "the page did not react" and "the page
/// could not react, and here is why". Both are honest, but only the second is actionable —
/// an agent told `uncertain` retries the same click, while an agent told the control is
/// disabled goes back and fills the field it missed.
///
/// This is why C4 accepts either `blocked` or `failed` but not `uncertain`: the disabled
/// state is *observable before acting*, so an implementation that reports uncertainty about
/// it has declined to look at information it already had.
#[tokio::test]
async fn c4_a_click_on_a_disabled_control_is_blocked_or_failed() {
    let _guard = ENV_LOCK.lock().await;
    let Some(c) = arrive("c4", CONTRACT_PAGE).await else {
        return;
    };

    // The premise: the control really is disabled, and observably so.
    assert_eq!(
        c.js("return document.getElementById('off').disabled").await,
        Value::Bool(true),
        "C4: the fixture's control is not disabled, so this scenario is untested"
    );

    let env = c
        .envelope(
            "click",
            json!({ "selector": "#off", "budget_s": NO_CHANGE_BUDGET_S }),
        )
        .await;
    assert_contract_shape("C4", &env);

    assert_ne!(
        status_of(&env),
        "succeeded",
        "C4 FORBIDDEN: a disabled control cannot have acted on the click, so `succeeded` is \
         a false success: {env}"
    );

    let status = status_of(&env);
    assert!(
        matches!(status, "blocked" | "failed"),
        "C4 requires `blocked` or `failed`, observed `{status}`. A disabled control is an \
         obstruction under §2, and its disabled state is visible before the action — so \
         reporting mere uncertainty withholds a diagnosis the implementation could have \
         made. The caller retries the identical click instead of fixing the field that \
         keeps the button disabled: {env}"
    );
}

// --- C5 -------------------------------------------------------------------------

/// **C5 — a fill inside an open shadow root reports `succeeded`, never `uncertain`.**
/// Invariant I2.
///
/// The forbidden status is `uncertain` here, which makes C5 the mirror image of C3: this is
/// a *false* uncertainty, and it is just as damaging in the other direction. Every
/// enterprise design system is built on custom elements, so if an implementation's observer
/// cannot cross a shadow boundary then every action inside a web component reports
/// `uncertain` — the tool is honest and unusable, and callers quickly learn to ignore the
/// status field entirely. At that point the whole contract has been thrown away, and C3's
/// guarantee no longer protects anyone.
///
/// This was a real defect in NeoBrowser: the fill worked, and the digest built from
/// `document.querySelectorAll` could not see into the component, so the status said
/// otherwise.
#[tokio::test]
async fn c5_a_fill_inside_a_shadow_root_succeeds() {
    let _guard = ENV_LOCK.lock().await;
    let Some(c) = arrive("c5", &fixture("shadow_form.html")).await else {
        return;
    };

    // The premise: an ordinary selector genuinely cannot reach it.
    assert_eq!(
        c.js("return !!document.querySelector('#email')").await,
        Value::Bool(false),
        "C5: a top-level selector can see the field, so this fixture is not testing shadow \
         DOM"
    );

    let env = c
        .envelope(
            "pierce",
            json!({ "selector": "#email", "action": "fill", "value": "ada@example.test",
                    "budget_s": AMPLE_BUDGET_S }),
        )
        .await;
    assert_contract_shape("C5", &env);

    assert_ne!(
        status_of(&env),
        "uncertain",
        "C5 FORBIDDEN: the fill worked and the implementation could not tell. An observer \
         blind to shadow roots reports `uncertain` for every action inside a web component, \
         and a status that is wrong most of the time is a status nobody reads: {env}"
    );
    assert_eq!(
        status_of(&env),
        "succeeded",
        "C5 requires `succeeded`: the observation must cross an open shadow boundary. \
         Observed `{}`: {env}",
        status_of(&env)
    );

    // And the value really is set, so the status is describing the truth.
    assert_eq!(
        c.js("return document.querySelector('contact-card').shadowRoot\
             .querySelector('#email').value")
            .await,
        Value::String("ada@example.test".into()),
        "C5: the field was not actually filled, so `succeeded` is describing something else"
    );
}

// --- C6 -------------------------------------------------------------------------

/// **C6 — filling a framework-controlled input reports `succeeded`, and the value survives
/// a re-render.** Invariant I10: the status is not derived from the mechanism.
///
/// A controlled input owns its value: the component keeps state, a re-render writes that
/// state back into the DOM, and it also installs its own `value` descriptor on the element
/// so it can tell a real edit from its own writes. The trap is that an automation tool
/// assigning `el.value = x` writes *through that descriptor* — the component's tracker is
/// updated, its change detector sees no divergence, its `onChange` never fires, and its
/// state stays stale. The DOM looks correct. Then the next re-render writes the stale state
/// back and the typed value vanishes.
///
/// This is the worst shape of false success available, because the field visibly holds the
/// right value at the moment the tool reports back. The status is right, the pixels are
/// right, and the form submits the old data. So C6 asserts the status *and* the durability:
/// fill, force a re-render, and require the value to still be there.
///
/// I10 is the invariant because the fix is a different mechanism — the prototype's setter
/// rather than the instance's — and a mechanism change must not by itself change the status.
/// Only the observed outcome may.
#[tokio::test]
async fn c6_a_fill_of_a_framework_controlled_input_survives_a_re_render() {
    let _guard = ENV_LOCK.lock().await;
    let Some(c) = arrive("c6", &fixture("tracked_input.html")).await else {
        return;
    };

    // The premise, proven rather than assumed: the fixture's trap must actually bite. A
    // direct `.value =` assignment goes through the instance descriptor, so the component
    // never learns of it and the next re-render discards it. If this assertion fails the
    // fixture is an ordinary input and C6 tests nothing.
    let discarded = c
        .js("var f = document.getElementById('field');\
             f.value = 'direct assignment';\
             f.dispatchEvent(new Event('input', { bubbles: true }));\
             window.rerender();\
             return JSON.stringify({ value: f.value, state: window.componentState() })")
        .await;
    let discarded: Value = serde_json::from_str(discarded.as_str().unwrap_or("{}")).unwrap();
    assert_eq!(
        discarded["state"],
        Value::String(String::new()),
        "C6: a direct `.value =` assignment reached the component's state, so this fixture \
         is not framework-controlled and the scenario is untested: {discarded}"
    );
    assert_ne!(
        discarded["value"],
        Value::String("direct assignment".into()),
        "C6: the re-render did not discard the un-tracked edit, so the fixture has no trap \
         to defeat: {discarded}"
    );

    let env = c
        .envelope(
            "fill",
            json!({ "selector": "#field", "value": "verified value", "budget_s": AMPLE_BUDGET_S }),
        )
        .await;
    assert_contract_shape("C6", &env);
    assert_eq!(
        status_of(&env),
        "succeeded",
        "C6 requires `succeeded`. Observed `{}`: {env}",
        status_of(&env)
    );

    // The durability requirement, which is the real content of C6: the component must have
    // learned the value, so a re-render preserves it.
    let survived = c
        .js("window.rerender();\
             var f = document.getElementById('field');\
             return JSON.stringify({ value: f.value, state: window.componentState(),\
                                     mirror: document.getElementById('mirror').textContent })")
        .await;
    let survived: Value = serde_json::from_str(survived.as_str().unwrap_or("{}")).unwrap();
    assert_eq!(
        survived["state"],
        Value::String("verified value".into()),
        "C6: the component's state never learned the value, so the fill only changed the \
         DOM. The form will submit the old data and the tool reported success: {survived}"
    );
    assert_eq!(
        survived["value"],
        Value::String("verified value".into()),
        "C6: the value did not survive the re-render — it was written into the DOM and then \
         overwritten by the component's stale state: {survived}"
    );
    assert!(
        survived["mirror"]
            .as_str()
            .unwrap_or("")
            .contains("verified value"),
        "C6: the component never re-rendered from the new value: {survived}"
    );
}

// --- C7 -------------------------------------------------------------------------

/// **C7 — a text change of identical length reports `succeeded`, never `uncertain`.**
/// Invariant I2.
///
/// A regression test promoted to a conformance scenario, because the bug it describes is
/// one any implementation will write. The cheapest possible observation of a page's text is
/// its length, and it is wrong in the case that matters most: a wizard going from `step 2`
/// to `step 3`, a counter from `9 items` to `8 items`, a status from `Pending` to `Shipped`
/// (`Pending` and `Shipped` are both seven characters).
///
/// The failure is silent and it is biased toward exactly the wrong direction: successful
/// progress through a multi-step flow reads as no progress, so the agent retries a step it
/// already completed — double-submitting the form, or clicking "Next" twice and skipping a
/// page. An observation must therefore be sensitive to the *content* of a change, not to
/// its size.
#[tokio::test]
async fn c7_a_same_length_text_change_is_observed_as_a_change() {
    let _guard = ENV_LOCK.lock().await;
    let Some(c) = arrive("c7", CONTRACT_PAGE).await else {
        return;
    };

    // The premise: the fixture's change really is length-preserving, so a length-based
    // observer would see nothing.
    let before_text = c.read("#heading").await;
    assert_eq!(before_text, "step 2");

    let env = c
        .envelope(
            "click",
            json!({ "selector": "#samelen", "budget_s": AMPLE_BUDGET_S }),
        )
        .await;
    assert_contract_shape("C7", &env);

    let after_text = c.read("#heading").await;
    assert_eq!(
        after_text, "step 3",
        "C7: the fixture's handler did not run"
    );
    assert_eq!(
        before_text.len(),
        after_text.len(),
        "C7: the change is not length-preserving, so a length-only observer would catch it \
         and the scenario is untested"
    );

    assert_ne!(
        status_of(&env),
        "uncertain",
        "C7 FORBIDDEN: the page moved from `step 2` to `step 3` and the implementation \
         could not tell. An observer that measures the length of the text rather than \
         hashing it reports every same-length edit as no change — so real progress through \
         a flow looks like a stuck page, and the caller repeats a step it already \
         completed: {env}"
    );
    assert_eq!(
        status_of(&env),
        "succeeded",
        "C7 requires `succeeded`. Observed `{}`: {env}",
        status_of(&env)
    );
    assert!(
        changes_of(&env).contains(&"text"),
        "C7: the evidence must name the text change it detected: {env}"
    );
}

// --- C8 -------------------------------------------------------------------------

/// **C8 — acting on a page that never settles returns within its budget.** Invariant I9:
/// budgets are bounded, decided before the action starts, and honoured.
///
/// The forbidden outcome is not a status, it is a hang. An implementation whose readiness
/// criterion is quiescence — "wait until the DOM stops changing" — never returns on this
/// page, and it does not need an adversarial fixture to reproduce: a live ticker, a
/// carousel, an animated chart, or a polling widget is enough. The tool stops responding,
/// and because there is no deadline there is also no report, so the caller cannot even
/// learn that it is stuck.
///
/// Any of the six statuses is conformant here. What must hold is that a status *arrives*,
/// bounded by the budget that was fixed before the action began. Both phases are checked:
/// arriving at the page (where a settle-wait lives) and acting on it (where a
/// wait-for-change lives).
#[tokio::test]
async fn c8_acting_on_a_page_that_never_settles_returns_within_budget() {
    let _guard = ENV_LOCK.lock().await;
    let url = fixture("never_settles.html");
    let Some(c) = arrive("c8", &url).await else {
        return;
    };

    // The premise: the page really does not settle. Two samples a beat apart must differ.
    let churning = c
        .js(
            "var count = function () { return document.getElementsByTagName('*').length; };\
             var first = count();\
             return new Promise(function (done) {\
               setTimeout(function () { done(count() > first); }, 200);\
             })",
        )
        .await;
    assert_eq!(
        churning,
        Value::Bool(true),
        "C8: the fixture's DOM is not growing, so nothing here would hang a \
         wait-for-quiescence implementation and the scenario is untested"
    );

    // Phase one: arrival. `navigate` waits for the DOM to stop changing, which on this
    // page is never, so only the deadline can end it.
    let started = Instant::now();
    let nav = c
        .envelope("navigate", json!({ "url": url, "budget_s": 1.0 }))
        .await;
    let nav_elapsed = started.elapsed();
    assert_contract_shape("C8", &nav);
    // The bound is deliberately far above the budget rather than just above it. The
    // forbidden outcome is a hang, so any finite bound catches it, and a tight one would
    // fail on a loaded machine for reasons unrelated to the contract.
    assert!(
        nav_elapsed < Duration::from_secs(10),
        "C8/I9: navigating a never-settling page with a 1s budget took {nav_elapsed:?}. \
         The budget is not what ends the wait, which means on a real page with a live \
         ticker this call never returns at all"
    );

    // Phase two: an action on the churning page.
    let started = Instant::now();
    let click = c
        .envelope("click", json!({ "selector": "#inert", "budget_s": 0.5 }))
        .await;
    let click_elapsed = started.elapsed();
    assert_contract_shape("C8", &click);
    assert!(
        click_elapsed < Duration::from_secs(10),
        "C8/I9: clicking on a never-settling page with a 0.5s budget took \
         {click_elapsed:?}, so the action is not bounded by the budget decided before it \
         started: {click}"
    );
}

// --- C9 -------------------------------------------------------------------------

/// **C9 — acting after the browser process is killed produces an error, never a success.**
/// Invariant I4: a dead transport produces errors, never empty successes.
///
/// The forbidden outcome is "any success", and the shape it takes in practice is not a
/// cheerful `succeeded` — it is an empty one. A transport layer that coalesces its errors
/// into default values hands back `null`, `""` or `0`, and none of those is
/// distinguishable, to the caller or to a model, from a page that genuinely evaluated to
/// nothing. `null` handed to a model is a fact about the page as far as the model is
/// concerned.
///
/// So the test asserts three things: the raw operation errors, the envelope built around it
/// reports `failed`, and the evidence on both sides is *empty rather than stale*. A cached
/// last-known-good observation here would be the worst of all outcomes — it would make a
/// dead browser look like a live page that did not react.
///
/// This scenario cannot be driven through the tool layer: `ToolCtx.browser` is
/// self-healing, so asking it for a tab after the kill relaunches Chrome and the dead
/// session is never exercised. The envelope is therefore built directly on the session that
/// died, which is what an in-flight action holds when the browser goes away underneath it.
#[tokio::test]
async fn c9_acting_after_the_browser_is_killed_is_reported_as_an_error() {
    let _guard = ENV_LOCK.lock().await;
    let Some(c) = arrive("c9", CONTRACT_PAGE).await else {
        return;
    };

    // Alive before the fault, or the test proves nothing.
    assert!(
        page::js(&c.tab, "return 1").await.is_ok(),
        "C9: the session was already broken before the kill"
    );

    kill_chrome_for(&c.home_key);
    await_dead_session(&c.tab).await;

    // I4: the raw operation must fail rather than return an empty result.
    let raw = neobrowser::ops::fill(&c.tab, "#inert", "SHOULD NOT LAND").await;
    assert!(
        raw.is_err(),
        "C9/I4: a fill through a dead transport returned Ok({raw:?}). A default or empty \
         value here is indistinguishable from a real result"
    );

    let tab = &c.tab;
    let result = action::perform(&c.tab, "fill", Budget::from_secs(3.0), || async move {
        neobrowser::ops::fill(tab, "#inert", "SHOULD NOT LAND").await
    })
    .await;
    let env = result.to_json();
    assert_contract_shape("C9", &env);

    assert_ne!(
        status_of(&env),
        "succeeded",
        "C9 FORBIDDEN: reported success through a dead browser: {env}"
    );
    assert_eq!(
        status_of(&env),
        "failed",
        "C9 requires the failure to be reported as one. Observed `{}`: {env}",
        status_of(&env)
    );
    assert!(
        !detail_of(&env).is_empty(),
        "C9: the error must carry what went wrong, or the caller cannot tell a dead \
         browser from a missing element: {env}"
    );

    // I3, in the form that matters most: unobservable is EMPTY, never the last known
    // value. A stale digest here would make a dead browser look like a live page.
    for side in ["before", "after"] {
        assert_eq!(
            env["evidence"][side]["state_hash"],
            Value::String(String::new()),
            "C9/I3: the {side} observation is not empty. Returning the last known state \
             when the current state is unavailable makes a dead page look alive: {env}"
        );
    }
    assert!(
        changes_of(&env).is_empty(),
        "C9: no change may be claimed when the page could not be observed at all: {env}"
    );
}

// --- C10 ------------------------------------------------------------------------

/// **C10 — a shutdown that interrupts an in-flight wait is reported as cancellation,
/// promptly.** Invariant I6.
///
/// Two distinct requirements, and the second is the one implementations get wrong.
///
/// *Promptly*: cancellation must be observed by in-flight waits, not only checked between
/// actions. An action holding a two-minute budget that only notices the shutdown when it
/// finishes waiting keeps the process alive for two minutes after the signal, and whatever
/// sent the signal escalates to SIGKILL — so the browser dies mid-command and the profile
/// is left in an unknown state.
///
/// *As cancellation*: an interrupted action must not be reported as a timeout. They look
/// identical from inside the wait loop and they mean opposite things. "The page was slow"
/// sends an operator to investigate a page that was fine; "we stopped waiting because the
/// server was shutting down" tells them nothing was wrong with the page and the action's
/// outcome is simply unknown. Reporting `succeeded` is of course forbidden outright: the
/// wait was cut short, so by definition nobody observed the result.
#[tokio::test]
async fn c10_a_shutdown_cancels_an_in_flight_wait_and_says_so() {
    let _guard = ENV_LOCK.lock().await;
    let Some(c) = arrive("c10", CONTRACT_PAGE).await else {
        return;
    };

    // Nothing will change the page, so this wait would otherwise run the full budget.
    let budget = Budget::from_secs(120.0);
    let flip = tokio::spawn(async {
        tokio::time::sleep(Duration::from_millis(900)).await;
        action::begin_shutdown();
    });

    let tab = &c.tab;
    let started = Instant::now();
    let result = action::perform(&c.tab, "click", budget, || async move {
        page::click_selector(tab, "#inert").await?;
        Ok("mouse events dispatched".to_string())
    })
    .await;
    let elapsed = started.elapsed();
    flip.await.expect("the shutdown task completes");
    // Leave the flag clear for whatever runs next in this binary.
    action::end_shutdown_for_tests();

    let env = result.to_json();
    assert_contract_shape("C10", &env);

    assert!(
        elapsed < Duration::from_secs(10),
        "C10/I6: the action took {elapsed:?} despite a shutdown arriving ~0.9s in with a \
         120s budget. Cancellation is not reaching the in-flight wait, so shutdown would \
         hang for the whole budget and then be killed the hard way"
    );
    assert_ne!(
        status_of(&env),
        "succeeded",
        "C10 FORBIDDEN: the wait was cut short, so nobody observed an outcome: {env}"
    );
    assert_eq!(
        status_of(&env),
        "uncertain",
        "C10: an interrupted action's outcome is unknown, which is what `uncertain` means. \
         Observed `{}`: {env}",
        status_of(&env)
    );

    let warnings = warnings_of(&env);
    assert!(
        warnings.iter().any(|w| w.contains("cancelled")),
        "C10/I6: the report must say it was cancelled: {env}"
    );
    assert!(
        !warnings.iter().any(|w| w.contains("budget_exhausted")),
        "C10 FORBIDDEN: reported as a timeout. `the page was slow` and `we stopped waiting \
         because the server is shutting down` are different facts, and conflating them \
         sends someone debugging a page that was never the problem: {env}"
    );
}

// --- C11 ------------------------------------------------------------------------

/// **C11 — a human gate is reported as `needs_human`, naming the gate, and is not touched.**
/// Invariant I7: human gates are reported, not defeated.
///
/// A captcha is an explicit statement by the site's operator. Reporting `succeeded` for
/// arriving at one is a false success of the ordinary kind — the requested page was never
/// reached, and every step after it reasons about content that was never loaded. But I7 is
/// also a correctness requirement beyond honesty: a tool that attempts to solve, bypass or
/// outsource a gate cannot be safely run by anyone against infrastructure they do not own,
/// which makes it unusable for its legitimate purpose.
///
/// `needs_human` is a distinct status from `blocked` because the remedy is distinct. A rate
/// limit needs backing off; a bot wall needs a different source. A captcha needs *a person*,
/// and collapsing all three into `failed` is what makes an agent retry a blocked request in
/// a loop.
///
/// The fixture records any interaction with the challenge, so "not defeated" is asserted
/// rather than assumed.
#[tokio::test]
async fn c11_a_human_gate_is_reported_as_needs_human_and_left_alone() {
    let _guard = ENV_LOCK.lock().await;
    let Some(c) = arrive("c11", HUMAN_GATE_PAGE).await else {
        return;
    };

    let env = &c.arrival;
    assert_ne!(
        status_of(env),
        "succeeded",
        "C11 FORBIDDEN: arriving at a challenge is not arriving at the requested page: {env}"
    );
    assert_eq!(
        status_of(env),
        "needs_human",
        "C11 requires `needs_human`: a captcha needs a person, which is a different remedy \
         from `blocked` (try another route) and from `failed` (it did not happen). Observed \
         `{}`: {env}",
        status_of(env)
    );

    // The gate must be named. Without the name, `needs_human` tells a caller to fetch a
    // human but not what to put in front of them.
    let warnings = warnings_of(env);
    assert!(
        warnings.iter().any(|w| w.contains("captcha")),
        "C11/I7: the gate must be named in the report: {env}"
    );
    assert!(
        warnings.iter().any(|w| !w.is_empty() && w.contains(':')),
        "C11: a gate must come with a next step, not just a label: {env}"
    );

    // Not defeated: the challenge is still standing and was never interacted with.
    assert_eq!(
        c.js("return document.getElementById('witness').textContent")
            .await,
        Value::String("untouched".into()),
        "C11/I7: something interacted with the challenge. A gate is an explicit statement \
         by the site operator; an implementation that attempts it cannot be run safely by \
         anyone against infrastructure they do not own"
    );
    assert_eq!(
        c.js("return !!document.querySelector('.cf-turnstile')")
            .await,
        Value::Bool(true),
        "C11/I7: the challenge was removed from the page rather than reported"
    );

    // And the vocabulary exists in the status enum rather than being improvised per site.
    assert_eq!(ActionStatus::NeedsHuman.as_str(), "needs_human");
    assert!(
        !ActionStatus::NeedsHuman.is_ok(),
        "C11/§3.1: `needs_human` must never derive to success"
    );
}

// --- C12 ------------------------------------------------------------------------

/// **C12 — a reference invalidated by a re-render either re-resolves correctly or fails,
/// and never acts on a different element.** Invariant I8: references are re-resolved at use.
///
/// This is the quietest bug in the specification and the most expensive. A node handle
/// invalidated between the observation and the action does not raise an error — it
/// addresses whatever now occupies that position. So the tool clicks *something*, the page
/// reacts, the change detector sees a difference, and the action reports `succeeded` with
/// evidence. Every layer agrees, and the wrong button was pressed.
///
/// On any SPA the window for this is not exotic; it is the normal case. A list re-renders
/// after a fetch resolves, the rows shift by one, and `Delete` is where `Archive` used to
/// be. Re-resolution costs a round trip and buys the guarantee that the thing acted on is
/// the thing described.
///
/// The fixture makes acting on the wrong element *detectable*: the two buttons write their
/// own names to a log, and the rebuild swaps their positions as well as invalidating their
/// handles. A silent misfire therefore shows up as the wrong name in the log rather than as
/// nothing at all.
#[tokio::test]
async fn c12_a_reference_invalidated_by_a_re_render_never_acts_on_another_element() {
    let _guard = ENV_LOCK.lock().await;
    let Some(c) = arrive("c12", &fixture("reorder_list.html")).await else {
        return;
    };

    // Obtain a stable reference from an observation, the way a caller does.
    let snap = observe::snapshot(&c.tab, observe::SnapshotMode::Interactive)
        .await
        .expect("snapshot the fixture");
    let archive = snap
        .nodes
        .iter()
        .find(|n| n.name == "Archive")
        .expect("the Archive button is in the observation");
    let reference = archive.reference.clone();
    let handle_before = archive.backend_node_id;
    let first_button_before = snap
        .nodes
        .iter()
        .find(|n| n.role == "button")
        .map(|n| n.name.clone())
        .expect("at least one button");
    assert_eq!(
        first_button_before, "Archive",
        "C12: the fixture must start with Archive first for the swap to be meaningful"
    );

    // Force the re-render that invalidates every handle and swaps the two rows.
    c.js("window.rebuild(); return 1").await;

    let after_rebuild = observe::snapshot(&c.tab, observe::SnapshotMode::Interactive)
        .await
        .expect("snapshot after the rebuild");
    let archive_now = after_rebuild
        .nodes
        .iter()
        .find(|n| n.name == "Archive")
        .expect("Archive still exists after the rebuild");

    // Two premises, both required or the scenario proves nothing. The handles really are
    // invalidated, and the position the old handle described now belongs to the OTHER
    // button — so a cached handle misfires visibly instead of harmlessly.
    assert_ne!(
        archive_now.backend_node_id, handle_before,
        "C12: the node id survived the rebuild, so this fixture does not invalidate \
         handles and the scenario is untested"
    );
    let first_button_after = after_rebuild
        .nodes
        .iter()
        .find(|n| n.role == "button")
        .map(|n| n.name.clone())
        .expect("at least one button after the rebuild");
    assert_eq!(
        first_button_after, "Delete",
        "C12: the rebuild did not swap the buttons, so acting on a stale handle would land \
         on the right element by luck and the scenario is untested"
    );

    // Act on the reference obtained before the rebuild.
    let env = c
        .envelope(
            "click",
            json!({ "ref": reference, "budget_s": AMPLE_BUDGET_S }),
        )
        .await;
    assert_contract_shape("C12", &env);

    // The forbidden outcome: acting on a different element. Checked first, because it is
    // the one that does damage — and note that it is checked against the PAGE, not against
    // the status, since the misfire reports `succeeded` with evidence.
    let log = c.read("#log").await;
    assert_ne!(
        log, "clicked: Delete",
        "C12/I8 FORBIDDEN: the reference `{reference}` described the Archive button and the \
         Delete button ran. A stale handle does not error, it addresses whatever now \
         occupies that position — and every layer above agrees the action succeeded: {env}"
    );

    // Either outcome is conformant, as long as it is reported truthfully.
    match status_of(&env) {
        "succeeded" => assert_eq!(
            log, "clicked: Archive",
            "C12: reported `succeeded` but the page does not show Archive having run: {env}"
        ),
        "failed" => assert_eq!(
            log, "clicked: (nothing)",
            "C12: reported `failed` while something on the page did run: {env}"
        ),
        other => panic!(
            "C12 requires the reference to re-resolve correctly or fail, observed \
             `{other}`: {env}"
        ),
    }

    // And the envelope says which reference it acted on, so the record is auditable
    // afterwards rather than only at the moment of the call.
    assert_eq!(
        env["target"]["ref"],
        Value::String(reference.clone()),
        "C12: the envelope must record the reference it was asked to act on: {env}"
    );
}

// --- C13 ------------------------------------------------------------------------

/// **C13 — observing an unobservable page twice yields empty observations and reports no
/// change.** Invariant I3: unobservable is empty, never stale.
///
/// The forbidden outcome is a fabricated change, and there are two ways to fabricate one.
///
/// The first is staleness: returning the last known state when the current state is
/// unavailable. That single line of defensive coding produces both of the failures I3 names
/// — an action that did nothing looks like an action that worked (the "before" is the real
/// page, the "after" is a cached copy of a page that has since changed), and a dead page
/// looks alive.
///
/// The second is treating the transition *into* unobservability as a difference. Losing
/// sight of the page is not evidence that the page changed, but it is a difference between
/// two values, and a change detector that only compares them will say so — which turns
/// every crashed renderer into a successful action.
///
/// Like C9 this is asserted against the session directly rather than through the tool
/// layer, because the browser manager would relaunch and hide the fault. The fault here is
/// deliberately different from C9's: the process stays alive and only this page goes away,
/// which is the shape a crashed renderer or a navigated-away tab takes.
#[tokio::test]
async fn c13_observing_an_unobservable_page_twice_reports_no_change() {
    let _guard = ENV_LOCK.lock().await;
    let Some(c) = arrive("c13", CONTRACT_PAGE).await else {
        return;
    };

    // The last known good observation: precisely the value a stale implementation would
    // hand back once the page is gone.
    let known_good = action::observe(&c.tab).await;
    assert!(
        !known_good.state_hash.is_empty(),
        "C13: the page was not observable to begin with, so the scenario is untested"
    );

    // Take the page away while leaving Chrome running: open a second tab, then close the
    // one this session is attached to.
    c.browser.new_tab().await.expect("open a second tab");
    c.browser
        .close_tab(0)
        .await
        .expect("close the tab under observation");
    await_dead_session(&c.tab).await;

    assert!(
        page::js(&c.tab, "return 1").await.is_err(),
        "C13: the closed page is still evaluating JavaScript, so it is not unobservable and \
         the scenario is untested"
    );

    let first = action::observe(&c.tab).await;
    let second = action::observe(&c.tab).await;

    assert!(
        first.state_hash.is_empty() && second.state_hash.is_empty(),
        "C13/I3: an unobservable page must observe as EMPTY. Got {first:?} and {second:?}"
    );
    assert_ne!(
        first, known_good,
        "C13/I3 FORBIDDEN: the observation is the last known good value. A cached state \
         makes an action that did nothing look like an action that worked, and makes a dead \
         page look alive"
    );
    assert!(
        action::detect_changes(&first, &second).is_empty(),
        "C13/I3: two empty observations must not compare as changed, or every unobservable \
         page would report a successful action: {:?}",
        action::detect_changes(&first, &second)
    );
    assert!(
        action::detect_changes(&known_good, &first).is_empty(),
        "C13/I3 FORBIDDEN: losing sight of the page was reported as a change. The page \
         becoming unobservable is not evidence that it changed — under this behaviour every \
         crashed renderer produces a `succeeded`: {:?}",
        action::detect_changes(&known_good, &first)
    );
    assert!(
        action::detect_changes(&first, &known_good).is_empty(),
        "C13/I3: and the same must hold in the other direction, or regaining sight of a \
         page would fabricate a change: {:?}",
        action::detect_changes(&first, &known_good)
    );
}
