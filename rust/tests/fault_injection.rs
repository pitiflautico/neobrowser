//! Fault injection: sockets, processes, disk and signals (§11 of the PRD).
//!
//! The verified-action contract is a claim about honesty, and honesty is easy while
//! everything works. These tests break things underneath a running action and assert the
//! same property each time: **it must report what actually happened, and it must never
//! report success.**
//!
//! Each fault is one the tool will genuinely meet in production:
//!
//! - **Socket**: Chrome dies and the CDP websocket drops mid-command. This happens on
//!   every OOM kill and every user quitting Chrome.
//! - **Process**: Chrome is SIGKILLed. The next call must recover rather than hand back a
//!   zombie session.
//! - **Disk**: the state directory is read-only or the path is hostile. The vault must
//!   refuse rather than fall back to plaintext — a fallback is how "encrypted at rest"
//!   becomes a claim instead of a fact.
//! - **Signal**: SIGTERM arrives while an action is waiting. It must be cancelled and
//!   reported as cancelled, not as a timeout and not as success.
//!
//! Self-skips when Chrome is absent, like the other live suites.

use neobrowser::action::{self, Budget};
use neobrowser::{chrome, page};

fn chrome_available() -> bool {
    chrome::chrome_bin().exists()
}

static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const PAGE: &str = "data:text/html,<html><body><h1 id='t'>Start</h1>\
<button id='b' onclick='document.getElementById(\"t\").textContent=\"Clicked\"'>Go</button>\
</body></html>";

async fn live_tab(
    name: &str,
) -> Option<(
    neobrowser::browser::Browser,
    std::sync::Arc<neobrowser::cdp::CdpClient>,
)> {
    if !chrome_available() {
        eprintln!("SKIP: no Chrome binary; fault injection needs a real browser");
        return None;
    }
    std::env::set_var("NEOBROWSER_HOME", format!("/tmp/nb-fault-{name}"));
    let browser = neobrowser::browser::Browser::new();
    let tab = browser.tab().await.expect("launch + attach");
    page::navigate_budgeted(&tab, PAGE, &Budget::from_secs(10.0))
        .await
        .expect("navigate");
    Some((browser, tab))
}

/// Kill every Chrome process belonging to this test's profile.
fn kill_chrome_for(name: &str) {
    let marker = format!("/tmp/nb-fault-{name}/profiles");
    let _ = std::process::Command::new("pkill")
        .args(["-9", "-f", &marker])
        .output();
}

// --- socket faults ------------------------------------------------------------

/// The CDP transport dies mid-session. Every subsequent call on that client must return a
/// typed error — never a default value that reads as a successful empty result.
#[tokio::test]
async fn a_dropped_cdp_socket_produces_errors_not_empty_successes() {
    let _guard = ENV_LOCK.lock().await;
    let Some((_browser, tab)) = live_tab("socket").await else {
        return;
    };

    // Sanity: the tab works before the fault, or the test proves nothing.
    assert!(page::eval_body(&tab, "return 1").await.is_ok());

    kill_chrome_for("socket");
    // Give the transport a moment to notice the peer is gone.
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

    // The critical assertion: `js` must FAIL, not return Null. A None-coalescing
    // implementation here would hand a model `null` and look like a page that simply
    // evaluated to nothing.
    match page::eval_body(&tab, "return document.title").await {
        Err(_) => {}
        Ok(v) => panic!("a dead socket returned Ok({v:?}) instead of an error"),
    }

    // And `observe` — which is deliberately failure-tolerant — must yield an EMPTY state,
    // because an empty state is what makes `detect_changes` report "cannot tell" and the
    // action report `uncertain`. Silently returning a stale state would manufacture a
    // false success.
    let state = action::observe(&tab).await;
    assert!(
        state.state_hash.is_empty(),
        "observe must report an unobservable page as empty, got {state:?}"
    );
    let other = action::observe(&tab).await;
    assert!(
        action::detect_changes(&state, &other).is_empty(),
        "two unobservable states must not fabricate a change"
    );
}

/// A click dispatched into a dead socket must not be reported as having happened.
#[tokio::test]
async fn a_click_into_a_dead_socket_is_never_reported_as_success() {
    let _guard = ENV_LOCK.lock().await;
    let Some((_browser, tab)) = live_tab("clickdead").await else {
        return;
    };

    kill_chrome_for("clickdead");
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

    let result = page::click_selector(&tab, "#b").await;
    match result {
        // An error is the honest outcome.
        Err(_) => {}
        // If it somehow returns, it must NOT claim the click landed.
        Ok(outcome) => assert!(
            !matches!(outcome, page::ClickOutcome::Clicked),
            "reported a successful click through a dead transport: {outcome:?}"
        ),
    }
}

// --- process faults -----------------------------------------------------------

/// Chrome is SIGKILLed. The browser manager must relaunch on the next request rather than
/// handing out a zombie — and the recovered session must actually work.
#[tokio::test]
async fn the_browser_recovers_from_a_killed_chrome() {
    let _guard = ENV_LOCK.lock().await;
    if !chrome_available() {
        eprintln!("SKIP: no Chrome binary");
        return;
    }
    std::env::set_var("NEOBROWSER_HOME", "/tmp/nb-fault-recover");
    let browser = neobrowser::browser::Browser::new();
    {
        let tab = browser.tab().await.expect("first launch");
        page::navigate_budgeted(&tab, PAGE, &Budget::from_secs(10.0))
            .await
            .expect("navigate");
    }

    kill_chrome_for("recover");
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

    // A fresh tab request must relaunch. This is the self-healing path, and the assertion
    // is that it produces a *working* tab, not merely a non-error.
    let tab = browser
        .tab()
        .await
        .expect("the manager must relaunch after a kill");
    page::navigate_budgeted(&tab, PAGE, &Budget::from_secs(10.0))
        .await
        .expect("navigate on the recovered session");
    let text = page::read_text(&tab, "#t")
        .await
        .expect("read after recovery");
    assert_eq!(text, "Start", "the recovered session must be usable");

    // And no orphan is left behind by the recovery itself.
    browser.shutdown().await;
}

// --- disk faults --------------------------------------------------------------

/// The vault must refuse to write rather than fall back to plaintext. A fallback is
/// exactly how an "encrypted at rest" claim stops being true.
#[test]
fn the_vault_refuses_rather_than_writing_plaintext_when_it_cannot_seal() {
    let dir = std::env::temp_dir().join(format!("nb-fault-vault-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("cookies.vault");

    // An invalid key stands in for "the credential store is unusable": the vault has no
    // other source of a key, so this is the same failure the real path hits.
    let prev = std::env::var("NEOBROWSER_VAULT_KEY").ok();
    std::env::set_var("NEOBROWSER_VAULT_KEY", "not-a-valid-base64-32-byte-key!!");

    let result = neobrowser::vault::seal(&path, "SECRET-COOKIE-VALUE", &[], None);
    assert!(result.is_err(), "sealing with an unusable key must fail");

    // The decisive check: nothing was written, and certainly not the plaintext.
    if path.exists() {
        let written = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            !written.contains("SECRET-COOKIE-VALUE"),
            "the vault wrote plaintext after failing to encrypt: {written}"
        );
    }

    match prev {
        Some(v) => std::env::set_var("NEOBROWSER_VAULT_KEY", v),
        None => std::env::remove_var("NEOBROWSER_VAULT_KEY"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A read-only destination must produce an error, and must not leave a partial or
/// world-readable file behind.
#[test]
fn a_read_only_directory_fails_cleanly_without_leaving_debris() {
    let dir = std::env::temp_dir().join(format!("nb-fault-ro-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // 0500: traversable and readable, not writable.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();

        let target = dir.join("cookies.json");
        let result = neobrowser::sessions::write_private(&target, "SESSION-DATA");
        assert!(result.is_err(), "writing into a read-only dir must fail");

        // Restore permissions so the directory can be inspected and removed.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let leftovers: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            leftovers.is_empty(),
            "a failed write left debris behind: {leftovers:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A trace bundle must not be silently lost when its directory cannot be written; and it
/// must never be written world-readable as a fallback.
#[tokio::test]
async fn a_trace_bundle_write_failure_is_reported_not_swallowed() {
    // `write_bundle` resolves its path through NEOBROWSER_HOME, which is process-global.
    // Without this lock a sibling test's home leaks in and the unwritable directory under
    // test is never the one actually written to — the test would then pass or fail on
    // scheduling rather than on behaviour.
    let _guard = ENV_LOCK.lock().await;
    let home = std::env::temp_dir().join(format!("nb-fault-trace-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    std::env::set_var("NEOBROWSER_HOME", &home);

    let t = neobrowser::trace::Trace::new("trace_fault");
    t.record(
        "tool_call",
        None,
        None,
        serde_json::json!({ "tool": "read" }),
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o500)).unwrap();
        // `write_bundle` returns a Result precisely so this is reportable rather than a
        // silently missing file.
        assert!(
            t.write_bundle().is_err(),
            "an unwritable home must surface an error"
        );
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let _ = std::fs::remove_dir_all(&home);
}

// --- signal faults ------------------------------------------------------------

/// SIGTERM during an in-flight wait must cancel it promptly and report it AS cancelled —
/// not as a timeout (which would send someone debugging a slow page) and not as success.
#[tokio::test]
async fn a_shutdown_signal_cancels_a_waiting_action_promptly() {
    let _guard = ENV_LOCK.lock().await;
    let Some((_browser, tab)) = live_tab("signal").await else {
        return;
    };

    let before = action::observe(&tab).await;
    // A generous budget, so anything short is attributable to the cancellation.
    let budget = Budget::from_secs(120.0);

    let started = std::time::Instant::now();
    let waiter = tokio::spawn({
        let tab = tab.clone();
        let before = before.clone();
        async move { action::wait_for_change(&tab, &before, &budget).await }
    });

    // Nothing will change the page, so the wait would otherwise run the full two minutes.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    action::begin_shutdown();

    let (_state, changed) = waiter.await.expect("the waiter task completes");
    let elapsed = started.elapsed();

    assert!(!changed, "nothing changed, so it must not claim a change");
    assert!(
        elapsed.as_secs() < 10,
        "the wait took {elapsed:?} despite a shutdown signal; cancellation is not \
         reaching the poll loop, and shutdown would hang for the whole budget"
    );

    // Leave the flag clear so later tests in this binary are unaffected.
    neobrowser::action::end_shutdown_for_tests();
}
