//! JS-level anti-detection patch, installed on every new document of a tab we own
//! via `Page.addScriptToEvaluateOnNewDocument`.
//!
//! Ported verbatim from the Python `_STEALTH_JS`. The guiding principle is *real >
//! fake*: we only fill in values that `--headless=new` leaves empty or contradictory
//! (webdriver, window.chrome shape, connection, hasFocus, empty languages/plugins).
//! We deliberately do NOT spoof WebGL vendor/renderer, hardwareConcurrency, or
//! deviceMemory — under headless with a real GPU those are genuine, and faking them
//! would create the very mismatch modern anti-bot systems look for.
//!
//! The UA `HeadlessChrome` tell is handled at launch via the `--user-agent` flag
//! (see `chrome::chrome_user_agent`), not here, so genuine Client Hints stay intact.

/// Loaded from `js/stealth.js`. See [`crate::js`] for why the snippets live
/// in real JavaScript files rather than Rust string literals.
pub fn stealth_js() -> &'static str {
    include_str!("../js/stealth.js")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stealth_js_covers_the_key_tells() {
        // Regression guard: don't let a refactor silently drop a patch.
        for needle in [
            "webdriver",
            "window.chrome",
            "navigator.connection",
            "document.hasFocus",
            "navigator.languages",
            "navigator.plugins",
            "notifications",
        ] {
            assert!(
                stealth_js().contains(needle),
                "stealth JS missing: {needle}"
            );
        }
    }

    #[test]
    fn stealth_js_does_not_spoof_webgl_or_hardware() {
        // These must stay genuine under headless with a real GPU. Check for actual
        // spoofing patterns, not the words themselves (they appear in the comment
        // that explains why we don't touch them).
        assert!(
            !stealth_js().contains("getParameter"),
            "WebGL getParameter override present"
        );
        assert!(
            !stealth_js().contains("'hardwareConcurrency'"),
            "hardwareConcurrency is being defined"
        );
        assert!(
            !stealth_js().contains("'deviceMemory'"),
            "deviceMemory is being defined"
        );
    }
}
