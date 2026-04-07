//! Advanced stealth / anti-detection.
//!
//! Returns JS snippets to inject before page load.
//! Covers: canvas, webgl, audio, fonts, timezone, plugins.

/// Full stealth JS — inject via Page.addScriptToEvaluateOnNewDocument.
/// This supplements the basic stealth (webdriver, UA, chrome.runtime) already in engine.rs.
pub fn advanced_stealth_js() -> &'static str {
    r#"
    // ── Canvas fingerprint noise ──
    (function() {
        const origToDataURL = HTMLCanvasElement.prototype.toDataURL;
        const origToBlob = HTMLCanvasElement.prototype.toBlob;
        const origGetImageData = CanvasRenderingContext2D.prototype.getImageData;

        function addNoise(canvas) {
            const ctx = canvas.getContext('2d');
            if (!ctx) return;
            const imageData = origGetImageData.call(ctx, 0, 0, canvas.width, canvas.height);
            const data = imageData.data;
            // Subtle deterministic noise based on canvas size (consistent per session)
            const seed = canvas.width * 7 + canvas.height * 13;
            for (let i = 0; i < data.length; i += 4) {
                const noise = ((seed + i * 31) % 5) - 2; // -2 to +2
                data[i] = Math.max(0, Math.min(255, data[i] + noise));
            }
            ctx.putImageData(imageData, 0, 0);
        }

        HTMLCanvasElement.prototype.toDataURL = function(...args) {
            addNoise(this);
            return origToDataURL.apply(this, args);
        };
        HTMLCanvasElement.prototype.toBlob = function(...args) {
            addNoise(this);
            return origToBlob.apply(this, args);
        };
    })();

    // ── WebGL fingerprint spoofing (works even in headless without GPU) ──
    (function() {
        const fakeParams = {
            37445: 'Google Inc. (Apple)',      // UNMASKED_VENDOR_WEBGL
            37446: 'ANGLE (Apple, Apple M2, OpenGL 4.1)',  // UNMASKED_RENDERER_WEBGL
        };

        // Patch getParameter on existing WebGL contexts
        if (typeof WebGLRenderingContext !== 'undefined') {
            const getParam = WebGLRenderingContext.prototype.getParameter;
            WebGLRenderingContext.prototype.getParameter = function(param) {
                if (fakeParams[param]) return fakeParams[param];
                return getParam.call(this, param);
            };
        }
        if (typeof WebGL2RenderingContext !== 'undefined') {
            const getParam2 = WebGL2RenderingContext.prototype.getParameter;
            WebGL2RenderingContext.prototype.getParameter = function(param) {
                if (fakeParams[param]) return fakeParams[param];
                return getParam2.call(this, param);
            };
        }

        // In headless without GPU, canvas.getContext('webgl') returns null.
        // Intercept to return a minimal fake context for fingerprint checks.
        const origGetContext = HTMLCanvasElement.prototype.getContext;
        HTMLCanvasElement.prototype.getContext = function(type, ...args) {
            const ctx = origGetContext.apply(this, [type, ...args]);
            if (ctx) return ctx;
            if (type === 'webgl' || type === 'webgl2' || type === 'experimental-webgl') {
                // Return a proxy that answers fingerprint queries
                return new Proxy({}, {
                    get(target, prop) {
                        if (prop === 'getParameter') return (p) => fakeParams[p] || 0;
                        if (prop === 'getExtension') return () => null;
                        if (prop === 'getSupportedExtensions') return () => ['WEBGL_debug_renderer_info'];
                        if (prop === 'canvas') return this;
                        return typeof target[prop] === 'function' ? () => {} : target[prop];
                    }
                });
            }
            return ctx;
        };
    })();

    // ── AudioContext fingerprint ──
    (function() {
        const origCreateOscillator = AudioContext.prototype.createOscillator;
        const origCreateDynamicsCompressor = AudioContext.prototype.createDynamicsCompressor;
        AudioContext.prototype.createOscillator = function() {
            const osc = origCreateOscillator.call(this);
            const origConnect = osc.connect.bind(osc);
            osc.connect = function(dest) {
                // Add subtle gain variation
                const gain = this.context.createGain();
                gain.gain.value = 0.99997 + Math.random() * 0.00006;
                origConnect(gain);
                gain.connect(dest);
                return dest;
            };
            return osc;
        };
    })();

    // ── Navigator plugins (Chrome always has these) ──
    (function() {
        if (navigator.plugins.length === 0) {
            Object.defineProperty(navigator, 'plugins', {
                get: () => {
                    const plugins = [
                        {name: 'Chrome PDF Plugin', filename: 'internal-pdf-viewer', description: 'Portable Document Format'},
                        {name: 'Chrome PDF Viewer', filename: 'mhjfbmdgcfjbbpaeojofohoefgiehjai', description: ''},
                        {name: 'Native Client', filename: 'internal-nacl-plugin', description: ''},
                    ];
                    plugins.length = 3;
                    plugins.item = (i) => plugins[i];
                    plugins.namedItem = (n) => plugins.find(p => p.name === n);
                    plugins.refresh = () => {};
                    return plugins;
                }
            });
            Object.defineProperty(navigator, 'mimeTypes', {
                get: () => {
                    const mt = [
                        {type: 'application/pdf', suffixes: 'pdf', description: 'Portable Document Format'},
                    ];
                    mt.length = 1;
                    mt.item = (i) => mt[i];
                    mt.namedItem = (n) => mt.find(m => m.type === n);
                    return mt;
                }
            });
        }
    })();

    // ── Timezone coherence ──
    (function() {
        // Ensure Date and Intl agree on timezone
        const tz = Intl.DateTimeFormat().resolvedOptions().timeZone;
        if (!tz) {
            // Force a plausible timezone
            Object.defineProperty(Intl.DateTimeFormat.prototype, 'resolvedOptions', {
                value: function() {
                    const orig = Object.getPrototypeOf(this).resolvedOptions.call(this);
                    if (!orig.timeZone) orig.timeZone = 'Europe/Madrid';
                    return orig;
                }
            });
        }
    })();

    // ── Screen properties consistency ──
    (function() {
        if (screen.width === 0 || screen.height === 0) {
            Object.defineProperty(screen, 'width', {get: () => 1920});
            Object.defineProperty(screen, 'height', {get: () => 1080});
            Object.defineProperty(screen, 'availWidth', {get: () => 1920});
            Object.defineProperty(screen, 'availHeight', {get: () => 1055});
            Object.defineProperty(screen, 'colorDepth', {get: () => 24});
            Object.defineProperty(screen, 'pixelDepth', {get: () => 24});
        }
    })();

    // ── WebDriver detection via iframe ──
    (function() {
        const origContentWindow = Object.getOwnPropertyDescriptor(HTMLIFrameElement.prototype, 'contentWindow');
        Object.defineProperty(HTMLIFrameElement.prototype, 'contentWindow', {
            get: function() {
                const w = origContentWindow.get.call(this);
                if (w) {
                    try {
                        Object.defineProperty(w.navigator, 'webdriver', {get: () => undefined});
                    } catch(e) {}
                }
                return w;
            }
        });
    })();
    "#
}
