// ═══════════════════════════════════════════════════════════════
// LAYOUT STUBS — realistic fake dimensions for fingerprint checks
// Loaded AFTER webapis.js. Overrides the zero-return stubs.
// ═══════════════════════════════════════════════════════════════

const _tagSizes = {
    'html': {w: 1920, h: 9000},
    'body': {w: 1920, h: 9000},
    'div': {w: 800, h: 100},
    'span': {w: 200, h: 20},
    'p': {w: 800, h: 60},
    'a': {w: 150, h: 20},
    'button': {w: 120, h: 40},
    'input': {w: 200, h: 30},
    'textarea': {w: 400, h: 100},
    'img': {w: 300, h: 200},
    'h1': {w: 800, h: 40},
    'h2': {w: 800, h: 35},
    'h3': {w: 800, h: 30},
    'h4': {w: 800, h: 26},
    'h5': {w: 800, h: 22},
    'h6': {w: 800, h: 20},
    'iframe': {w: 300, h: 150},
    'canvas': {w: 300, h: 150},
    'table': {w: 800, h: 400},
    'tr': {w: 800, h: 30},
    'td': {w: 200, h: 30},
    'th': {w: 200, h: 30},
    'form': {w: 600, h: 300},
    'nav': {w: 1920, h: 60},
    'header': {w: 1920, h: 80},
    'footer': {w: 1920, h: 200},
    'section': {w: 1920, h: 500},
    'article': {w: 800, h: 2000},
    'main': {w: 1200, h: 3000},
    'aside': {w: 300, h: 500},
    'ul': {w: 800, h: 200},
    'ol': {w: 800, h: 200},
    'li': {w: 780, h: 24},
    'label': {w: 150, h: 20},
    'select': {w: 200, h: 30},
    'video': {w: 640, h: 360},
    'audio': {w: 300, h: 54},
    'svg': {w: 200, h: 200},
};

// ── Fix 1: Element dimensions ────────────────────────────────

if (typeof Element !== 'undefined') {
    // getBoundingClientRect — realistic sizes based on tag
    Element.prototype.getBoundingClientRect = function() {
        const tag = (this.tagName || 'div').toLowerCase();
        const size = _tagSizes[tag] || {w: 100, h: 50};
        // Pseudo-position: stack siblings vertically
        const siblings = this.parentNode?.children || [];
        const idx = Array.from(siblings).indexOf(this);
        const y = idx * 30;
        return new DOMRect(0, y, size.w, size.h);
    };

    Element.prototype.getClientRects = function() {
        return [this.getBoundingClientRect()];
    };

    // offsetWidth/Height/Top/Left + client/scroll dimensions
    Object.defineProperties(Element.prototype, {
        offsetWidth:  { get() { return (_tagSizes[(this.tagName||'').toLowerCase()] || {w:100}).w; }, configurable: true },
        offsetHeight: { get() { return (_tagSizes[(this.tagName||'').toLowerCase()] || {h:50}).h; }, configurable: true },
        offsetTop:    { get() { const s = this.parentNode?.children; return s ? Array.from(s).indexOf(this) * 30 : 0; }, configurable: true },
        offsetLeft:   { get() { return 0; }, configurable: true },
        offsetParent: { get() { return this.parentNode; }, configurable: true },
        clientWidth:  { get() { return this.offsetWidth; }, configurable: true },
        clientHeight: { get() { return this.offsetHeight; }, configurable: true },
        clientTop:    { get() { return 0; }, configurable: true },
        clientLeft:   { get() { return 0; }, configurable: true },
        scrollWidth:  { get() { return this.offsetWidth; }, configurable: true },
        scrollHeight: { get() { return this.offsetHeight + 500; }, configurable: true },
        scrollTop:    { get() { return 0; }, set(v) {}, configurable: true },
        scrollLeft:   { get() { return 0; }, set(v) {}, configurable: true },
    });

    // scrollIntoView — no-op
    Element.prototype.scrollIntoView = function() {};
}

// document.body / document.documentElement dimensions
if (typeof document !== 'undefined') {
    try {
        if (document.body) {
            Object.defineProperty(document.body, 'clientWidth',  { get: () => 1920, configurable: true });
            Object.defineProperty(document.body, 'clientHeight', { get: () => 1080, configurable: true });
        }
        if (document.documentElement) {
            Object.defineProperty(document.documentElement, 'clientWidth',  { get: () => 1920, configurable: true });
            Object.defineProperty(document.documentElement, 'clientHeight', { get: () => 1080, configurable: true });
        }
    } catch {}
}

// ── Fix 2: Canvas fingerprint (consistent, not empty) ────────

if (typeof document !== 'undefined' && document.createElement) {
    const _origCreateEl = document.createElement.bind(document);
    document.createElement = function(tag, ...args) {
        const el = _origCreateEl(tag, ...args);
        if (tag.toLowerCase() === 'canvas') {
            // Consistent non-empty PNG (1x1 white pixel)
            el.toDataURL = function(type) {
                return 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==';
            };
            el.toBlob = function(cb, type, quality) {
                if (cb) cb(new Blob(['fake-png'], { type: type || 'image/png' }));
            };
            // Canvas dimensions
            el.width = el.width || 300;
            el.height = el.height || 150;

            const _origGetContext = el.getContext?.bind(el);
            el.getContext = function(type) {
                const ctx = _origGetContext ? _origGetContext(type) : {};
                if (type === '2d' || type === '2D') {
                    // Enhance measureText with full TextMetrics
                    ctx.measureText = function(text) {
                        const fontSize = parseFloat(ctx.font) || 10;
                        const ratio = fontSize / 10;
                        return {
                            width: text.length * 7.2 * ratio,
                            actualBoundingBoxAscent: 8 * ratio,
                            actualBoundingBoxDescent: 2 * ratio,
                            fontBoundingBoxAscent: 10 * ratio,
                            fontBoundingBoxDescent: 3 * ratio,
                            actualBoundingBoxLeft: 0,
                            actualBoundingBoxRight: text.length * 7.2 * ratio,
                            emHeightAscent: 8 * ratio,
                            emHeightDescent: 2 * ratio,
                        };
                    };
                    // getImageData returns non-zero data (subtle noise)
                    const _origGetImageData = ctx.getImageData;
                    ctx.getImageData = function(sx, sy, sw, sh) {
                        const w = sw || 1, h = sh || 1;
                        const data = new Uint8ClampedArray(w * h * 4);
                        // Fill with subtle deterministic noise (not all zeros)
                        for (let i = 0; i < data.length; i += 4) {
                            const seed = (i * 2654435761) >>> 0; // Knuth hash
                            data[i]     = (seed & 0xFF);         // R
                            data[i + 1] = ((seed >> 8) & 0xFF);  // G
                            data[i + 2] = ((seed >> 16) & 0xFF); // B
                            data[i + 3] = 255;                   // A (opaque)
                        }
                        return { data, width: w, height: h };
                    };
                    ctx.canvas = el;
                }
                if (type === 'webgl' || type === 'webgl2' || type === 'experimental-webgl') {
                    return _createWebGLContext(el);
                }
                return ctx;
            };
        }
        return el;
    };
}

// Minimal WebGL context stub (fingerprinters check getParameter, getExtension)
function _createWebGLContext(canvas) {
    const _glNoop = () => {};
    return {
        canvas,
        drawingBufferWidth: canvas.width || 300,
        drawingBufferHeight: canvas.height || 150,
        getParameter(pname) {
            // Common WebGL parameters fingerprinters check
            const params = {
                7936: 'WebKit',           // VENDOR
                7937: 'WebKit WebGL',     // RENDERER
                7938: 'WebGL 1.0',        // VERSION
                35724: 'WebGL GLSL ES 1.0', // SHADING_LANGUAGE_VERSION
                3379: 16384,              // MAX_TEXTURE_SIZE
                34076: 16384,             // MAX_CUBE_MAP_TEXTURE_SIZE
                34024: 16384,             // MAX_RENDERBUFFER_SIZE
                36347: 1024,              // MAX_TEXTURE_IMAGE_UNITS
                36348: 16,                // MAX_VERTEX_TEXTURE_IMAGE_UNITS
                35661: 16,                // MAX_COMBINED_TEXTURE_IMAGE_UNITS
                34921: 16,                // MAX_VERTEX_ATTRIBS
                36349: 1024,              // MAX_VERTEX_UNIFORM_VECTORS
                36348: 16,                // MAX_VARYING_VECTORS
                36345: 4096,              // MAX_FRAGMENT_UNIFORM_VECTORS
                3386: new Int32Array([32767, 32767]),  // MAX_VIEWPORT_DIMS
                33901: new Float32Array([1, 1024]),    // ALIASED_LINE_WIDTH_RANGE
                33902: new Float32Array([1, 1024]),    // ALIASED_POINT_SIZE_RANGE
            };
            return params[pname] !== undefined ? params[pname] : null;
        },
        getExtension(name) {
            // Common extensions that fingerprinters enumerate
            const knownExtensions = [
                'WEBGL_debug_renderer_info', 'EXT_texture_filter_anisotropic',
                'WEBKIT_EXT_texture_filter_anisotropic',
            ];
            if (name === 'WEBGL_debug_renderer_info') {
                return { UNMASKED_VENDOR_WEBGL: 37445, UNMASKED_RENDERER_WEBGL: 37446 };
            }
            if (knownExtensions.includes(name)) return {};
            return null;
        },
        getSupportedExtensions() {
            return ['WEBGL_debug_renderer_info', 'EXT_texture_filter_anisotropic',
                    'OES_texture_float', 'OES_element_index_uint', 'OES_standard_derivatives'];
        },
        getShaderPrecisionFormat() {
            return { rangeMin: 127, rangeMax: 127, precision: 23 };
        },
        createBuffer: _glNoop, bindBuffer: _glNoop, bufferData: _glNoop,
        createShader: () => ({}), shaderSource: _glNoop, compileShader: _glNoop,
        getShaderParameter: () => true, createProgram: () => ({}),
        attachShader: _glNoop, linkProgram: _glNoop, getProgramParameter: () => true,
        useProgram: _glNoop, getAttribLocation: () => 0, getUniformLocation: () => ({}),
        enableVertexAttribArray: _glNoop, vertexAttribPointer: _glNoop,
        uniform1f: _glNoop, uniform2f: _glNoop, uniform3f: _glNoop, uniform4f: _glNoop,
        uniformMatrix4fv: _glNoop, drawArrays: _glNoop, drawElements: _glNoop,
        viewport: _glNoop, clear: _glNoop, clearColor: _glNoop, enable: _glNoop, disable: _glNoop,
        blendFunc: _glNoop, depthFunc: _glNoop, scissor: _glNoop,
        createTexture: () => ({}), bindTexture: _glNoop, texImage2D: _glNoop,
        texParameteri: _glNoop, activeTexture: _glNoop, pixelStorei: _glNoop,
        createFramebuffer: () => ({}), bindFramebuffer: _glNoop,
        framebufferTexture2D: _glNoop, checkFramebufferStatus: () => 36053, // COMPLETE
        readPixels(x, y, w, h, format, type, pixels) {
            // Fill with deterministic data
            if (pixels && pixels.length) {
                for (let i = 0; i < pixels.length; i++) {
                    pixels[i] = ((i * 2654435761) >>> 24) & 0xFF;
                }
            }
        },
        getContextAttributes: () => ({
            alpha: true, antialias: true, depth: true, failIfMajorPerformanceCaveat: false,
            powerPreference: 'default', premultipliedAlpha: true, preserveDrawingBuffer: false,
            stencil: false, desynchronized: false,
        }),
        isContextLost: () => false,
    };
}

// ── Fix 3: getComputedStyle (realistic CSS defaults) ─────────

globalThis.getComputedStyle = function(el, pseudo) {
    const tag = (el?.tagName || 'div').toLowerCase();
    const inlineTags = ['span','a','strong','em','b','i','code','small','abbr','cite','sub','sup'];
    const defaults = {
        'display': inlineTags.includes(tag) ? 'inline' : 'block',
        'visibility': 'visible',
        'opacity': '1',
        'position': 'static',
        'overflow': 'visible',
        'font-size': '16px',
        'font-family': 'Arial, sans-serif',
        'font-weight': /^h[1-6]$/.test(tag) || tag === 'b' || tag === 'strong' ? '700' : '400',
        'font-style': tag === 'em' || tag === 'i' ? 'italic' : 'normal',
        'line-height': 'normal',
        'color': 'rgb(0, 0, 0)',
        'background-color': 'rgba(0, 0, 0, 0)',
        'width': (_tagSizes[tag]?.w || 100) + 'px',
        'height': (_tagSizes[tag]?.h || 50) + 'px',
        'margin': '0px',
        'margin-top': '0px',
        'margin-right': '0px',
        'margin-bottom': '0px',
        'margin-left': '0px',
        'padding': '0px',
        'padding-top': '0px',
        'padding-right': '0px',
        'padding-bottom': '0px',
        'padding-left': '0px',
        'border': '0px none rgb(0, 0, 0)',
        'border-top-width': '0px',
        'border-right-width': '0px',
        'border-bottom-width': '0px',
        'border-left-width': '0px',
        'border-top-style': 'none',
        'border-right-style': 'none',
        'border-bottom-style': 'none',
        'border-left-style': 'none',
        'box-sizing': 'content-box',
        'cursor': tag === 'a' || tag === 'button' ? 'pointer' : 'auto',
        'text-decoration': tag === 'a' ? 'underline' : 'none',
        'text-align': 'start',
        'vertical-align': 'baseline',
        'float': 'none',
        'clear': 'none',
        'z-index': 'auto',
        'transform': 'none',
        'transition': 'all 0s ease 0s',
        'pointer-events': 'auto',
        'user-select': 'auto',
    };

    // Also include inline styles from the element
    if (el?.style) {
        try {
            for (let i = 0; i < (el.style.length || 0); i++) {
                const prop = el.style[i];
                if (prop) defaults[prop] = el.style.getPropertyValue(prop);
            }
        } catch {}
    }

    return new Proxy(defaults, {
        get(target, prop) {
            if (prop === 'getPropertyValue') return (name) => target[name] || '';
            if (prop === 'length') return Object.keys(target).length;
            if (prop === 'cssText') return Object.entries(target).map(([k,v]) => `${k}: ${v}`).join('; ');
            if (prop === 'item') return (i) => Object.keys(target)[i] || '';
            if (prop === Symbol.iterator) return function*() { for (const k of Object.keys(target)) yield k; };
            if (typeof prop === 'symbol') return undefined;
            // camelCase → kebab-case lookup
            const kebab = String(prop).replace(/[A-Z]/g, m => '-' + m.toLowerCase());
            return target[prop] || target[kebab] || '';
        }
    });
};

// ═══════════════════════════════════════════════════════════════
// VIEW TRANSITIONS API — React 19+ uses document.startViewTransition()
// Without this, React's hydration crashes with "null.then()" error.
// ═══════════════════════════════════════════════════════════════
if (typeof document !== 'undefined' && !document.startViewTransition) {
    document.startViewTransition = function(callbackOrOptions) {
        const cb = typeof callbackOrOptions === 'function' ? callbackOrOptions : callbackOrOptions?.update;
        const result = cb ? cb() : undefined;
        const done = result instanceof Promise ? result : Promise.resolve();
        return {
            finished: done,
            ready: Promise.resolve(),
            updateCallbackDone: done,
            skipTransition: function() {},
        };
    };
}


// ═══════════════════════════════════════════════════════════════
// ERROR TRAP — capture unhandled rejections with stack traces
// ═══════════════════════════════════════════════════════════════
globalThis.__neo_hydration_errors = [];
globalThis.addEventListener('unhandledrejection', (e) => {
    const err = e.reason;
    globalThis.__neo_hydration_errors.push({
        message: err?.message || String(err),
        stack: err?.stack?.split('\n').slice(0, 5),
    });
});
