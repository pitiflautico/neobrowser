//! Regression tests for the multi-step-form bugs (see
//! `docs/BUGS-formularios-multipaso.md`).
//!
//! All four came from the same failure mode: a tool reporting success because it
//! dispatched an action, without checking that the action had any effect. These
//! assert the *effect*, never the return value alone.
//!
//! Hermetic: a `data:` URL fixture, no network. Self-skips when Chrome is absent.

use neobrowser::browser::Browser;
use neobrowser::page::{self, ClickOutcome};
use neobrowser::{chrome, ops};

fn chrome_available() -> bool {
    chrome::chrome_bin().exists()
}

/// `NEOBROWSER_HOME` is process-global, so tests that set it must not overlap.
/// Each test takes this lock for its whole body and gets its own home, so no two
/// Chromes ever contend for the same profile (which would trip the very
/// SingletonLock rule under test).
static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn isolate_home(name: &str) {
    std::env::set_var("NEOBROWSER_HOME", format!("/tmp/nb-rust-it-{name}"));
}

/// Fixture reproducing the page-level traps, all at once:
///
/// - a collapsed step-1 form whose "Continue" button is invisible but present,
/// - an open step-2 form with its own "Continue",
/// - a checkbox far below the fold,
/// - a checkbox covered by a fixed overlay.
///
/// The banner deliberately covers ONLY the top strip, so each trap stays
/// independent: `#covered` sits under it, the Continue buttons and `#below` do
/// not. A full-screen veil would contaminate every other assertion.
const FIXTURE: &str = "data:text/html,\
<html><body style='margin:0'>\
<div id='veil' class='cookie-banner' \
style='position:fixed;top:0;left:0;right:0;height:60px;background:rgb(51,51,51);z-index:99'></div>\
<div style='height:12px'></div>\
<input type='checkbox' id='covered'>\
<div style='height:240px'></div>\
<form id='step1' style='height:0;overflow:hidden'>\
<button type='button' id='c1' onclick='window.clicked=\"step1\"'>Continue</button>\
</form>\
<form id='step2'>\
<button type='button' id='c2' onclick='window.clicked=\"step2\"'>Continue</button>\
</form>\
<div style='height:2500px'></div>\
<input type='checkbox' id='below'>\
<div style='height:600px'></div>\
</body></html>";

/// Launch an isolated browser on the fixture. The returned `Browser` must be
/// kept alive for the duration of the test (dropping it reaps Chrome).
async fn fixture_tab(name: &str) -> Option<(Browser, std::sync::Arc<neobrowser::cdp::CdpClient>)> {
    if !chrome_available() {
        eprintln!("SKIP: no Chrome binary found; these tests need a real browser");
        return None;
    }
    isolate_home(name);
    let browser = Browser::new();
    let tab = browser.tab().await.expect("launch + attach a CDP tab");
    page::navigate(&tab, FIXTURE, 1.0)
        .await
        .expect("navigate to fixture");
    Some((browser, tab))
}

/// Bug 1: `find_and_click` used to take the first text match in the DOM, even
/// inside a collapsed container — so every "Continue" hit the closed step 1 and
/// step 2 was never submitted.
#[tokio::test]
async fn find_and_click_skips_collapsed_matches() {
    let _guard = ENV_LOCK.lock().await;
    let Some((_browser, tab)) = fixture_tab("collapsed").await else {
        return;
    };

    let raw = ops::find_and_click(&tab, "Continue", "", 0)
        .await
        .expect("find_and_click runs");
    let report: serde_json::Value = serde_json::from_str(&raw).expect("valid JSON report");

    assert_eq!(report["ok"], true, "should have clicked something: {raw}");
    // Both buttons match by text; only one is visible.
    assert_eq!(report["matched_total"], 2, "both matches counted: {raw}");
    assert_eq!(report["matched_visible"], 1, "one visible match: {raw}");

    // The effect is what matters: step 2 got the click, not the collapsed step 1.
    let clicked = page::eval_body(&tab, "return window.clicked || ''")
        .await
        .expect("read window.clicked");
    assert_eq!(
        clicked.as_str().unwrap_or(""),
        "step2",
        "click landed on the collapsed step-1 button"
    );
}

/// Bug 1b: when every text match is hidden, that must be an explicit failure —
/// not a silent `ok: true` on an unclickable node.
#[tokio::test]
async fn find_and_click_reports_when_all_matches_are_hidden() {
    let _guard = ENV_LOCK.lock().await;
    let Some((_browser, tab)) = fixture_tab("hidden").await else {
        return;
    };

    // Collapse step 2 as well: now both "Continue" buttons are invisible.
    page::eval_body(
        &tab,
        "return (document.getElementById('step2').style.cssText = 'height:0;overflow:hidden')",
    )
    .await
    .expect("collapse step2");

    let raw = ops::find_and_click(&tab, "Continue", "", 0)
        .await
        .expect("find_and_click runs");
    let report: serde_json::Value = serde_json::from_str(&raw).expect("valid JSON report");

    assert_eq!(report["ok"], false, "hidden-only match must fail: {raw}");
    assert_eq!(report["matched_visible"], 0, "{raw}");
    let err = report["error"].as_str().unwrap_or("");
    assert!(
        err.contains("hidden") || err.contains("collapsed"),
        "error should explain why: {err}"
    );
}

/// Bug 2a: a target below the fold used to report "Clicked" while the mouse
/// event landed off-screen and the checkbox stayed unchecked.
#[tokio::test]
async fn click_scrolls_target_into_view() {
    let _guard = ENV_LOCK.lock().await;
    let Some((_browser, tab)) = fixture_tab("scroll").await else {
        return;
    };

    let outcome = page::click_selector(&tab, "#below")
        .await
        .expect("click runs");
    assert_eq!(outcome, ClickOutcome::Clicked, "expected a landed click");

    let checked = page::eval_body(&tab, "return document.getElementById('below').checked")
        .await
        .expect("read checked");
    assert_eq!(
        checked.as_bool(),
        Some(true),
        "below-the-fold checkbox was not actually toggled"
    );
}

/// Bug 2b: a target under an overlay used to report "Clicked" while the overlay
/// swallowed the event. It must now be reported as obscured, and NOT dispatched.
#[tokio::test]
async fn click_detects_an_overlay_instead_of_claiming_success() {
    let _guard = ENV_LOCK.lock().await;
    let Some((_browser, tab)) = fixture_tab("overlay").await else {
        return;
    };

    let outcome = page::click_selector(&tab, "#covered")
        .await
        .expect("click runs");

    match &outcome {
        ClickOutcome::Obscured { by } => {
            assert!(
                by.contains("div") || by.contains("cookie"),
                "obscuring element should be described, got {by:?}"
            );
        }
        other => panic!("expected Obscured, got {other:?}"),
    }

    // And the click must not have gone through to the covered control.
    let checked = page::eval_body(&tab, "return document.getElementById('covered').checked")
        .await
        .expect("read checked");
    assert_eq!(
        checked.as_bool(),
        Some(false),
        "click was dispatched despite the overlay"
    );

    // Remove the overlay and the very same call should now succeed.
    page::eval_body(&tab, "return document.getElementById('veil').remove()")
        .await
        .expect("remove overlay");
    let outcome = page::click_selector(&tab, "#covered")
        .await
        .expect("click runs");
    assert_eq!(outcome, ClickOutcome::Clicked, "clickable once uncovered");
    let checked = page::eval_body(&tab, "return document.getElementById('covered').checked")
        .await
        .expect("read checked");
    assert_eq!(checked.as_bool(), Some(true), "should be checked now");
}

/// Bug 2c: a selector matching nothing must say so, not be lumped in with
/// "no layout".
#[tokio::test]
async fn click_distinguishes_a_missing_target() {
    let _guard = ENV_LOCK.lock().await;
    let Some((_browser, tab)) = fixture_tab("missing").await else {
        return;
    };
    let outcome = page::click_selector(&tab, "#does-not-exist")
        .await
        .expect("click runs");
    assert_eq!(outcome, ClickOutcome::NotFound);
}

/// Bug 3: an orphaned `SingletonLock` (pid long gone) used to make every launch
/// fail with an opaque timeout. It must be cleared automatically.
#[tokio::test]
async fn stale_singleton_lock_does_not_block_launch() {
    if !chrome_available() {
        eprintln!("SKIP: no Chrome binary found");
        return;
    }
    let _guard = ENV_LOCK.lock().await;
    let home = "/tmp/nb-rust-it-stalelock";
    std::env::set_var("NEOBROWSER_HOME", home);
    let profile = std::path::Path::new(home).join("profiles").join("default");
    std::fs::create_dir_all(&profile).expect("create profile dir");

    // A pid that cannot plausibly be running.
    let lock = profile.join("SingletonLock");
    let _ = std::fs::remove_file(&lock);
    #[cfg(unix)]
    std::os::unix::fs::symlink("test-host-999999", &lock).expect("plant stale lock");

    let mut proc = chrome::ChromeProcess::launch(&profile)
        .await
        .expect("launch with a stale lock present");
    let ready = chrome::wait_for_chrome(proc.port, std::time::Duration::from_secs(20)).await;
    proc.kill(true).await;

    assert!(
        ready.is_ok(),
        "stale lock should have been cleared: {:?}",
        ready.err()
    );
}

/// Bug 3b: a lock owned by a LIVE process must be left alone — clearing it would
/// yank the profile out from under a running sibling.
#[test]
fn live_singleton_lock_is_left_alone() {
    let home = "/tmp/nb-rust-livelock-it";
    let profile = std::path::Path::new(home).join("profiles").join("live");
    std::fs::create_dir_all(&profile).expect("create profile dir");
    let lock = profile.join("SingletonLock");
    let _ = std::fs::remove_file(&lock);

    // Our own pid is definitely alive.
    let me = std::process::id();
    #[cfg(unix)]
    std::os::unix::fs::symlink(format!("test-host-{me}"), &lock).expect("plant live lock");

    chrome::clear_stale_lock_for_test(&profile);

    // symlink_metadata, not exists(): the link target is a fake path, so
    // exists() would follow it and report false even with the link in place.
    assert!(
        std::fs::symlink_metadata(&lock).is_ok(),
        "a lock held by a live process must not be removed"
    );
    let _ = std::fs::remove_file(&lock);
}

// ---------------------------------------------------------------------------
// Concurrent-profile isolation (NEOBROWSER_PROFILE)
//
// Chrome takes an exclusive lock on a user-data dir, so two NeoBrowser sessions
// sharing one profile cannot both run — the second dies with an opaque timeout.
// These assert the real behaviour with real Chrome processes, not mocks.
// ---------------------------------------------------------------------------

/// A named profile must get its OWN user-data dir, so two sessions can run at
/// the same time instead of fighting over `profiles/default`.
#[tokio::test]
async fn named_profiles_get_separate_user_data_dirs() {
    if !chrome_available() {
        eprintln!("SKIP: no Chrome binary found");
        return;
    }
    let _guard = ENV_LOCK.lock().await;
    let home = "/tmp/nb-rust-it-profiles";
    std::env::set_var("NEOBROWSER_HOME", home);

    std::env::set_var("NEOBROWSER_PROFILE", "alpha");
    let alpha = neobrowser::paths::profile_dir();
    std::env::set_var("NEOBROWSER_PROFILE", "beta");
    let beta = neobrowser::paths::profile_dir();
    std::env::remove_var("NEOBROWSER_PROFILE");
    let fallback = neobrowser::paths::profile_dir();

    assert_ne!(alpha, beta, "distinct profiles must not share a directory");
    assert!(alpha.ends_with("alpha"), "got {alpha:?}");
    assert!(beta.ends_with("beta"), "got {beta:?}");
    assert!(fallback.ends_with("default"), "unset must fall back");

    // The real point: two Chromes, both alive, at the same time.
    let mut a = chrome::ChromeProcess::launch(&alpha)
        .await
        .expect("launch alpha");
    let mut b = chrome::ChromeProcess::launch(&beta)
        .await
        .expect("launch beta");
    let ra = chrome::wait_for_chrome(a.port, std::time::Duration::from_secs(20)).await;
    let rb = chrome::wait_for_chrome(b.port, std::time::Duration::from_secs(20)).await;
    a.kill(true).await;
    b.kill(true).await;

    assert!(ra.is_ok(), "alpha never came up: {:?}", ra.err());
    assert!(rb.is_ok(), "beta never came up: {:?}", rb.err());
}

/// A profile already held by a LIVE Chrome must fail with an actionable error
/// naming the profile, the pid and the port to attach to — not the opaque
/// "did not become ready" timeout that sent us debugging by hand.
#[tokio::test]
async fn profile_held_by_a_live_chrome_fails_with_a_way_out() {
    if !chrome_available() {
        eprintln!("SKIP: no Chrome binary found");
        return;
    }
    let _guard = ENV_LOCK.lock().await;
    std::env::set_var("NEOBROWSER_HOME", "/tmp/nb-rust-it-inuse");
    std::env::set_var("NEOBROWSER_PROFILE", "busy");
    let dir = neobrowser::paths::profile_dir();

    let mut holder = chrome::ChromeProcess::launch(&dir)
        .await
        .expect("first launch");
    chrome::wait_for_chrome(holder.port, std::time::Duration::from_secs(20))
        .await
        .expect("holder ready");

    // Second launch on the same profile, while the first is alive.
    let second = chrome::ChromeProcess::launch(&dir).await;
    let err = match second {
        Err(e) => e.to_string(),
        Ok(mut p) => {
            p.kill(true).await;
            holder.kill(true).await;
            panic!("second launch should have been refused while the profile is in use");
        }
    };
    holder.kill(true).await;
    std::env::remove_var("NEOBROWSER_PROFILE");

    assert!(err.contains("busy"), "error must name the profile: {err}");
    assert!(
        err.contains("NEOBROWSER_PROFILE"),
        "error must offer a way out: {err}"
    );
    assert!(
        err.contains("NEOBROWSER_ATTACH_PORT"),
        "error must offer attaching: {err}"
    );
}

/// Regression: the collapsed-ancestor filter used to walk all the way up to
/// <html>. Sites with fixed or virtualised scrolling (Lenis, body{position:fixed})
/// give <html> a zero height with overflow:hidden, so every clickable on the page
/// was reported as "hidden or inside a collapsed container" and nothing could be
/// clicked. Found dogfooding on a real site (cloudstudio.es), not in a fixture.
#[tokio::test]
async fn zero_height_html_does_not_hide_the_whole_page() {
    if !chrome_available() {
        eprintln!("SKIP: no Chrome binary found");
        return;
    }
    let _guard = ENV_LOCK.lock().await;
    isolate_home("zeroheight");
    let browser = Browser::new();
    let tab = browser.tab().await.expect("launch + attach a CDP tab");

    // html{height:0;overflow:hidden} with a fixed body — the real-world shape.
    let fixture = "data:text/html,\
<html style='height:0;overflow:hidden'>\
<body style='position:fixed;inset:0;margin:0;overflow:hidden'>\
<button type='button' id='go' onclick='window.hit=1'>Explore</button>\
</body></html>";
    page::navigate(&tab, fixture, 1.0).await.expect("navigate");

    let html_h = page::eval_body(
        &tab,
        "return document.documentElement.getBoundingClientRect().height",
    )
    .await
    .expect("read html height");
    assert_eq!(
        html_h.as_f64(),
        Some(0.0),
        "fixture must actually reproduce the zero-height html"
    );

    let raw = ops::find_and_click(&tab, "Explore", "", 0)
        .await
        .expect("find_and_click runs");
    let report: serde_json::Value = serde_json::from_str(&raw).expect("valid JSON");
    assert_eq!(
        report["ok"], true,
        "visible button must be clickable: {raw}"
    );
    assert_eq!(report["matched_visible"], 1, "{raw}");

    let hit = page::eval_body(&tab, "return window.hit || 0")
        .await
        .expect("read hit");
    assert_eq!(hit.as_i64(), Some(1), "the click never reached the button");
}

// ---------------------------------------------------------------------------
// Wall detection: only a VISIBLE challenge counts
// ---------------------------------------------------------------------------

/// Regression: any page loading Stripe.js was flagged as having a captcha,
/// because Stripe embeds an *invisible* anti-fraud hCaptcha iframe
/// (js.stripe.com/v3/hcaptcha-invisible-…, visibility:hidden, 1px tall).
/// navigate then told the agent "a real profile or human handoff may be needed"
/// on an ordinary checkout page with nothing in its way — advice that makes an
/// obedient agent abandon a flow it could complete. Found on thefwa.com.
#[tokio::test]
async fn invisible_captcha_is_not_reported_as_a_wall() {
    if !chrome_available() {
        eprintln!("SKIP: no Chrome binary found");
        return;
    }
    let _guard = ENV_LOCK.lock().await;
    isolate_home("wallcaptcha");
    let browser = Browser::new();
    let tab = browser.tab().await.expect("launch + attach a CDP tab");

    // Shaped exactly like Stripe's: matches the hcaptcha selector, invisible.
    let invisible = "data:text/html,\
<html><body><h1>Checkout</h1>\
<iframe src='https://example.com/v3/hcaptcha-invisible-abc' \
style='visibility:hidden;width:1905px;height:1px;border:0'></iframe>\
</body></html>";
    page::navigate(&tab, invisible, 1.0)
        .await
        .expect("navigate");
    let hint = neobrowser::walls::detect(&tab).await;
    assert!(
        hint.is_none(),
        "an invisible captcha must not be reported as a wall, got {hint:?}"
    );

    // A real, visible widget still must be reported.
    let visible = "data:text/html,\
<html><body><h1>Verify</h1>\
<iframe src='https://example.com/recaptcha/api2/anchor' \
style='width:304px;height:78px;border:0'></iframe>\
</body></html>";
    page::navigate(&tab, visible, 1.0).await.expect("navigate");
    let hint = neobrowser::walls::detect(&tab).await;
    assert!(
        hint.is_some(),
        "a visible captcha widget must still be reported"
    );
}

/// A submit control whose label lives in `value` must be findable by that label.
///
/// `<input type="submit" value="Login">` has no `textContent` at all — the word the user
/// reads is in the `value` attribute. `find_and_click` matched only `textContent` and
/// `aria-label`, so it could not find the submit button on a large fraction of the real web.
///
/// This was found by the real-site battery (`scripts/real-sites.py`), not by any test here:
/// the login form of saucedemo.com filled correctly, the click reported `uncertain`, and the
/// page never advanced. The accessibility tree had it right the whole time — `observe`
/// reported `button "Login"` — so the two halves of the tool disagreed about the same
/// element, which is exactly the kind of thing only a real page surfaces.
///
/// The regression guard covers every place a visible label can hide, because fixing one and
/// leaving the others is how this recurs.
#[tokio::test]
async fn a_submit_control_is_found_by_the_label_a_user_reads() {
    let _guard = ENV_LOCK.lock().await;
    if !chrome_available() {
        eprintln!("SKIP: no Chrome binary");
        return;
    }
    isolate_home("labels");
    let browser = Browser::new();
    let tab = browser.tab().await.expect("launch");

    // Each control carries its label in a different attribute, and none in textContent.
    const PAGE: &str = "data:text/html,<html><body>\
<input type='submit' id='a' value='Send order'>\
<input type='button' id='b' value='Cancel'>\
<button id='c' title='Print receipt'></button>\
<input type='image' id='d' alt='Search now' src='data:image/gif;base64,R0lGODlhAQABAAAAACw='>\
</body></html>";
    page::navigate_budgeted(&tab, PAGE, &neobrowser::action::Budget::from_secs(10.0))
        .await
        .expect("navigate");

    for (label, want_id) in [
        ("send order", "a"),
        ("cancel", "b"),
        ("print receipt", "c"),
        ("search now", "d"),
    ] {
        let report = ops::find_and_click(&tab, label, "", 0)
            .await
            .unwrap_or_else(|e| panic!("find_and_click({label}) errored: {e}"));
        // Assert on the reported outcome, not on the text appearing somewhere in the report.
        // The first version of this test checked `report.contains(label)`, which is also true
        // of the failure message ("no match for: send order") — so it passed with the bug
        // present. A test that cannot fail is worse than no test, because it certifies the
        // thing it never checked.
        let parsed: serde_json::Value = serde_json::from_str(&report)
            .unwrap_or_else(|e| panic!("find_and_click({label}) must return JSON ({e}): {report}"));
        assert_eq!(
            parsed["ok"],
            serde_json::json!(true),
            "`{label}` must be findable — it is the only text a user sees on #{want_id}, and a \
             matcher that reads textContent alone finds none of these four: {report}"
        );
        assert!(
            parsed["matched_visible"].as_i64().unwrap_or(0) >= 1,
            "`{label}` must match a visible control: {report}"
        );
    }
    browser.shutdown().await;
}
