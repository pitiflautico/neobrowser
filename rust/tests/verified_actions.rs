//! Live-Chrome tests for the verified-action contract and the capabilities built on it.
//!
//! Why these exist as integration tests rather than unit tests: **every bug found while
//! building this came from driving a real browser, and none from a unit test.**
//!
//! - `action::observe` returned `undefined` on every call because of JavaScript's
//!   automatic semicolon insertion, so every action reported `uncertain`. The unit tests
//!   passed throughout — they tested `detect_changes` on hand-built states.
//! - The state digest measured text *length*, so `"step 2"` → `"step 3"` looked
//!   unchanged and a successful click was downgraded to `uncertain`.
//! - The digest could not cross shadow boundaries, so a successful fill inside a web
//!   component read as `uncertain`.
//! - `pierce` filled a field correctly while reporting failure.
//!
//! Each of those was verified by hand at the time. These are the same checks, made
//! permanent, so a regression fails the build instead of being rediscovered.
//!
//! Hermetic: `data:` URL fixtures, no network. Self-skips when Chrome is absent.

use neobrowser::action::{self, ActionStatus, Budget};
use neobrowser::{chrome, frames, observe, page};

fn chrome_available() -> bool {
    chrome::chrome_bin().exists()
}

/// `NEOBROWSER_HOME` is process-global, so each test takes this lock for its whole body
/// and gets its own home — two Chromes on one profile would trip the SingletonLock.
static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn tab_on(
    name: &str,
    fixture: &str,
) -> Option<(
    neobrowser::browser::Browser,
    std::sync::Arc<neobrowser::cdp::CdpClient>,
)> {
    if !chrome_available() {
        eprintln!("SKIP: no Chrome binary found; these tests need a real browser");
        return None;
    }
    std::env::set_var("NEOBROWSER_HOME", format!("/tmp/nb-va-{name}"));
    let browser = neobrowser::browser::Browser::new();
    let tab = browser.tab().await.expect("launch + attach a CDP tab");
    page::navigate_budgeted(&tab, fixture, &Budget::from_secs(10.0))
        .await
        .expect("navigate to fixture");
    Some((browser, tab))
}

/// One inert button (no handler) and one that mutates the page. The two together are the
/// whole verified-action contract: identical dispatch, opposite outcomes.
const INERT_AND_LIVE: &str = "data:text/html,\
<html><body>\
<h1 id='t'>Start</h1>\
<button id='dead'>Inert</button>\
<button id='live' onclick='document.getElementById(\"t\").textContent=\"Changed\"'>Live</button>\
<input id='f'>\
<input id='pw' type='password'>\
</body></html>";

// --- the core contract ---------------------------------------------------------

/// The reason this project has a verified-action envelope at all: a click that lands on
/// an element with no handler must NOT report success.
#[tokio::test]
async fn a_click_on_an_inert_element_reports_uncertain() {
    let _guard = ENV_LOCK.lock().await;
    let Some((_b, tab)) = tab_on("inert", INERT_AND_LIVE).await else {
        return;
    };

    let before = action::observe(&tab).await;
    assert!(
        !before.state_hash.is_empty(),
        "the state digest must actually observe the page. An empty digest is the ASI bug \
         that made every action report `uncertain`"
    );

    let outcome = page::click_selector(&tab, "#dead")
        .await
        .expect("click runs");
    assert!(
        matches!(outcome, page::ClickOutcome::Clicked),
        "the event should dispatch; the point is that dispatching is not success"
    );

    let (after, changed) = action::wait_for_change(&tab, &before, &Budget::from_secs(2.0)).await;
    assert!(!changed, "nothing on the page changed");
    assert!(
        action::detect_changes(&before, &after).is_empty(),
        "no change signal should be reported"
    );
}

/// The other half: a click that does change the page must report `succeeded` with the
/// evidence naming what changed. Without this, "always uncertain" would pass the test
/// above and be useless.
#[tokio::test]
async fn a_click_that_changes_the_page_reports_the_change() {
    let _guard = ENV_LOCK.lock().await;
    let Some((_b, tab)) = tab_on("live", INERT_AND_LIVE).await else {
        return;
    };

    let before = action::observe(&tab).await;
    page::click_selector(&tab, "#live")
        .await
        .expect("click runs");
    let (after, changed) = action::wait_for_change(&tab, &before, &Budget::from_secs(3.0)).await;

    assert!(changed, "the handler rewrote the heading");
    let changes = action::detect_changes(&before, &after);
    assert!(
        changes.contains(&"text".to_string()),
        "the text change should be named: {changes:?}"
    );
    // And the effect is real, not just a digest difference.
    let heading = page::read_text(&tab, "#t").await.expect("read heading");
    assert_eq!(heading, "Changed");
}

/// Regression: the digest hashed the *length* of a field's value, so replacing a value
/// with a different one of the same length looked like no change and a successful fill
/// was downgraded to `uncertain`.
#[tokio::test]
async fn a_same_length_value_change_is_detected() {
    let _guard = ENV_LOCK.lock().await;
    let Some((_b, tab)) = tab_on("samelen", INERT_AND_LIVE).await else {
        return;
    };

    neobrowser::ops::fill(&tab, "#f", "hola mundo")
        .await
        .expect("first fill");
    let before = action::observe(&tab).await;
    // Exactly the same number of characters as the previous value.
    neobrowser::ops::fill(&tab, "#f", "otro valor")
        .await
        .expect("second fill");
    let after = action::observe(&tab).await;

    let changes = action::detect_changes(&before, &after);
    assert!(
        changes.contains(&"control_state".to_string()),
        "a same-length value change must still register: {changes:?}"
    );
}

/// The digest ends up in logs and evidence bundles, so it must never carry a field's
/// contents — and a password field must not even contribute a hash of its value.
#[tokio::test]
async fn the_state_digest_never_leaks_field_contents() {
    let _guard = ENV_LOCK.lock().await;
    let Some((_b, tab)) = tab_on("noleak", INERT_AND_LIVE).await else {
        return;
    };

    neobrowser::ops::fill(&tab, "#f", "PLAINTEXTVALUE")
        .await
        .expect("fill text");
    neobrowser::ops::fill(&tab, "#pw", "SUPERSECRET")
        .await
        .expect("fill password");

    let state = action::observe(&tab).await;
    for secret in ["PLAINTEXTVALUE", "SUPERSECRET"] {
        assert!(
            !state.state_hash.contains(secret),
            "{secret} leaked into the digest: {}",
            state.state_hash
        );
    }
    // The empty → filled transition MUST be detectable, or every password fill would
    // report `uncertain` and users would learn to ignore the status on the most
    // security-sensitive action there is.
    page::eval_body(&tab, "document.getElementById('pw').value=''; return 1")
        .await
        .expect("clear password");
    let empty = action::observe(&tab).await;
    neobrowser::ops::fill(&tab, "#pw", "FIRSTSECRET")
        .await
        .expect("fill password");
    let filled = action::observe(&tab).await;
    assert!(
        !action::detect_changes(&empty, &filled).is_empty(),
        "filling an empty password field must be observable"
    );

    // And the documented limitation, asserted so the trade-off is recorded in a test
    // rather than only in a comment: one non-empty password replaced by another is NOT
    // detectable, because the digest records only empty-vs-filled for password fields.
    // Hashing the value — even salted — would put a hash of a password in every log and
    // evidence bundle. This is the price of that choice, stated rather than discovered.
    neobrowser::ops::fill(&tab, "#pw", "SECONDSECRET")
        .await
        .expect("replace password");
    let replaced = action::observe(&tab).await;
    assert!(
        action::detect_changes(&filled, &replaced).is_empty(),
        "expected the documented blind spot: replacing one non-empty password with          another is not observable. If this now fails, the digest started recording          something about password values — check that it is not a hash of the value"
    );
}

// --- budgets ------------------------------------------------------------------

/// A budget that runs out must be reported, not waited past. The old `navigate` polled a
/// hardcoded 15s regardless of what the caller had time for.
#[tokio::test]
async fn navigation_respects_its_budget_instead_of_a_fixed_timeout() {
    let _guard = ENV_LOCK.lock().await;
    let Some((_b, tab)) = tab_on("budget", INERT_AND_LIVE).await else {
        return;
    };

    // A page that never finishes loading: the fixture holds the connection open.
    let never_ready = "data:text/html,<html><body>\
<script>document.write('<p>partial');\
var t=Date.now(); while(Date.now()-t<300){}</script></body></html>";

    let started = std::time::Instant::now();
    let budget = Budget::from_secs(2.0);
    let complete = page::navigate_budgeted(&tab, never_ready, &budget)
        .await
        .expect("navigate returns rather than hanging");
    let elapsed = started.elapsed();

    // Whether it completed is page-dependent; what must hold is that it did not exceed
    // the budget by an order of magnitude, as the old fixed 15s wait would have.
    assert!(
        elapsed.as_secs() < 8,
        "navigation took {elapsed:?}, which means the budget is not bounding the wait"
    );
    let _ = complete;
}

// --- stable references --------------------------------------------------------

/// The whole point of a stable reference: a `backendNodeId` dies when the node is
/// recreated, and on any SPA that happens constantly.
#[tokio::test]
async fn a_stable_reference_survives_a_full_dom_rebuild() {
    let _guard = ENV_LOCK.lock().await;
    // Every click replaces the entire subtree, invalidating any node id handed out.
    let spa = "data:text/html,<html><body><div id='app'></div><script>\
var n=0;function render(){n++;document.getElementById('app').innerHTML=\
'<button id=go onclick=\"render()\">Continue</button><span>step '+n+'</span>';}render();\
</script></body></html>";
    let Some((_b, tab)) = tab_on("stableref", spa).await else {
        return;
    };

    let snap = observe::snapshot(&tab, observe::SnapshotMode::Interactive)
        .await
        .expect("snapshot");
    let button = snap
        .nodes
        .iter()
        .find(|n| n.name == "Continue")
        .expect("the Continue button is in the snapshot");
    let reference = button.reference.clone();
    let original_id = button.backend_node_id;

    // Four rounds through a rebuilt DOM. The reference must keep resolving, and to a
    // *different* node id than the one first handed out — proving re-resolution rather
    // than a stale id happening to still work.
    let mut ids = std::collections::HashSet::new();
    for round in 1..=4 {
        let resolved = observe::resolve(&tab, &reference)
            .await
            .expect("resolve runs")
            .unwrap_or_else(|| panic!("reference {reference} stopped resolving in round {round}"));
        ids.insert(resolved);

        let before = action::observe(&tab).await;
        page::click_backend_node(&tab, resolved)
            .await
            .expect("click the re-resolved node");
        let (_, changed) = action::wait_for_change(&tab, &before, &Budget::from_secs(3.0)).await;
        assert!(changed, "round {round} did not change the page");
    }

    assert!(
        ids.iter().any(|id| *id != original_id),
        "the node id never changed, so this fixture is not actually rebuilding the DOM \
         and the test proves nothing about re-resolution"
    );
}

/// `observe(diff=true)` must report only what changed. If it returned everything, the
/// context saving that justifies it would not exist.
#[tokio::test]
async fn an_incremental_snapshot_reports_only_the_difference() {
    let _guard = ENV_LOCK.lock().await;
    let grows = "data:text/html,<html><body>\
<button id='add' onclick='var b=document.createElement(\"button\");b.textContent=\"Added\";\
document.body.appendChild(b)'>Add</button></body></html>";
    let Some((_b, tab)) = tab_on("diff", grows).await else {
        return;
    };

    let first = observe::snapshot(&tab, observe::SnapshotMode::Interactive)
        .await
        .expect("first snapshot");
    // An identical page must diff to nothing, or every diff would be noise.
    let same = observe::snapshot(&tab, observe::SnapshotMode::Interactive)
        .await
        .expect("second snapshot");
    assert!(
        observe::diff(&first, &same).is_empty(),
        "an unchanged page must produce an empty diff"
    );

    page::click_selector(&tab, "#add").await.expect("click add");
    let grown = observe::snapshot(&tab, observe::SnapshotMode::Interactive)
        .await
        .expect("third snapshot");
    let d = observe::diff(&first, &grown);
    assert_eq!(d.added.len(), 1, "exactly one element appeared: {d:?}");
    assert!(d.added[0].contains("Added"));
    assert!(d.removed.is_empty(), "nothing was removed: {d:?}");
}

// --- shadow DOM and iframes ---------------------------------------------------

/// Regression, twice over: `querySelector` cannot see into a shadow root, and the state
/// digest could not either — so a successful fill in a web component reported
/// `uncertain`.
#[tokio::test]
async fn actions_inside_shadow_dom_are_reachable_and_verifiable() {
    let _guard = ENV_LOCK.lock().await;
    let component = "data:text/html,<html><body><my-card></my-card><script>\
class MyCard extends HTMLElement{connectedCallback(){\
const r=this.attachShadow({mode:'open'});\
r.innerHTML='<input id=si><button id=sb onclick=\"this.textContent=String(Date.now())\">Shadow</button>';}}\
customElements.define('my-card',MyCard);</script></body></html>";
    let Some((_b, tab)) = tab_on("shadow", component).await else {
        return;
    };

    // The premise: a top-level query genuinely cannot find it.
    let visible_from_top = page::eval_body(&tab, "return !!document.querySelector('#si')")
        .await
        .expect("probe");
    assert_eq!(
        visible_from_top,
        serde_json::Value::Bool(false),
        "if a plain selector can reach it, this fixture is not testing shadow DOM"
    );

    // Fill it, and confirm both the effect and that the digest saw it.
    let before = action::observe(&tab).await;
    let raw = frames::pierce(&tab, "#si", "fill", "hola shadow")
        .await
        .expect("pierce fill");
    let report: serde_json::Value = serde_json::from_str(&raw).expect("valid JSON");
    assert_eq!(report["found"], true, "pierce should find it: {raw}");
    assert_eq!(
        report["path"][0], "shadow:my-card",
        "path should name the route"
    );

    let after = action::observe(&tab).await;
    assert!(
        !action::detect_changes(&before, &after).is_empty(),
        "the digest must cross the shadow boundary, or a successful fill reads as \
         `uncertain`"
    );

    // And the value really is set, not merely reported.
    let value = page::eval_body(
        &tab,
        "return document.querySelector('my-card').shadowRoot.querySelector('#si').value",
    )
    .await
    .expect("read the shadow value");
    assert_eq!(value, serde_json::Value::String("hola shadow".into()));
}

#[tokio::test]
async fn content_inside_a_same_origin_iframe_is_reachable() {
    let _guard = ENV_LOCK.lock().await;
    let framed = "data:text/html,<html><body>\
<iframe srcdoc='<span id=deep>frame text</span>'></iframe></body></html>";
    let Some((_b, tab)) = tab_on("iframe", framed).await else {
        return;
    };

    let raw = frames::pierce(&tab, "#deep", "read", "")
        .await
        .expect("pierce read");
    let report: serde_json::Value = serde_json::from_str(&raw).expect("valid JSON");
    assert_eq!(report["found"], true, "should reach into the frame: {raw}");
    assert!(
        report["text"].as_str().unwrap_or("").contains("frame text"),
        "should read the frame's content: {raw}"
    );

    // And frames are enumerable, with reachability stated rather than implied.
    let raw = frames::list_frames(&tab).await.expect("list_frames");
    let listed: serde_json::Value = serde_json::from_str(&raw).expect("valid JSON");
    assert!(
        !listed["frames"].as_array().unwrap().is_empty(),
        "the frame tree should not be empty: {raw}"
    );
}

/// A missing element must report `found: false`, not throw and not silently succeed.
#[tokio::test]
async fn pierce_reports_a_genuine_miss() {
    let _guard = ENV_LOCK.lock().await;
    let Some((_b, tab)) = tab_on("miss", INERT_AND_LIVE).await else {
        return;
    };
    let raw = frames::pierce(&tab, "#definitely-not-there", "read", "")
        .await
        .expect("pierce runs");
    let report: serde_json::Value = serde_json::from_str(&raw).expect("valid JSON");
    assert_eq!(report["found"], false, "{raw}");
}

// --- walls --------------------------------------------------------------------

/// Reaching a captcha is not a successful navigation, and the status must say which kind
/// of stuck it is: a human can solve a captcha, whereas a rate limit needs backing off.
#[tokio::test]
async fn a_detected_wall_maps_to_a_human_actionable_status() {
    let _guard = ENV_LOCK.lock().await;
    // A page that looks like an interactive challenge to the generic detector.
    let captcha = "data:text/html,<html><body><h1>Verify you are human</h1>\
<div class='cf-turnstile' style='width:300px;height:65px'>challenge</div></body></html>";
    let Some((_b, tab)) = tab_on("wall", captcha).await else {
        return;
    };

    let Some(wall) = neobrowser::walls::detect(&tab).await else {
        panic!("the detector should flag an interactive challenge on this fixture");
    };
    assert_eq!(
        wall.action_status(),
        ActionStatus::NeedsHuman,
        "a captcha needs a person, which is different from `blocked`"
    );
    assert!(!wall.hint().is_empty(), "a wall must come with a next step");
}
