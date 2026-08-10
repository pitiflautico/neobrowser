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

pub const STEALTH_JS: &str = r#"
(() => {
  const def = (obj, prop, val) => {
    try { Object.defineProperty(obj, prop, { get: () => val, configurable: true }); } catch (e) {}
  };
  // navigator.webdriver: force undefined even if a Chrome build still exposes it.
  try { Object.defineProperty(Navigator.prototype, 'webdriver', { get: () => undefined, configurable: true }); } catch (e) {}

  // window.chrome — headless leaves this partial; Cloudflare inspects chrome.app,
  // chrome.runtime enums, loadTimes and csi. Provide a genuine-shaped object.
  try {
    if (!window.chrome || !window.chrome.runtime) {
      window.chrome = {
        app: { isInstalled: false,
               InstallState: { DISABLED: 'disabled', INSTALLED: 'installed', NOT_INSTALLED: 'not_installed' },
               RunningState: { CANNOT_RUN: 'cannot_run', READY_TO_RUN: 'ready_to_run', RUNNING: 'running' },
               getDetails: () => null, getIsInstalled: () => false, installState: () => {} },
        runtime: { id: undefined, connect: () => {}, sendMessage: () => {},
                   PlatformOs: { MAC: 'mac', WIN: 'win', ANDROID: 'android', CROS: 'cros', LINUX: 'linux' },
                   PlatformArch: { ARM: 'arm', X86_32: 'x86-32', X86_64: 'x86-64' },
                   OnInstalledReason: { INSTALL: 'install', UPDATE: 'update', CHROME_UPDATE: 'chrome_update' } },
        loadTimes: function () { const t = performance.timing;
          return { requestTime: t.navigationStart / 1000, startLoadTime: t.navigationStart / 1000,
                   finishDocumentLoadTime: t.domContentLoadedEventEnd / 1000, finishLoadTime: t.loadEventEnd / 1000,
                   firstPaintTime: 0, firstPaintAfterLoadTime: 0, navigationType: 'Other',
                   wasFetchedViaSpdy: false, wasNpnNegotiated: false, npnNegotiatedProtocol: 'unknown',
                   wasAlternateProtocolAvailable: false, connectionInfo: 'unknown' }; },
        csi: function () { const t = performance.timing;
          return { startE: t.navigationStart, onloadT: t.loadEventEnd, pageT: Date.now() - t.navigationStart, tran: 15 }; },
      };
    }
  } catch (e) {}

  // Network Information API — headless omits it; Cloudflare checks navigator.connection.
  try { if (!navigator.connection) def(navigator, 'connection', { effectiveType: '4g', rtt: 50, downlink: 10, saveData: false, onchange: null }); } catch (e) {}
  // Cloudflare uses document.hasFocus() to detect hidden/inactive automated tabs.
  try { document.hasFocus = () => true; } catch (e) {}
  // Headless can report an atypical screen size; align it with a common desktop.
  try {
    if (screen.width < 1000) {
      def(screen, 'width', 1920); def(screen, 'height', 1080);
      def(screen, 'availWidth', 1920); def(screen, 'availHeight', 1055);
    }
  } catch (e) {}

  // Fill languages / plugins ONLY when headless actually left them empty — never
  // overwrite genuine values (a mismatch is exactly what anti-bot looks for).
  try { if (!navigator.languages || navigator.languages.length === 0) def(navigator, 'languages', ['en-US', 'en']); } catch (e) {}
  try {
    if (navigator.plugins && navigator.plugins.length === 0) {
      def(navigator, 'plugins', [{ name: 'PDF Viewer' }, { name: 'Chrome PDF Viewer' }, { name: 'Chromium PDF Viewer' }]);
    }
  } catch (e) {}
  // Notification permission consistency (headless contradicts Notification.permission).
  try {
    const perms = window.navigator.permissions;
    if (perms && perms.query) {
      const original = perms.query.bind(perms);
      perms.query = (p) => (p && p.name === 'notifications')
        ? Promise.resolve({ state: Notification.permission }) : original(p);
    }
  } catch (e) {}
  // NOTE: we deliberately do NOT spoof WebGL vendor/renderer or hardwareConcurrency/
  // deviceMemory. Under --headless=new with a real GPU those values are genuine;
  // faking them (e.g. to a different GPU than the host) would CREATE the mismatch
  // modern anti-bot systems detect. Real > fake.
})();
"#;

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
            assert!(STEALTH_JS.contains(needle), "stealth JS missing: {needle}");
        }
    }

    #[test]
    fn stealth_js_does_not_spoof_webgl_or_hardware() {
        // These must stay genuine under headless with a real GPU. Check for actual
        // spoofing patterns, not the words themselves (they appear in the comment
        // that explains why we don't touch them).
        assert!(
            !STEALTH_JS.contains("getParameter"),
            "WebGL getParameter override present"
        );
        assert!(
            !STEALTH_JS.contains("'hardwareConcurrency'"),
            "hardwareConcurrency is being defined"
        );
        assert!(
            !STEALTH_JS.contains("'deviceMemory'"),
            "deviceMemory is being defined"
        );
    }
}
