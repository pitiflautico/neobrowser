//! Finding a Chrome to drive, and asking it what it is.
//!
//! The version matters beyond diagnostics: the User-Agent is derived from the real binary,
//! because a UA claiming a Chrome version that does not match the browser's actual
//! behaviour is a stronger automation signal than sending no UA at all.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Locate a Chrome/Chromium binary cross-platform.
///
/// Honors `NEOBROWSER_CHROME_BIN` first, then probes the usual macOS app-bundle
/// paths, the PATH (Linux), and the standard Windows install locations. Falls
/// back to the macOS default so a failure names a concrete, fixable path.
pub fn discover_chrome_bin() -> PathBuf {
    if let Some(env) = std::env::var_os("NEOBROWSER_CHROME_BIN") {
        if !env.is_empty() {
            return PathBuf::from(env);
        }
    }
    let mac_paths = [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
    ];
    for p in mac_paths {
        if Path::new(p).exists() {
            return PathBuf::from(p);
        }
    }
    for name in [
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
        "chrome",
    ] {
        if let Some(found) = which(name) {
            return found;
        }
    }
    for p in [
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
    ] {
        if Path::new(p).exists() {
            return PathBuf::from(p);
        }
    }
    PathBuf::from(mac_paths[0])
}

/// Minimal `which`: search PATH for an executable by name.
fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// The discovered Chrome binary, cached process-wide.
pub fn chrome_bin() -> &'static Path {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(discover_chrome_bin).as_path()
}

/// Return the installed Chrome major version (e.g. "150"), or `None` if unknown.
pub fn detect_chrome_major(chrome_bin: &Path) -> Option<String> {
    let out = std::process::Command::new(chrome_bin)
        .arg("--version")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    // Match the first "<major>.<minor>" run of digits.
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            // Require a following '.' and another digit to look like a version.
            if i + 1 < bytes.len() && bytes[i] == b'.' && bytes[i + 1].is_ascii_digit() {
                return Some(text[start..i].to_string());
            }
        } else {
            i += 1;
        }
    }
    None
}

/// Build a User-Agent matching the REAL installed Chrome, consistent with its
/// genuine Client Hints. Applied via the `--user-agent` launch flag (which, unlike
/// CDP `Network.setUserAgentOverride`, does NOT blank Client Hints), turning the
/// only remaining headless tell (`HeadlessChrome`) into a clean identity.
pub fn chrome_user_agent() -> Option<&'static str> {
    static UA: OnceLock<Option<String>> = OnceLock::new();
    UA.get_or_init(|| {
        let major = detect_chrome_major(chrome_bin())?;
        let token = if cfg!(target_os = "windows") {
            "Windows NT 10.0; Win64; x64"
        } else if cfg!(target_os = "linux") {
            "X11; Linux x86_64"
        } else {
            // Darwin and anything else -> frozen macOS token.
            "Macintosh; Intel Mac OS X 10_15_7"
        };
        Some(format!(
            "Mozilla/5.0 ({token}) AppleWebKit/537.36 (KHTML, like Gecko) \
             Chrome/{major}.0.0.0 Safari/537.36"
        ))
    })
    .as_deref()
}

/// Headless launch flags — deliberately minimal and free of automation tells.
/// `--disable-blink-features=AutomationControlled` suppresses `navigator.webdriver`.
/// `--disable-gpu` is intentionally absent: under `--headless=new` the GPU works and
/// software WebGL (SwiftShader) is itself a headless fingerprint. Opt in via
/// `NEOBROWSER_DISABLE_GPU` on GPU-less CI hosts.
///
/// `--no-sandbox` is deliberately NOT here. NeoBrowser points a browser at
/// arbitrary untrusted pages, so the renderer sandbox is the last line between a
/// drive-by exploit and the user's machine — and in real-profile mode, their live
/// sessions. It is added only through the audited opt-in in `sandbox::resolve_sandbox`.
pub const DEFAULT_CHROME_FLAGS: &[&str] = &[
    "--headless=new",
    "--disable-dev-shm-usage",
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-background-networking",
    "--disable-sync",
    "--disable-translate",
    "--mute-audio",
    "--window-size=1920,1080",
    "--disable-blink-features=AutomationControlled",
    // Keep the renderer live: in --headless=new an occluded/backgrounded tab is
    // throttled, which stalls requestAnimationFrame / IntersectionObserver and
    // leaves virtualized lists and deferred dialogs unrendered. See browser.rs
    // (focus emulation) and page::nudge_frame for the rest of the fix.
    "--disable-backgrounding-occluded-windows",
    "--disable-renderer-backgrounding",
    "--disable-background-timer-throttling",
];
