//! Polymorphic browser identity generator.
//!
//! Each session gets a unique, internally consistent fingerprint:
//! UA, screen, WebGL GPU, timezone, fonts, hardware — all coherent.
//! Indistinguishable from a real browser because every value comes
//! from databases of real-world hardware.

use rand::Rng;
use serde::{Deserialize, Serialize};

/// A complete browser identity — all fields are internally consistent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserIdentity {
    pub platform: Platform,
    pub user_agent: String,
    pub accept_language: String,
    pub screen: ScreenProfile,
    pub webgl: WebGLProfile,
    pub hardware: HardwareProfile,
    pub timezone: String,
    pub locale: String,
    pub canvas_seed: u64,
    pub audio_seed: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Platform {
    MacOS,
    Windows,
    Linux,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenProfile {
    pub width: u32,
    pub height: u32,
    pub avail_width: u32,
    pub avail_height: u32,
    pub color_depth: u32,
    pub pixel_ratio: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebGLProfile {
    pub vendor: String,
    pub renderer: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareProfile {
    pub cores: u32,
    pub memory: u32, // deviceMemory in GB
    pub max_touch: u32,
}

// ─── Real-world data ───

const MAC_UAS: &[&str] = &[
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/133.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/132.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_5) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_4_1) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/133.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 13_6_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36",
];

const WIN_UAS: &[&str] = &[
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/133.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/132.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 11.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
];

const LINUX_UAS: &[&str] = &[
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36",
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/133.0.0.0 Safari/537.36",
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/132.0.0.0 Safari/537.36",
];

// Real GPU combos from actual hardware
const MAC_GPUS: &[(&str, &str)] = &[
    ("Google Inc. (Apple)", "ANGLE (Apple, Apple M1, OpenGL 4.1)"),
    ("Google Inc. (Apple)", "ANGLE (Apple, Apple M1 Pro, OpenGL 4.1)"),
    ("Google Inc. (Apple)", "ANGLE (Apple, Apple M1 Max, OpenGL 4.1)"),
    ("Google Inc. (Apple)", "ANGLE (Apple, Apple M2, OpenGL 4.1)"),
    ("Google Inc. (Apple)", "ANGLE (Apple, Apple M2 Pro, OpenGL 4.1)"),
    ("Google Inc. (Apple)", "ANGLE (Apple, Apple M3, OpenGL 4.1)"),
    ("Google Inc. (Apple)", "ANGLE (Apple, Apple M3 Pro, OpenGL 4.1)"),
    ("Google Inc. (Apple)", "ANGLE (Apple, Apple M4, OpenGL 4.1)"),
    ("Google Inc. (Intel Inc.)", "ANGLE (Intel Inc., Intel(R) UHD Graphics 630, OpenGL 4.1)"),
    ("Google Inc. (AMD)", "ANGLE (AMD, AMD Radeon Pro 5500M, OpenGL 4.1)"),
];

const WIN_GPUS: &[(&str, &str)] = &[
    ("Google Inc. (NVIDIA)", "ANGLE (NVIDIA, NVIDIA GeForce RTX 3060 Direct3D11 vs_5_0 ps_5_0, D3D11)"),
    ("Google Inc. (NVIDIA)", "ANGLE (NVIDIA, NVIDIA GeForce RTX 3070 Direct3D11 vs_5_0 ps_5_0, D3D11)"),
    ("Google Inc. (NVIDIA)", "ANGLE (NVIDIA, NVIDIA GeForce RTX 4060 Direct3D11 vs_5_0 ps_5_0, D3D11)"),
    ("Google Inc. (NVIDIA)", "ANGLE (NVIDIA, NVIDIA GeForce RTX 4070 Direct3D11 vs_5_0 ps_5_0, D3D11)"),
    ("Google Inc. (NVIDIA)", "ANGLE (NVIDIA, NVIDIA GeForce GTX 1660 Super Direct3D11 vs_5_0 ps_5_0, D3D11)"),
    ("Google Inc. (AMD)", "ANGLE (AMD, AMD Radeon RX 6700 XT Direct3D11 vs_5_0 ps_5_0, D3D11)"),
    ("Google Inc. (AMD)", "ANGLE (AMD, AMD Radeon RX 7800 XT Direct3D11 vs_5_0 ps_5_0, D3D11)"),
    ("Google Inc. (Intel)", "ANGLE (Intel, Intel(R) UHD Graphics 770 Direct3D11 vs_5_0 ps_5_0, D3D11)"),
    ("Google Inc. (Intel)", "ANGLE (Intel, Intel(R) Iris(R) Xe Graphics Direct3D11 vs_5_0 ps_5_0, D3D11)"),
];

const LINUX_GPUS: &[(&str, &str)] = &[
    ("Google Inc. (NVIDIA Corporation)", "ANGLE (NVIDIA Corporation, NVIDIA GeForce RTX 3060/PCIe/SSE2, OpenGL 4.5)"),
    ("Google Inc. (NVIDIA Corporation)", "ANGLE (NVIDIA Corporation, NVIDIA GeForce GTX 1080/PCIe/SSE2, OpenGL 4.5)"),
    ("Google Inc. (Intel)", "ANGLE (Intel, Mesa Intel(R) UHD Graphics 630 (CFL GT2), OpenGL 4.6)"),
    ("Google Inc. (AMD)", "ANGLE (AMD, AMD Radeon RX 580 (POLARIS10), OpenGL 4.6)"),
];

// Common screen resolutions by platform
const MAC_SCREENS: &[(u32, u32, f64)] = &[
    (1440, 900, 2.0),   // MacBook Air 13"
    (1512, 982, 2.0),   // MacBook Pro 14"
    (1728, 1117, 2.0),  // MacBook Pro 16"
    (1920, 1080, 2.0),  // External monitor
    (2560, 1440, 2.0),  // 27" monitor
    (1680, 1050, 2.0),  // MacBook Pro 15" older
];

const WIN_SCREENS: &[(u32, u32, f64)] = &[
    (1920, 1080, 1.0),
    (2560, 1440, 1.0),
    (1920, 1080, 1.25),
    (1920, 1080, 1.5),
    (3840, 2160, 1.5),
    (1366, 768, 1.0),
    (1600, 900, 1.0),
    (2560, 1080, 1.0),
];

const LINUX_SCREENS: &[(u32, u32, f64)] = &[
    (1920, 1080, 1.0),
    (2560, 1440, 1.0),
    (3840, 2160, 1.0),
    (1920, 1200, 1.0),
];

const TIMEZONES_EU: &[(&str, &str)] = &[
    ("Europe/Madrid", "es-ES,es;q=0.9,en;q=0.8"),
    ("Europe/London", "en-GB,en;q=0.9"),
    ("Europe/Paris", "fr-FR,fr;q=0.9,en;q=0.8"),
    ("Europe/Berlin", "de-DE,de;q=0.9,en;q=0.8"),
    ("Europe/Rome", "it-IT,it;q=0.9,en;q=0.8"),
    ("Europe/Amsterdam", "nl-NL,nl;q=0.9,en;q=0.8"),
    ("Europe/Lisbon", "pt-PT,pt;q=0.9,en;q=0.8"),
];

const TIMEZONES_US: &[(&str, &str)] = &[
    ("America/New_York", "en-US,en;q=0.9"),
    ("America/Chicago", "en-US,en;q=0.9"),
    ("America/Denver", "en-US,en;q=0.9"),
    ("America/Los_Angeles", "en-US,en;q=0.9"),
];

// ─── Generator ───

impl BrowserIdentity {
    /// Generate a random, internally consistent browser identity.
    pub fn random() -> Self {
        let mut rng = rand::thread_rng();

        // Match the REAL OS — TLS fingerprint must be consistent with UA.
        // Cloudflare compares JA3 hash (from the Chrome binary, OS-specific)
        // with the User-Agent. Mismatch = instant block.
        let platform = if cfg!(target_os = "macos") {
            Platform::MacOS
        } else if cfg!(target_os = "windows") {
            Platform::Windows
        } else {
            Platform::Linux
        };

        let (ua, gpu, screen, platform_str, cores_range, mem_range) = match platform {
            Platform::MacOS => {
                let ua = MAC_UAS[rng.gen_range(0..MAC_UAS.len())];
                let gpu = MAC_GPUS[rng.gen_range(0..MAC_GPUS.len())];
                let scr = MAC_SCREENS[rng.gen_range(0..MAC_SCREENS.len())];
                (ua, gpu, scr, "MacIntel", (4u32, 16u32), (8u32, 36u32))
            }
            Platform::Windows => {
                let ua = WIN_UAS[rng.gen_range(0..WIN_UAS.len())];
                let gpu = WIN_GPUS[rng.gen_range(0..WIN_GPUS.len())];
                let scr = WIN_SCREENS[rng.gen_range(0..WIN_SCREENS.len())];
                (ua, gpu, scr, "Win32", (4, 32), (4, 64))
            }
            Platform::Linux => {
                let ua = LINUX_UAS[rng.gen_range(0..LINUX_UAS.len())];
                let gpu = LINUX_GPUS[rng.gen_range(0..LINUX_GPUS.len())];
                let scr = LINUX_SCREENS[rng.gen_range(0..LINUX_SCREENS.len())];
                (ua, gpu, scr, "Linux x86_64", (4, 32), (4, 64))
            }
        };

        // Pick timezone weighted by platform
        let (tz, lang) = if matches!(platform, Platform::MacOS | Platform::Linux) && rng.gen_bool(0.6) {
            let tz = TIMEZONES_EU[rng.gen_range(0..TIMEZONES_EU.len())];
            (tz.0, tz.1)
        } else {
            let tz = TIMEZONES_US[rng.gen_range(0..TIMEZONES_US.len())];
            (tz.0, tz.1)
        };

        // Hardware coherent with platform
        let cores = [4, 6, 8, 10, 12, 16][rng.gen_range(0..6)].min(cores_range.1);
        let cores = cores.max(cores_range.0);
        let memory = [4, 8, 16, 32][rng.gen_range(0..4)].min(mem_range.1);
        let memory = memory.max(mem_range.0);

        let taskbar_height = match platform {
            Platform::MacOS => 25 + rng.gen_range(0..10),
            Platform::Windows => 40 + rng.gen_range(0..8),
            Platform::Linux => 28 + rng.gen_range(0..12),
        };

        BrowserIdentity {
            platform,
            user_agent: ua.to_string(),
            accept_language: lang.to_string(),
            screen: ScreenProfile {
                width: screen.0,
                height: screen.1,
                avail_width: screen.0,
                avail_height: screen.1 - taskbar_height,
                color_depth: 24,
                pixel_ratio: screen.2,
            },
            webgl: WebGLProfile {
                vendor: gpu.0.to_string(),
                renderer: gpu.1.to_string(),
            },
            hardware: HardwareProfile {
                cores,
                memory,
                max_touch: 0,
            },
            timezone: tz.to_string(),
            locale: lang.split(',').next().unwrap_or("en-US").to_string(),
            canvas_seed: rng.gen(),
            audio_seed: 0.99990 + rng.gen::<f64>() * 0.00020,
        }
    }

    /// Generate the stealth JS injection for this identity.
    pub fn to_stealth_js(&self) -> String {
        let scr = &self.screen;
        let gl = &self.webgl;
        let hw = &self.hardware;
        let gl_vendor = gl.vendor.replace('\'', "\\'");
        let gl_renderer = gl.renderer.replace('\'', "\\'");

        format!(r#"
            // ══ NeoBrowser Polymorphic Identity ══

            // WebDriver — nuclear removal
            (function() {{
                const proto = Object.getPrototypeOf(navigator);
                delete proto.webdriver;
                Object.defineProperty(navigator, 'webdriver', {{
                    get: () => false,
                    configurable: true,
                    enumerable: false,
                }});
                // Also nuke the CDP markers
                delete window.cdc_adoQpoasnfa76pfcZLmcfl_Array;
                delete window.cdc_adoQpoasnfa76pfcZLmcfl_Promise;
                delete window.cdc_adoQpoasnfa76pfcZLmcfl_Symbol;
            }})();

            // Platform & hardware
            Object.defineProperty(navigator, 'platform', {{get: () => '{platform}'}});
            Object.defineProperty(navigator, 'hardwareConcurrency', {{get: () => {cores}}});
            Object.defineProperty(navigator, 'deviceMemory', {{get: () => {memory}}});
            Object.defineProperty(navigator, 'maxTouchPoints', {{get: () => {touch}}});
            Object.defineProperty(navigator, 'languages', {{get: () => {languages}}});

            // Screen
            Object.defineProperty(screen, 'width', {{get: () => {sw}}});
            Object.defineProperty(screen, 'height', {{get: () => {sh}}});
            Object.defineProperty(screen, 'availWidth', {{get: () => {aw}}});
            Object.defineProperty(screen, 'availHeight', {{get: () => {ah}}});
            Object.defineProperty(screen, 'colorDepth', {{get: () => {cd}}});
            Object.defineProperty(screen, 'pixelDepth', {{get: () => {cd}}});
            Object.defineProperty(window, 'devicePixelRatio', {{get: () => {dpr}}});
            Object.defineProperty(window, 'innerWidth', {{get: () => {sw}}});
            Object.defineProperty(window, 'innerHeight', {{get: () => {ah}}});
            Object.defineProperty(window, 'outerWidth', {{get: () => {sw}}});
            Object.defineProperty(window, 'outerHeight', {{get: () => {sh}}});

            // Visibility
            Object.defineProperty(document, 'hidden', {{get: () => false}});
            Object.defineProperty(document, 'visibilityState', {{get: () => 'visible'}});

            // Connection
            if (!navigator.connection) {{
                Object.defineProperty(navigator, 'connection', {{
                    get: () => ({{effectiveType: '4g', rtt: {rtt}, downlink: {downlink}, saveData: false}}),
                }});
            }}

            // Chrome runtime polyfill
            if (!window.chrome) window.chrome = {{}};
            if (!window.chrome.runtime) {{
                window.chrome.runtime = {{
                    connect: () => {{}}, sendMessage: () => {{}},
                    onMessage: {{ addListener: () => {{}}, removeListener: () => {{}} }},
                }};
            }}
            if (!window.chrome.csi) window.chrome.csi = () => ({{}});
            if (!window.chrome.loadTimes) window.chrome.loadTimes = () => ({{}});

            // WebGL — spoofed with identity GPU
            (function() {{
                const fakeParams = {{
                    37445: '{gl_vendor}',
                    37446: '{gl_renderer}',
                }};
                if (typeof WebGLRenderingContext !== 'undefined') {{
                    const gp = WebGLRenderingContext.prototype.getParameter;
                    WebGLRenderingContext.prototype.getParameter = function(p) {{
                        return fakeParams[p] || gp.call(this, p);
                    }};
                }}
                if (typeof WebGL2RenderingContext !== 'undefined') {{
                    const gp2 = WebGL2RenderingContext.prototype.getParameter;
                    WebGL2RenderingContext.prototype.getParameter = function(p) {{
                        return fakeParams[p] || gp2.call(this, p);
                    }};
                }}
                // Headless fallback — fake WebGL context if none available
                const origGC = HTMLCanvasElement.prototype.getContext;
                HTMLCanvasElement.prototype.getContext = function(type, ...args) {{
                    const ctx = origGC.apply(this, [type, ...args]);
                    if (ctx) return ctx;
                    if (type === 'webgl' || type === 'webgl2' || type === 'experimental-webgl') {{
                        return new Proxy({{}}, {{
                            get(t, prop) {{
                                if (prop === 'getParameter') return (p) => fakeParams[p] || 0;
                                if (prop === 'getExtension') return (n) => n === 'WEBGL_debug_renderer_info' ? {{}} : null;
                                if (prop === 'getSupportedExtensions') return () => ['WEBGL_debug_renderer_info'];
                                if (prop === 'canvas') return this;
                                return typeof t[prop] === 'function' ? () => {{}} : t[prop];
                            }}
                        }});
                    }}
                    return ctx;
                }};
            }})();

            // Canvas — deterministic noise unique to this identity
            (function() {{
                const seed = {canvas_seed}n;
                const origToDataURL = HTMLCanvasElement.prototype.toDataURL;
                const origGetImageData = CanvasRenderingContext2D.prototype.getImageData;
                function noise(canvas) {{
                    const ctx = canvas.getContext('2d');
                    if (!ctx) return;
                    try {{
                        const img = origGetImageData.call(ctx, 0, 0, canvas.width, canvas.height);
                        const d = img.data;
                        for (let i = 0; i < d.length; i += 4) {{
                            const n = Number((seed + BigInt(i) * 31n) % 5n) - 2;
                            d[i] = Math.max(0, Math.min(255, d[i] + n));
                        }}
                        ctx.putImageData(img, 0, 0);
                    }} catch(e) {{}}
                }}
                HTMLCanvasElement.prototype.toDataURL = function(...a) {{
                    noise(this); return origToDataURL.apply(this, a);
                }};
            }})();

            // Audio — unique gain per identity
            (function() {{
                if (typeof AudioContext === 'undefined') return;
                const orig = AudioContext.prototype.createOscillator;
                AudioContext.prototype.createOscillator = function() {{
                    const osc = orig.call(this);
                    const origConnect = osc.connect.bind(osc);
                    osc.connect = function(dest) {{
                        const gain = this.context.createGain();
                        gain.gain.value = {audio_seed};
                        origConnect(gain);
                        gain.connect(dest);
                        return dest;
                    }};
                    return osc;
                }};
            }})();

            // Plugins (Chrome always has these)
            (function() {{
                if (navigator.plugins.length === 0) {{
                    Object.defineProperty(navigator, 'plugins', {{
                        get: () => {{
                            const p = [
                                {{name:'Chrome PDF Plugin',filename:'internal-pdf-viewer',description:'Portable Document Format'}},
                                {{name:'Chrome PDF Viewer',filename:'mhjfbmdgcfjbbpaeojofohoefgiehjai',description:''}},
                                {{name:'Native Client',filename:'internal-nacl-plugin',description:''}},
                            ];
                            p.length = 3;
                            p.item = (i) => p[i];
                            p.namedItem = (n) => p.find(x => x.name === n);
                            p.refresh = () => {{}};
                            return p;
                        }}
                    }});
                }}
            }})();

            // Permissions
            if (navigator.permissions) {{
                const origQ = navigator.permissions.query.bind(navigator.permissions);
                navigator.permissions.query = (p) => {{
                    if (p.name === 'notifications') return Promise.resolve({{state:'prompt',onchange:null}});
                    return origQ(p);
                }};
            }}

            // Iframe webdriver propagation
            (function() {{
                const desc = Object.getOwnPropertyDescriptor(HTMLIFrameElement.prototype, 'contentWindow');
                if (desc) {{
                    Object.defineProperty(HTMLIFrameElement.prototype, 'contentWindow', {{
                        get: function() {{
                            const w = desc.get.call(this);
                            if (w) {{ try {{ delete w.navigator.__proto__.webdriver; }} catch(e) {{}} }}
                            return w;
                        }}
                    }});
                }}
            }})();
        "#,
            platform = match self.platform { Platform::MacOS => "MacIntel", Platform::Windows => "Win32", Platform::Linux => "Linux x86_64" },
            cores = hw.cores,
            memory = hw.memory,
            touch = hw.max_touch,
            languages = format!("['{}']", self.accept_language.split(',').map(|l| l.split(';').next().unwrap_or("en")).collect::<Vec<_>>().join("','")),
            sw = scr.width, sh = scr.height,
            aw = scr.avail_width, ah = scr.avail_height,
            cd = scr.color_depth, dpr = scr.pixel_ratio,
            rtt = 50 + (self.canvas_seed % 100) as u32,
            downlink = 5 + (self.canvas_seed % 15) as u32,
            gl_vendor = gl_vendor,
            gl_renderer = gl_renderer,
            canvas_seed = self.canvas_seed,
            audio_seed = self.audio_seed,
        )
    }

    /// Platform string for CDP Network.setUserAgentOverride
    pub fn platform_str(&self) -> &str {
        match self.platform {
            Platform::MacOS => "MacIntel",
            Platform::Windows => "Win32",
            Platform::Linux => "Linux x86_64",
        }
    }
}

impl std::fmt::Display for BrowserIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?} | {} | {}x{} | {} | {}",
            self.platform,
            self.user_agent.split("Chrome/").nth(1).unwrap_or("?").split(' ').next().unwrap_or("?"),
            self.screen.width, self.screen.height,
            self.webgl.renderer.split(',').nth(1).unwrap_or("?").trim(),
            self.timezone,
        )
    }
}
