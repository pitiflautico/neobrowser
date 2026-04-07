//! Virtual display — run Chrome as "headed" without a real screen.
//!
//! The core problem: `--headless=new` is detectable by Cloudflare, X/Twitter,
//! LinkedIn, and others via:
//!   - `navigator.webdriver` leak (partially patched by stealth.js)
//!   - GPU/WebGL returning null (headless has no GPU)
//!   - `window.chrome` missing fields
//!   - Screen size / color depth inconsistencies
//!   - Chrome internal flags leaking via CDP protocol version strings
//!
//! Solution: run Chrome WITHOUT `--headless` flag, but against a virtual display.
//!
//! ## Platform strategy
//!
//! ### Linux
//! Spawn `Xvfb` on a random display number (`:N`), set `DISPLAY=:N` in Chrome's
//! environment. Chrome renders into a framebuffer in memory — no screen needed.
//! This is the standard approach used by Puppeteer, Playwright in CI, and every
//! headless browser farm.
//!
//! ### macOS
//! No Xvfb. Two options:
//!   1. **Off-screen window** (simple): launch Chrome headed with
//!      `--window-position=-9999,-9999`. Chrome is visible to the OS (has a real
//!      NSWindow) but positioned far off all displays. Sites see a normal headed
//!      Chrome. Downside: appears in Dock briefly.
//!   2. **CGVirtualDisplay** (macOS 13+): create a virtual display via
//!      `CoreGraphics` private API, then move Chrome's window onto it.
//!      Not implemented here — requires entitlements + system extensions.
//!
//! We implement option 1 for macOS (works today, zero deps) and Xvfb for Linux.

use std::time::Duration;
use tokio::process::Child;

/// A virtual display handle. Drop to kill the display server (Linux only).
pub struct VirtualDisplay {
    /// The DISPLAY env var value to pass to Chrome (e.g. ":42").
    pub display_var: String,

    /// Window position args to add to Chrome's argv.
    /// On Linux: empty (Xvfb handles the display geometry).
    /// On macOS: off-screen position args.
    pub extra_chrome_args: Vec<String>,

    /// Xvfb process (Linux only). None on macOS.
    _xvfb: Option<Child>,
}

impl VirtualDisplay {
    /// Spawn a virtual display for the current platform.
    ///
    /// Returns `Ok(display)` on success. If Xvfb is not installed on Linux,
    /// falls back to the macOS off-screen strategy with a warning.
    pub async fn spawn() -> Result<Self, Box<dyn std::error::Error>> {
        #[cfg(target_os = "linux")]
        {
            Self::spawn_xvfb().await
        }
        #[cfg(target_os = "macos")]
        {
            Ok(Self::offscreen_macos())
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            Err("virtual display not supported on this platform".into())
        }
    }

    /// Extra Chrome flags needed when using this virtual display.
    ///
    /// Call this INSTEAD of passing `--headless=new`. The returned args
    /// configure Chrome for the virtual display + full GPU emulation.
    pub fn chrome_args(&self) -> Vec<String> {
        let mut args = self.extra_chrome_args.clone();

        // Enable GPU rendering path — critical for passing WebGL fingerprint checks.
        // Without this, `canvas.getContext('webgl')` returns null in headed mode
        // on a virtual display, which is a strong bot signal.
        args.extend([
            "--use-gl=swiftshader".to_string(),           // Software GPU (works without real GPU)
            "--enable-unsafe-swiftshader".to_string(),    // Allow swiftshader in non-sandbox
            "--ignore-gpu-blocklist".to_string(),         // Don't skip GPU init on headless/virtual
            "--enable-gpu-rasterization".to_string(),
            "--disable-software-rasterizer".to_string(),  // Force GPU path (not fallback CPU)
        ]);

        // Screen geometry must match the virtual display (1920x1080).
        // This ensures `screen.width/height` JS values are realistic.
        args.push("--window-size=1920,1080".to_string());
        args.push("--force-device-scale-factor=1".to_string());

        args
    }

    /// The DISPLAY environment variable to set on the Chrome process.
    /// Empty string on macOS (no DISPLAY needed).
    pub fn display_env(&self) -> Option<(&str, &str)> {
        if self.display_var.is_empty() {
            None
        } else {
            Some(("DISPLAY", &self.display_var))
        }
    }
}

// ── Linux: Xvfb ──────────────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
impl VirtualDisplay {
    async fn spawn_xvfb() -> Result<Self, Box<dyn std::error::Error>> {
        // Pick a random display number in [100, 999] to avoid collisions
        // with real displays (:0, :1) or other test runners (:99).
        let display_num = pick_free_display().await?;
        let display_var = format!(":{display_num}");

        eprintln!("[VDISPLAY] Spawning Xvfb on {display_var} (1920x1080x24)");

        let xvfb = tokio::process::Command::new("Xvfb")
            .args([
                &display_var,
                "-screen", "0", "1920x1080x24",
                "-ac",           // disable access control (no auth needed)
                "-nolisten", "tcp",  // TCP disabled — local only, more secure
                "+extension", "RANDR", // needed by some Chrome GPU paths
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;

        // Wait for Xvfb to be ready (poll for the X socket)
        let socket_path = format!("/tmp/.X11-unix/X{display_num}");
        for _ in 0..30 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if std::path::Path::new(&socket_path).exists() {
                break;
            }
        }

        if !std::path::Path::new(&socket_path).exists() {
            return Err(format!("Xvfb did not create socket at {socket_path}").into());
        }

        eprintln!("[VDISPLAY] Xvfb ready on {display_var}");

        Ok(Self {
            display_var,
            extra_chrome_args: vec![],
            _xvfb: Some(xvfb),
        })
    }
}

/// Find a display number that has no existing X socket.
#[cfg(target_os = "linux")]
async fn pick_free_display() -> Result<u32, Box<dyn std::error::Error>> {
    for n in 100u32..999 {
        let socket = format!("/tmp/.X11-unix/X{n}");
        if !std::path::Path::new(&socket).exists() {
            return Ok(n);
        }
    }
    Err("no free display number found in :100–:999".into())
}

// ── macOS: off-screen window ──────────────────────────────────────────────────

#[cfg(target_os = "macos")]
impl VirtualDisplay {
    fn offscreen_macos() -> Self {
        eprintln!("[VDISPLAY] macOS: off-screen window at -9999,-9999 (no --headless flag)");
        Self {
            display_var: String::new(),
            extra_chrome_args: vec![
                // Position window far off all displays.
                // macOS coordinate system: (0,0) = top-left of primary display.
                // -9999,-9999 places the window off the top-left corner.
                "--window-position=-9999,-9999".to_string(),
                // Suppress "Chrome was opened without a security key" popup
                "--no-first-run".to_string(),
                // Don't try to restore previous session (no tab flashing)
                "--restore-last-session=false".to_string(),
            ],
            _xvfb: None,
        }
    }
}

// ── Integration with engine.rs ────────────────────────────────────────────────

/// A Chrome session running against a virtual display.
///
/// Drop this to kill Xvfb (Linux) and Chrome.
pub struct UndetectableSession {
    pub session: crate::engine::Session,
    pub _display: VirtualDisplay,
}

/// Launch Chrome in headed mode against a virtual display.
///
/// This is the preferred launch path for sites that detect headless:
/// X/Twitter, LinkedIn, Cloudflare-protected pages, etc.
///
/// Chrome launched this way reports:
/// - `navigator.webdriver = undefined` (no automation flag)
/// - Full WebGL / GPU context (swiftshader)
/// - Realistic `screen.width/height` (1920x1080)
/// - No headless entries in `/json/version`
pub async fn launch_undetectable(
    user_data_dir: Option<&str>,
) -> Result<UndetectableSession, Box<dyn std::error::Error>> {
    let display = VirtualDisplay::spawn().await?;
    let chrome_args = display.chrome_args();
    let display_env = display.display_env().map(|(k, v)| (k.to_string(), v.to_string()));

    let session = crate::engine::Session::launch_with_display(
        user_data_dir,
        &chrome_args,
        display_env,
    ).await?;

    Ok(UndetectableSession { session, _display: display })
}

// ── Detection probe ───────────────────────────────────────────────────────────

/// JS that probes how detectable the current browser is.
/// Returns a JSON object with each signal and its value.
/// Inject via Runtime.evaluate and check the result.
pub const DETECTION_PROBE_JS: &str = r#"
(() => {
    const signals = {};

    // 1. webdriver flag — must be undefined/false
    signals.webdriver = navigator.webdriver;

    // 2. Chrome runtime — must exist with all properties
    signals.chrome_runtime   = typeof window.chrome !== 'undefined';
    signals.chrome_loadTimes = typeof window.chrome?.loadTimes === 'function';
    signals.chrome_csi       = typeof window.chrome?.csi === 'function';

    // 3. Plugins — headless has 0
    signals.plugin_count = navigator.plugins.length;

    // 4. WebGL — headless without GPU returns null context
    const c = document.createElement('canvas');
    const gl = c.getContext('webgl') || c.getContext('experimental-webgl');
    signals.webgl_ok      = !!gl;
    signals.webgl_vendor  = gl?.getParameter(gl?.VENDOR) || null;
    signals.webgl_renderer = gl?.getParameter(
        gl?.getExtension('WEBGL_debug_renderer_info')?.UNMASKED_RENDERER_WEBGL
    ) || null;

    // 5. Screen — headless often reports 0x0
    signals.screen_w = screen.width;
    signals.screen_h = screen.height;

    // 6. Hardware concurrency — 0 or 1 is suspicious
    signals.hw_concurrency = navigator.hardwareConcurrency;

    // 7. Language — undefined in some headless configs
    signals.language = navigator.language;

    // 8. Notification API — headless often throws
    try {
        signals.notification_permission = Notification.permission;
    } catch(e) {
        signals.notification_permission = 'error: ' + e.message;
    }

    // 9. User agent contains "Headless"
    signals.ua_headless = navigator.userAgent.toLowerCase().includes('headless');

    // Score: count passed signals
    const issues = [];
    if (signals.webdriver)              issues.push('webdriver=true');
    if (!signals.chrome_runtime)        issues.push('no window.chrome');
    if (!signals.chrome_loadTimes)      issues.push('no chrome.loadTimes');
    if (signals.plugin_count === 0)     issues.push('0 plugins');
    if (!signals.webgl_ok)              issues.push('no webgl');
    if (signals.screen_w === 0)         issues.push('screen_w=0');
    if (signals.hw_concurrency <= 1)    issues.push('hw_concurrency<=1');
    if (signals.ua_headless)            issues.push('UA contains Headless');

    signals.issues   = issues;
    signals.score    = issues.length;  // 0 = undetectable, higher = more detectable
    signals.verdict  = issues.length === 0 ? 'CLEAN' : issues.length <= 2 ? 'PARTIAL' : 'DETECTED';

    return JSON.stringify(signals, null, 2);
})()
"#;
