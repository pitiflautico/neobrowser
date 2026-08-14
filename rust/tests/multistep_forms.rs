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
    let clicked = page::js(&tab, "return window.clicked || ''")
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
    page::js(
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

    let checked = page::js(&tab, "return document.getElementById('below').checked")
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
    let checked = page::js(&tab, "return document.getElementById('covered').checked")
        .await
        .expect("read checked");
    assert_eq!(
        checked.as_bool(),
        Some(false),
        "click was dispatched despite the overlay"
    );

    // Remove the overlay and the very same call should now succeed.
    page::js(&tab, "return document.getElementById('veil').remove()")
        .await
        .expect("remove overlay");
    let outcome = page::click_selector(&tab, "#covered")
        .await
        .expect("click runs");
    assert_eq!(outcome, ClickOutcome::Clicked, "clickable once uncovered");
    let checked = page::js(&tab, "return document.getElementById('covered').checked")
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
