//! Real stealth verification: launch actual Chrome, apply the stealth patch, and
//! assert every headless tell is clean. This is the gap the Python suite never had
//! (it tested the JS blob's *content*, never that it actually neutralizes the tells
//! in a live browser).
//!
//! The hermetic test uses a `data:` URL so no network is needed and runs in CI when
//! Chrome is present (it self-skips otherwise). The `#[ignore]` test hits the real
//! bot.sannysoft detector and is run on demand with `cargo test -- --ignored`.

use std::time::Duration;

use neobrowser::browser::Browser;
use neobrowser::{chrome, page};

/// True if a Chrome binary is actually available to launch.
fn chrome_available() -> bool {
    chrome::chrome_bin().exists()
}

/// Give this test binary an isolated NeoBrowser home so it never touches real profiles.
fn isolate_home() {
    std::env::set_var("NEOBROWSER_HOME", "/tmp/nb-rust-stealth-it");
}

async fn eval_bool(client: &neobrowser::cdp::CdpClient, expr: &str) -> bool {
    match page::js(client, expr).await {
        Ok(v) => v.as_bool().unwrap_or(false),
        Err(_) => false,
    }
}

#[tokio::test]
async fn stealth_neutralizes_headless_tells_on_a_live_page() {
    if !chrome_available() {
        eprintln!("SKIP: no Chrome binary found; stealth verification needs a real browser");
        return;
    }
    isolate_home();

    let browser = Browser::new();
    let tab = browser.tab().await.expect("launch + attach a CDP tab");

    // Navigate to a fresh document so addScriptToEvaluateOnNewDocument fires.
    page::navigate(
        &tab,
        "data:text/html,<html><body>stealth</body></html>",
        1.0,
    )
    .await
    .expect("navigate to data url");

    // Each assertion is a tell an anti-bot system checks.
    assert!(
        eval_bool(&tab, "return navigator.webdriver === undefined").await,
        "navigator.webdriver leaked"
    );
    assert!(
        eval_bool(&tab, "return !!(window.chrome && window.chrome.runtime)").await,
        "window.chrome.runtime missing (partial headless chrome object)"
    );
    assert!(
        eval_bool(&tab, "return !!navigator.connection").await,
        "navigator.connection missing (headless omits it)"
    );
    assert!(
        eval_bool(&tab, "return document.hasFocus() === true").await,
        "document.hasFocus() is false (inactive/automated tab tell)"
    );
    assert!(
        eval_bool(
            &tab,
            "return navigator.languages && navigator.languages.length > 0"
        )
        .await,
        "navigator.languages empty"
    );
    assert!(
        eval_bool(
            &tab,
            "return navigator.plugins && navigator.plugins.length > 0"
        )
        .await,
        "navigator.plugins empty"
    );

    // Sanity: the window.chrome enums the real object exposes, which CF inspects.
    assert!(
        eval_bool(
            &tab,
            "return typeof window.chrome.runtime.PlatformOs === 'object'"
        )
        .await,
        "window.chrome.runtime enums missing"
    );

    browser.shutdown().await;
}

/// On-demand check against the real bot.sannysoft detector. Requires network.
/// Run with: `cargo test --test stealth_verify -- --ignored`
#[tokio::test]
#[ignore]
async fn passes_bot_sannysoft_webdriver_check() {
    if !chrome_available() {
        eprintln!("SKIP: no Chrome binary found");
        return;
    }
    isolate_home();
    let browser = Browser::new();
    let tab = browser.tab().await.expect("tab");
    page::navigate(&tab, "https://bot.sannysoft.com/", 3.0)
        .await
        .expect("navigate");
    // Let the detector's async probes settle, forcing frames so results render.
    tokio::time::sleep(Duration::from_secs(2)).await;
    page::nudge_frame(&tab).await;
    let text = page::read_text(&tab, "body")
        .await
        .unwrap_or_default()
        .to_lowercase();
    let head: String = text.chars().take(500).collect();

    // Strict: the WebDriver row must read the definitive "missing (passed)" (i.e.
    // navigator.webdriver is undefined) — not just any "passed" anywhere on the page.
    assert!(
        text.contains("missing (passed)"),
        "WebDriver check is not 'missing (passed)'; head: {head}"
    );
    // And no core fingerprint check may be reported as failed.
    for bad in [
        "present (failed)",
        "webdriver (new) failed",
        "headlesschrome",
    ] {
        assert!(
            !text.contains(bad),
            "stealth regression — page contains '{bad}'; head: {head}"
        );
    }
    browser.shutdown().await;
}
