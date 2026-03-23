// NeoRender V2 Browser Shim — intercepts browser behaviors that happy-dom can't handle.
// Loaded AFTER bootstrap.js. Overrides basic stubs with navigation-aware versions.
// Hooks into Rust ops: op_navigation_request, op_cookie_get, op_cookie_set.

// Use __neo_ops saved by bootstrap.js (Deno is deleted for sandbox security).
const _shimOps = globalThis.__neo_ops;

// ═══════════════════════════════════════════════════════════════
// INTERACTION TRACE — causal pipeline debugging
// ═══════════════════════════════════════════════════════════════

globalThis.__neo_interaction_trace = [];
globalThis.__neo_traceStep = function(step, data) {
    globalThis.__neo_interaction_trace.push({
        step: step,
        ts: Date.now(),
        data: typeof data === 'string' ? data : JSON.stringify(data)?.substring(0, 200)
    });
};

// Query trace
globalThis.__neo_getTrace = function() {
    return JSON.stringify(globalThis.__neo_interaction_trace.slice(-30));
};

// Clear trace
globalThis.__neo_clearTrace = function() {
    globalThis.__neo_interaction_trace = [];
};

// ── encodeURIComponent safety wrapper ──
// React Router passes route params to encodeURIComponent(). In our engine,
// some params (conversation IDs from turbo-stream decode) arrive as objects
// instead of strings. This wrapper auto-coerces to prevent crashes.
const _origEncodeURI = globalThis.encodeURIComponent;
globalThis.encodeURIComponent = function(v) {
    if (typeof v === 'object' && v !== null) return _origEncodeURI(String(v));
    return _origEncodeURI(v);
};

// ── React Router SSR turbo-stream interceptor ──
// React Router 7 SSR streams hydration data via __reactRouterContext.streamController.
// happy-dom's ReadableStream.getReader().read() hangs so the normal decode path fails.
// Fix: intercept WHEN __reactRouterContext is created (via defineProperty trap on window),
// then patch streamController.enqueue/close to accumulate data and decode with turbo-stream.
(function installReactRouterStreamInterceptor() {
    let _ctx = globalThis.__reactRouterContext;
    const _chunks = [];

    function patchController(ctx) {
        if (!ctx || !ctx.streamController || ctx.__neo_patched) return;
        ctx.__neo_patched = true;
        const sc = ctx.streamController;
        const origEnqueue = sc.enqueue?.bind(sc);
        const origClose = sc.close?.bind(sc);

        sc.enqueue = function(data) {
            _chunks.push(typeof data === 'string' ? data : new TextDecoder().decode(data));
            console.error('[TURBO-DEBUG] enqueue called, chunks so far: ' + _chunks.length);
            if (origEnqueue) try { origEnqueue(data); } catch(e) {}
        };

        sc.close = function() {
            const raw = _chunks.join('');
            console.error('[TURBO-DEBUG] close called, chunks: ' + _chunks.length + ', raw length: ' + raw.length);
            if (raw && typeof turboStream !== 'undefined' && turboStream.decode) {
                try {
                    // turboStream.decode() expects a ReadableStream, not a string.
                    // Wrap the accumulated data into a ReadableStream that immediately
                    // enqueues the full payload and closes.
                    var stream = new ReadableStream({
                        start: function(controller) {
                            controller.enqueue(raw);
                            controller.close();
                        }
                    });
                    // decode() is async — returns a Promise.
                    var decodePromise = turboStream.decode(stream);
                    decodePromise.then(function(result) {
                        if (result !== undefined && result !== null) {
                            ctx.state = result;
                            console.error('[turbo-stream] decoded SSR state, keys: ' + Object.keys(result).join(','));
                        }
                    }).catch(function(e) {
                        console.error('[turbo-stream] decode promise rejected: ' + e.message);
                    });
                } catch(e) {
                    console.error('[turbo-stream] decode failed: ' + e.message + '\n' + e.stack);
                }
            } else {
                console.error('[TURBO-DEBUG] skip decode: raw=' + !!raw + ', turboStream=' + (typeof turboStream) + ', decode=' + !!(typeof turboStream !== 'undefined' && turboStream.decode));
            }
            if (origClose) try { origClose(); } catch(e) {}
        };
    }

    // Patch if already exists
    if (_ctx) patchController(_ctx);

    // Trap future assignment via defineProperty
    try {
        Object.defineProperty(globalThis, '__reactRouterContext', {
            get() { return _ctx; },
            set(val) {
                _ctx = val;
                if (val && typeof val === 'object') patchController(val);
            },
            configurable: true, enumerable: true,
        });
    } catch(e) {}
})();

// ═══════════════════════════════════════════════════════════════
// 1. NAVIGATION INTERCEPTION — capture form.submit(), location changes
// ═══════════════════════════════════════════════════════════════

// Form submit helper — extracts form data and sends navigation request.
function __neoFormSubmit() {
    var formData = {};
    var inputs = this.querySelectorAll('input, select, textarea');
    inputs.forEach(function(el) {
        if (!el.name || el.disabled) return;
        if (el.type === 'checkbox' || el.type === 'radio') {
            if (el.checked) formData[el.name] = el.value || 'on';
        } else {
            // Support multiple values with same name (e.g. multi-select)
            if (formData[el.name] !== undefined) {
                if (!Array.isArray(formData[el.name])) formData[el.name] = [formData[el.name]];
                formData[el.name].push(el.value || '');
            } else {
                formData[el.name] = el.value || '';
            }
        }
    });
    var action = this.getAttribute('action') || this.action || globalThis.__neo_location.href || '';
    var method = (this.getAttribute('method') || this.method || 'GET').toUpperCase();
    try {
        _shimOps.op_navigation_request(JSON.stringify({
            url: action, method: method, form_data: formData, type: 'form_submit'
        }));
    } catch(e) {}
}

// Intercept form.submit() — happy-dom's form elements may have a different
// prototype chain than globalThis.HTMLFormElement, so we patch both the
// exported prototype AND the actual prototype from a real form element.
if (typeof HTMLFormElement !== 'undefined' && HTMLFormElement.prototype) {
    HTMLFormElement.prototype.submit = __neoFormSubmit;
}
// Also patch the actual prototype chain used by happy-dom's form elements.
if (typeof document !== 'undefined') {
    try {
        var __testForm = document.createElement('form');
        var __formProto = Object.getPrototypeOf(__testForm);
        // Walk up the prototype chain and add submit to the right level
        while (__formProto && __formProto !== Object.prototype) {
            if (!__formProto.submit) {
                __formProto.submit = __neoFormSubmit;
            }
            __formProto = Object.getPrototypeOf(__formProto);
        }
    } catch(e) {}
}

// ─── URL resolution helper (pure function, no side effects) ───
// NOTE: deno_core's URL(relative, base) has bugs with pathname resolution,
// so we use manual resolution instead of relying on the two-arg URL constructor.
function __neoResolveUrl(url, base) {
    // Absolute URL — return as-is
    if (url.indexOf('://') !== -1) return url;
    if (!base) return url;
    try {
        var b = new URL(base);
        if (url.charAt(0) === '/') {
            // Root-relative: use origin + the url (which may contain ?query and #hash)
            return b.origin + url;
        }
        if (url.charAt(0) === '#') {
            return b.origin + b.pathname + b.search + url;
        }
        if (url.charAt(0) === '?') {
            return b.origin + b.pathname + url;
        }
        // Relative path
        var dir = b.pathname.replace(/\/[^/]*$/, '/');
        return b.origin + dir + url;
    } catch(e) { return url; }
}

// ═══════════════════════════════════════════════════════════════
// LOCATION CLASS — proper constructor with instanceof support
// ═══════════════════════════════════════════════════════════════

// Symbol to guard constructor — prevents `new Location()` from userland
var __locationKey = Symbol('neo.location');

// Internal URL store — the source of truth for all Location properties
var __locationUrl = { href: 'about:blank' };

function __syncLocationUrl(href) {
    try {
        var u = new URL(href);
        __locationUrl = u;
    } catch(e) {
        // If URL parsing fails, keep previous state
    }
}

function Location(key) {
    if (key !== __locationKey) {
        throw new TypeError('Illegal constructor');
    }
}

Object.defineProperties(Location.prototype, {
    href: {
        get: function() { return __locationUrl.href; },
        set: function(val) {
            // Setting href triggers navigation (same as assign)
            var s = String(val);
            __syncLocationUrl(s);
            try {
                _shimOps.op_navigation_request(JSON.stringify({
                    url: s, method: 'GET', type: 'location_assign'
                }));
            } catch(e) {}
        },
        enumerable: true, configurable: false
    },
    origin: {
        get: function() { return __locationUrl.origin; },
        enumerable: true, configurable: false
    },
    protocol: {
        get: function() { return __locationUrl.protocol; },
        set: function(val) {
            try {
                var u = new URL(__locationUrl.href);
                u.protocol = val;
                __syncLocationUrl(u.href);
            } catch(e) {}
        },
        enumerable: true, configurable: false
    },
    host: {
        get: function() { return __locationUrl.host; },
        set: function(val) {
            try {
                var u = new URL(__locationUrl.href);
                u.host = val;
                __syncLocationUrl(u.href);
            } catch(e) {}
        },
        enumerable: true, configurable: false
    },
    hostname: {
        get: function() { return __locationUrl.hostname; },
        set: function(val) {
            try {
                var u = new URL(__locationUrl.href);
                u.hostname = val;
                __syncLocationUrl(u.href);
            } catch(e) {}
        },
        enumerable: true, configurable: false
    },
    port: {
        get: function() { return __locationUrl.port; },
        set: function(val) {
            try {
                var u = new URL(__locationUrl.href);
                u.port = val;
                __syncLocationUrl(u.href);
            } catch(e) {}
        },
        enumerable: true, configurable: false
    },
    pathname: {
        get: function() { return __locationUrl.pathname; },
        set: function(val) {
            try {
                var u = new URL(__locationUrl.href);
                u.pathname = val;
                __syncLocationUrl(u.href);
            } catch(e) {}
        },
        enumerable: true, configurable: false
    },
    search: {
        get: function() { return __locationUrl.search; },
        set: function(val) {
            try {
                var u = new URL(__locationUrl.href);
                u.search = val;
                __syncLocationUrl(u.href);
            } catch(e) {}
        },
        enumerable: true, configurable: false
    },
    hash: {
        get: function() { return __locationUrl.hash; },
        set: function(val) {
            try {
                var u = new URL(__locationUrl.href);
                u.hash = val;
                __syncLocationUrl(u.href);
            } catch(e) {}
        },
        enumerable: true, configurable: false
    },
    assign: {
        value: function assign(url) {
            try {
                _shimOps.op_navigation_request(JSON.stringify({
                    url: String(url), method: 'GET', type: 'location_assign'
                }));
            } catch(e) {}
        },
        writable: false, enumerable: true, configurable: false
    },
    replace: {
        value: function replace(url) {
            var s = String(url);
            // Filter out non-URL arguments (e.g. regex objects from String.replace confusion)
            if (!s || s.startsWith('/') && s.includes('[') || s.includes('(') || s === 'undefined') return;
            try {
                _shimOps.op_navigation_request(JSON.stringify({
                    url: s, method: 'GET', type: 'location_replace'
                }));
            } catch(e) {}
        },
        writable: false, enumerable: true, configurable: false
    },
    reload: {
        value: function reload() {
            try {
                _shimOps.op_navigation_request(JSON.stringify({
                    url: __locationUrl.href, method: 'GET', type: 'reload'
                }));
            } catch(e) {}
        },
        writable: false, enumerable: true, configurable: false
    },
    toString: {
        value: function toString() { return __locationUrl.href; },
        writable: false, enumerable: true, configurable: false
    },
    valueOf: {
        value: function valueOf() { return __locationUrl.href; },
        writable: false, enumerable: true, configurable: false
    },
    ancestorOrigins: {
        get: function() {
            return { length: 0, item: function() { return null; }, contains: function() { return false; } };
        },
        enumerable: true, configurable: false
    }
});

Object.defineProperty(Location.prototype, Symbol.toStringTag, {
    value: 'Location', configurable: true
});

// Expose Location constructor globally (for instanceof checks)
globalThis.Location = Location;

// Create THE singleton location instance
var __neo_location_instance = new Location(__locationKey);

// Backward compat: __neo_location points to the same instance
globalThis.__neo_location = __neo_location_instance;

// Internal: update location from a URL string WITHOUT triggering navigation ops.
// Used by History.pushState/replaceState and by Rust set_document_html.
function __neoUpdateLocation(url) {
    var resolved = __neoResolveUrl(url, __locationUrl.href);
    __syncLocationUrl(resolved);
}

// Rust calls `__loc.href = ...; __loc.protocol = ...` etc. on __neo_location.
// With our getter/setter properties on Location.prototype, setting .href triggers
// navigation. We need a way for Rust to set properties WITHOUT navigation.
// Solution: Rust sets via __neo_location which now has prototype getters/setters.
// The .href setter triggers navigation — but Rust's set_document_html wraps in try/catch
// and the op_navigation_request is a no-op during initial load.
// For safety, provide an explicit internal setter:
globalThis.__neo_setLocationHref = function(href) {
    __syncLocationUrl(href);
};

// Override globalThis.location with the Location instance
Object.defineProperty(globalThis, 'location', {
    get: function() { return __neo_location_instance; },
    set: function(val) {
        if (typeof val === 'string') {
            try {
                _shimOps.op_navigation_request(JSON.stringify({
                    url: val, method: 'GET', type: 'location_assign'
                }));
            } catch(e) {}
        }
    },
    configurable: true
});

// Also override on document if it exists
if (typeof document !== 'undefined') {
    try {
        Object.defineProperty(document, 'location', {
            get: function() { return __neo_location_instance; },
            set: function(val) { globalThis.location = val; },
            configurable: true
        });
    } catch(e) {}
}

// window.open — detect but don't follow
globalThis.open = function(url, target, features) {
    try {
        _shimOps.op_navigation_request(JSON.stringify({
            url: String(url || ''), method: 'GET', type: 'window_open', target: target || '_blank'
        }));
    } catch(e) {}
    return null;
};

// window.close — no-op
globalThis.close = function() {};

// ═══════════════════════════════════════════════════════════════
// HISTORY CLASS — proper constructor with instanceof support
// ═══════════════════════════════════════════════════════════════

var __historyKey = Symbol('neo.history');

// Internal state — shared between History instance and helpers
var __historyState = { entries: [], index: -1 };

// Backward compat
globalThis.__neo_history = __historyState;

function History(key) {
    if (key !== __historyKey) {
        throw new TypeError('Illegal constructor');
    }
}

Object.defineProperties(History.prototype, {
    length: {
        get: function() {
            return __historyState.entries.length || 1;
        },
        enumerable: true, configurable: true
    },
    state: {
        get: function() {
            var h = __historyState;
            if (h.index >= 0 && h.index < h.entries.length) {
                return h.entries[h.index].state;
            }
            return null;
        },
        enumerable: true, configurable: true
    },
    scrollRestoration: {
        get: function() { return 'auto'; },
        set: function(v) { /* no-op */ },
        enumerable: true, configurable: true
    },
    pushState: {
        value: function pushState(state, title, url) {
            var h = __historyState;
            // Truncate forward entries
            h.entries.length = h.index + 1;
            h.entries.push({ state: state, title: title, url: url, nav_type: 'synthetic' });
            h.index = h.entries.length - 1;
            // pushState syncs location but does NOT dispatch popstate (per spec)
            if (url) __neoUpdateLocation(url);
            console.log('[NAV-TRACE] pushState: ' + url);
            __neo_traceStep('pushState', url);
        },
        writable: true, enumerable: true, configurable: true
    },
    replaceState: {
        value: function replaceState(state, title, url) {
            var h = __historyState;
            if (h.entries.length > 0 && h.index >= 0) {
                h.entries[h.index] = { state: state, title: title, url: url, nav_type: 'synthetic' };
            } else {
                h.entries.push({ state: state, title: title, url: url, nav_type: 'synthetic' });
                h.index = 0;
            }
            // replaceState syncs location but does NOT dispatch popstate (per spec)
            if (url) __neoUpdateLocation(url);
            console.log('[NAV-TRACE] replaceState: ' + url);
            __neo_traceStep('replaceState', url);
        },
        writable: true, enumerable: true, configurable: true
    },
    back: {
        value: function back() {
            var h = __historyState;
            if (h.index > 0) {
                h.index--;
                var entry = h.entries[h.index];
                if (entry && entry.url) __neoUpdateLocation(entry.url);
                // Dispatch popstate — React Router listens for this
                try {
                    globalThis.dispatchEvent(new PopStateEvent('popstate', { state: entry ? entry.state : null }));
                } catch(e) {}
            }
        },
        writable: true, enumerable: true, configurable: true
    },
    forward: {
        value: function forward() {
            var h = __historyState;
            if (h.index < h.entries.length - 1) {
                h.index++;
                var entry = h.entries[h.index];
                if (entry && entry.url) __neoUpdateLocation(entry.url);
                try {
                    globalThis.dispatchEvent(new PopStateEvent('popstate', { state: entry ? entry.state : null }));
                } catch(e) {}
            }
        },
        writable: true, enumerable: true, configurable: true
    },
    go: {
        value: function go(delta) {
            if (!delta) return;
            var h = __historyState;
            var target = h.index + (delta | 0);
            if (target >= 0 && target < h.entries.length) {
                h.index = target;
                var entry = h.entries[h.index];
                if (entry && entry.url) __neoUpdateLocation(entry.url);
                try {
                    globalThis.dispatchEvent(new PopStateEvent('popstate', { state: entry ? entry.state : null }));
                } catch(e) {}
            }
        },
        writable: true, enumerable: true, configurable: true
    }
});

Object.defineProperty(History.prototype, Symbol.toStringTag, {
    value: 'History', configurable: true
});

// Expose History constructor globally (for instanceof checks)
globalThis.History = History;

// Create THE singleton history instance
globalThis.history = new History(__historyKey);

// ═══════════════════════════════════════════════════════════════
// 3. COOKIE ACCESS — backed by Rust ops
// ═══════════════════════════════════════════════════════════════

if (typeof document !== 'undefined') {
    Object.defineProperty(document, 'cookie', {
        get: function() {
            try { return _shimOps.op_cookie_get(); }
            catch(e) { return ''; }
        },
        set: function(val) {
            try { _shimOps.op_cookie_set(String(val)); }
            catch(e) {}
        },
        configurable: true
    });
}

// ═══════════════════════════════════════════════════════════════
// 4. FOCUS MANAGEMENT
// ═══════════════════════════════════════════════════════════════

var __activeElement = null;
if (typeof document !== 'undefined') {
    Object.defineProperty(document, 'activeElement', {
        get: function() { return __activeElement || document.body; },
        configurable: true
    });
}

// Focus must dispatch: focusin (bubbles:true) + focus (bubbles:false)
// Blur must dispatch: focusout (bubbles:true) + blur (bubbles:false)
if (typeof HTMLElement !== 'undefined' && HTMLElement.prototype) {
    HTMLElement.prototype.focus = function() {
        var prev = __activeElement;
        if (prev === this) return; // already focused
        // Blur previous — fire change if dirty (F2e)
        if (prev && prev !== document.body) {
            if (prev.__neo_dirty) {
                prev.dispatchEvent(new Event('change', {bubbles: true}));
                prev.__neo_dirty = false;
            }
            prev.dispatchEvent(new FocusEvent('focusout', {bubbles: true, relatedTarget: this}));
            prev.dispatchEvent(new FocusEvent('blur', {bubbles: false, relatedTarget: this}));
        }
        // Update activeElement BETWEEN blur and focus (spec order)
        __activeElement = this;
        // Focus new
        this.dispatchEvent(new FocusEvent('focusin', {bubbles: true, relatedTarget: prev}));
        this.dispatchEvent(new FocusEvent('focus', {bubbles: false, relatedTarget: prev}));
    };

    HTMLElement.prototype.blur = function() {
        if (__activeElement !== this) return;
        // F2e: fire change if dirty
        if (this.__neo_dirty) {
            this.dispatchEvent(new Event('change', {bubbles: true}));
            this.__neo_dirty = false;
        }
        this.dispatchEvent(new FocusEvent('focusout', {bubbles: true, relatedTarget: null}));
        this.dispatchEvent(new FocusEvent('blur', {bubbles: false, relatedTarget: null}));
        __activeElement = document.body || null;
    };
}

// ═══════════════════════════════════════════════════════════════
// 5. OBSERVER STUBS — report visible/sized for lazy-loading SPAs
// ═══════════════════════════════════════════════════════════════

// IntersectionObserver — report everything as visible immediately.
// Tracks observed elements with a Set to prevent double-firing.
globalThis.IntersectionObserver = class IntersectionObserver {
    constructor(callback, options) {
        this._callback = callback;
        this._elements = new Set();
        this._options = options || {};
    }
    observe(el) {
        if (this._elements.has(el)) return; // already observed, don't double-fire
        this._elements.add(el);
        var self = this;
        var cb = this._callback;
        setTimeout(function() {
            if (!self._elements.has(el)) return; // unobserved before callback fired
            cb([{
                target: el, isIntersecting: true, intersectionRatio: 1,
                boundingClientRect: { top: 0, left: 0, width: 1024, height: 768, right: 1024, bottom: 768, x: 0, y: 0 },
                intersectionRect: { top: 0, left: 0, width: 1024, height: 768, right: 1024, bottom: 768, x: 0, y: 0 },
                rootBounds: null, time: Date.now()
            }], self);
        }, 0);
    }
    unobserve(el) { this._elements.delete(el); }
    disconnect() { this._elements.clear(); }
    takeRecords() { return []; }
};

// ResizeObserver — report desktop viewport
globalThis.ResizeObserver = class ResizeObserver {
    constructor(callback) { this._callback = callback; this._elements = []; }
    observe(el) {
        this._elements.push(el);
        var cb = this._callback;
        var self = this;
        setTimeout(function() {
            cb([{
                target: el,
                contentRect: { width: 1024, height: 768, top: 0, left: 0, right: 1024, bottom: 768, x: 0, y: 0 },
                borderBoxSize: [{ inlineSize: 1024, blockSize: 768 }],
                contentBoxSize: [{ inlineSize: 1024, blockSize: 768 }],
            }], self);
        }, 0);
    }
    unobserve(el) {
        this._elements = this._elements.filter(function(e) { return e !== el; });
    }
    disconnect() { this._elements = []; }
};

// MutationObserver — use happy-dom's if available, otherwise stub
if (!globalThis.MutationObserver) {
    globalThis.MutationObserver = class MutationObserver {
        constructor(callback) { this._callback = callback; }
        observe() {}
        disconnect() {}
        takeRecords() { return []; }
    };
}

// ═══════════════════════════════════════════════════════════════
// 6. CSS STUBS — desktop-aware matchMedia + getComputedStyle
// ═══════════════════════════════════════════════════════════════

// matchMedia — desktop viewport awareness
globalThis.matchMedia = function(query) {
    // Desktop: match min-width queries, reject mobile max-width queries
    var matches = true;
    if (query.indexOf('max-width: 767') !== -1 || query.indexOf('max-width:767') !== -1 ||
        query.indexOf('max-width: 480') !== -1 || query.indexOf('max-width:480') !== -1 ||
        query.indexOf('max-width: 640') !== -1 || query.indexOf('max-width:640') !== -1) {
        matches = false;
    }
    if (query.indexOf('prefers-color-scheme: dark') !== -1) {
        matches = false;
    }
    return {
        matches: matches,
        media: query,
        addEventListener: function() {},
        removeEventListener: function() {},
        addListener: function() {},
        removeListener: function() {},
        onchange: null,
        dispatchEvent: function() { return true; }
    };
};

// getComputedStyle — return sensible defaults
globalThis.getComputedStyle = function(el, pseudo) {
    return new Proxy({}, {
        get: function(_, prop) {
            if (prop === 'display') return 'block';
            if (prop === 'visibility') return 'visible';
            if (prop === 'opacity') return '1';
            if (prop === 'position') return 'static';
            if (prop === 'overflow') return 'visible';
            if (prop === 'width') return '1024px';
            if (prop === 'height') return '768px';
            if (prop === 'getPropertyValue') return function(p) { return ''; };
            if (prop === 'length') return 0;
            if (prop === 'item') return function(i) { return ''; };
            if (typeof prop === 'symbol') return undefined;
            return '';
        }
    });
};

// Visibility API + readyState (COMPAT — set at pipeline phases, not real browser load cycle)
if (typeof document !== 'undefined') {
    try {
        Object.defineProperty(document, 'hidden', { get: function() { return false; }, configurable: true });
        Object.defineProperty(document, 'visibilityState', { get: function() { return 'visible'; }, configurable: true });
        // readyState: 'complete' since we run after DOM parse + JS execution
        if (!document.readyState || document.readyState === 'loading') {
            Object.defineProperty(document, 'readyState', {
                value: 'complete', writable: true, configurable: true
            });
        }
    } catch(e) {}
}

// ═══════════════════════════════════════════════════════════════
// 7. SPA HYDRATION STUBS — APIs needed by React/Next.js
// ═══════════════════════════════════════════════════════════════

// Service Worker
if (typeof navigator !== 'undefined' && !navigator.serviceWorker) {
    navigator.serviceWorker = {
        ready: Promise.resolve({ active: null }),
        register: function() { return Promise.resolve({}); },
        getRegistrations: function() { return Promise.resolve([]); },
        addEventListener: function() {},
        removeEventListener: function() {},
        controller: null,
    };
}

// Crypto.subtle (needed by some apps for integrity checks)
if (typeof crypto !== 'undefined' && !crypto.subtle) {
    crypto.subtle = {
        digest: function() { return Promise.resolve(new ArrayBuffer(32)); },
        encrypt: function() { return Promise.resolve(new ArrayBuffer(0)); },
        decrypt: function() { return Promise.resolve(new ArrayBuffer(0)); },
        sign: function() { return Promise.resolve(new ArrayBuffer(0)); },
        verify: function() { return Promise.resolve(false); },
        importKey: function() { return Promise.resolve({}); },
        exportKey: function() { return Promise.resolve({}); },
        generateKey: function() { return Promise.resolve({}); },
        deriveBits: function() { return Promise.resolve(new ArrayBuffer(0)); },
        deriveKey: function() { return Promise.resolve({}); },
    };
}

// document.createRange (React uses this for text insertion)
if (typeof document !== 'undefined' && !document.createRange) {
    document.createRange = function() {
        return {
            setStart: function() {},
            setEnd: function() {},
            commonAncestorContainer: document.body,
            selectNodeContents: function() {},
            collapse: function() {},
            getBoundingClientRect: function() {
                return { top: 0, left: 0, right: 0, bottom: 0, width: 0, height: 0 };
            },
            getClientRects: function() { return []; },
            createContextualFragment: function(html) {
                var tmpl = document.createElement('template');
                tmpl.innerHTML = html;
                return tmpl.content || document.createDocumentFragment();
            },
        };
    };
}

// window.getSelection (React uses this)
if (!globalThis.getSelection) {
    globalThis.getSelection = function() {
        return {
            rangeCount: 0,
            getRangeAt: function() { return document.createRange ? document.createRange() : {}; },
            addRange: function() {},
            removeAllRanges: function() {},
            toString: function() { return ''; },
        };
    };
}

// queueMicrotask if missing
if (!globalThis.queueMicrotask) {
    globalThis.queueMicrotask = function(cb) { Promise.resolve().then(cb); };
}

// requestIdleCallback / cancelIdleCallback
if (!globalThis.requestIdleCallback) {
    globalThis.requestIdleCallback = function(cb) {
        return setTimeout(function() {
            cb({ didTimeout: false, timeRemaining: function() { return 50; } });
        }, 0);
    };
    globalThis.cancelIdleCallback = function(id) { clearTimeout(id); };
}

// requestAnimationFrame / cancelAnimationFrame (may already exist but ensure)
if (!globalThis.requestAnimationFrame) {
    globalThis.requestAnimationFrame = function(cb) { return setTimeout(function() { cb(Date.now()); }, 0); };
    globalThis.cancelAnimationFrame = function(id) { clearTimeout(id); };
}

// Performance API stubs
if (!globalThis.performance) {
    globalThis.performance = { now: function() { return Date.now(); }, mark: function() {}, measure: function() {}, getEntriesByName: function() { return []; }, getEntriesByType: function() { return []; } };
}

// ═══════════════════════════════════════════════════════════════
// 8. SCROLL STUBS — no-op, fake layout geometry
// ═══════════════════════════════════════════════════════════════

globalThis.scrollTo = function() {};
globalThis.scrollBy = function() {};
globalThis.scroll = function() {};
globalThis.scrollX = 0;
globalThis.scrollY = 0;
globalThis.pageXOffset = 0;
globalThis.pageYOffset = 0;
globalThis.innerWidth = 1440;
globalThis.innerHeight = 900;
globalThis.outerWidth = 1440;
globalThis.outerHeight = 900;

if (typeof Element !== 'undefined' && Element.prototype) {
    Element.prototype.scrollIntoView = Element.prototype.scrollIntoView || function() {};
    Element.prototype.getBoundingClientRect = Element.prototype.getBoundingClientRect || function() {
        return { top: 0, left: 0, right: 1024, bottom: 768, width: 1024, height: 768, x: 0, y: 0 };
    };
    Element.prototype.getClientRects = Element.prototype.getClientRects || function() {
        return [{ top: 0, left: 0, right: 1024, bottom: 768, width: 1024, height: 768, x: 0, y: 0 }];
    };
    // scroll properties
    if (!('scrollTop' in Element.prototype)) {
        Object.defineProperty(Element.prototype, 'scrollTop', {
            get: function() { return 0; }, set: function() {}, configurable: true
        });
    }
    if (!('scrollLeft' in Element.prototype)) {
        Object.defineProperty(Element.prototype, 'scrollLeft', {
            get: function() { return 0; }, set: function() {}, configurable: true
        });
    }
}

// ═══════════════════════════════════════════════════════════════
// 9. ENHANCED EVENT CONSTRUCTORS — React 18+ needs full event APIs
// ═══════════════════════════════════════════════════════════════

// InputEvent — React controlled inputs need inputType, isComposing, dataTransfer
(function() {
    var BaseEvent = globalThis.Event;
    globalThis.InputEvent = class InputEvent extends BaseEvent {
        constructor(type, init) {
            init = init || {};
            super(type, init);
            this.data = init.data || null;
            this.inputType = init.inputType || '';
            this.isComposing = init.isComposing || false;
            this.dataTransfer = init.dataTransfer || null;
        }
    };
})();

// PointerEvent — modern click/touch handlers (React, lit-html, etc.)
(function() {
    var BaseMouseEvent = globalThis.MouseEvent;
    globalThis.PointerEvent = class PointerEvent extends BaseMouseEvent {
        constructor(type, init) {
            init = init || {};
            super(type, init);
            this.pointerId = init.pointerId || 0;
            this.width = init.width || 1;
            this.height = init.height || 1;
            this.pressure = init.pressure || 0;
            this.tiltX = init.tiltX || 0;
            this.tiltY = init.tiltY || 0;
            this.pointerType = init.pointerType || 'mouse';
            this.isPrimary = init.isPrimary !== undefined ? init.isPrimary : true;
        }
    };
})();

// FocusEvent — needs relatedTarget for focus/blur event pairs
(function() {
    var BaseEvent = globalThis.Event;
    globalThis.FocusEvent = class FocusEvent extends BaseEvent {
        constructor(type, init) {
            init = init || {};
            super(type, init);
            this.relatedTarget = init.relatedTarget || null;
        }
    };
})();

// ═══════════════════════════════════════════════════════════════
// 10. UNHANDLED REJECTION LOGGING — surface async errors
// ═══════════════════════════════════════════════════════════════

globalThis.addEventListener('unhandledrejection', function(event) {
    if (typeof _shimOps !== 'undefined' && _shimOps.op_console_log) {
        try {
            var reason = event && event.reason;
            var msg = (reason && reason.message) || (reason && String(reason)) || 'unknown';
            _shimOps.op_console_log('[REJECTION] ' + msg);
        } catch(e) {}
    }
});

// ═══════════════════════════════════════════════════════════════
// 11. FORM CONSTRAINT VALIDATION
// ═══════════════════════════════════════════════════════════════

if (typeof HTMLInputElement !== 'undefined' && HTMLInputElement.prototype && !HTMLInputElement.prototype.checkValidity) {
    var validatable = [HTMLInputElement, HTMLTextAreaElement, HTMLSelectElement];
    validatable.forEach(function(Ctor) {
        if (!Ctor || !Ctor.prototype) return;

        Ctor.prototype.checkValidity = function() {
            if (this._customValidity) return false;
            if (this.required && !this.value) return false;
            if (this.pattern) {
                try { if (!new RegExp('^(?:' + this.pattern + ')$').test(this.value)) return false; }
                catch(e) {}
            }
            if (this.minLength > 0 && this.value.length < this.minLength) return false;
            if (this.maxLength > 0 && this.value.length > this.maxLength) return false;
            if (this.type === 'email' && this.value && !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(this.value)) return false;
            if (this.type === 'url' && this.value) {
                try { new URL(this.value); } catch(e) { return false; }
            }
            if (this.type === 'number' && this.value) {
                var n = Number(this.value);
                if (isNaN(n)) return false;
                if (this.min !== '' && n < Number(this.min)) return false;
                if (this.max !== '' && n > Number(this.max)) return false;
            }
            return true;
        };

        Ctor.prototype.reportValidity = function() {
            var valid = this.checkValidity();
            if (!valid) {
                this.dispatchEvent(new Event('invalid', {bubbles: false, cancelable: true}));
            }
            return valid;
        };

        Ctor.prototype.setCustomValidity = function(msg) {
            this._customValidity = msg || '';
        };

        Object.defineProperty(Ctor.prototype, 'validationMessage', {
            get: function() {
                if (this._customValidity) return this._customValidity;
                if (this.required && !this.value) return 'Please fill out this field.';
                if (this.type === 'email' && this.value && !/^[^\s@]+@[^\s@]+/.test(this.value))
                    return 'Please include an email address.';
                return '';
            },
            configurable: true
        });

        Object.defineProperty(Ctor.prototype, 'validity', {
            get: function() {
                var val = this.value || '';
                return {
                    valueMissing: this.required && !val,
                    typeMismatch: (this.type === 'email' && val && !/^[^\s@]+@[^\s@]+/.test(val)),
                    patternMismatch: this.pattern ? !new RegExp('^(?:' + this.pattern + ')$').test(val) : false,
                    tooShort: this.minLength > 0 && val.length < this.minLength,
                    tooLong: this.maxLength > 0 && val.length > this.maxLength,
                    rangeUnderflow: false,
                    rangeOverflow: false,
                    stepMismatch: false,
                    badInput: false,
                    customError: !!this._customValidity,
                    valid: this.checkValidity(),
                };
            },
            configurable: true
        });

        Object.defineProperty(Ctor.prototype, 'willValidate', {
            get: function() { return !this.disabled && this.type !== 'hidden'; },
            configurable: true
        });
    });
}

// Form-level validation
if (typeof HTMLFormElement !== 'undefined' && HTMLFormElement.prototype && !HTMLFormElement.prototype.checkValidity) {
    HTMLFormElement.prototype.checkValidity = function() {
        var inputs = this.querySelectorAll('input,textarea,select');
        var valid = true;
        inputs.forEach(function(el) {
            if (el.willValidate && !el.checkValidity()) valid = false;
        });
        return valid;
    };

    HTMLFormElement.prototype.reportValidity = function() {
        var inputs = this.querySelectorAll('input,textarea,select');
        var valid = true;
        inputs.forEach(function(el) {
            if (el.willValidate && !el.reportValidity()) valid = false;
        });
        return valid;
    };

    HTMLFormElement.prototype.requestSubmit = function(submitter) {
        if (!this.checkValidity()) {
            this.reportValidity();
            return;
        }
        var evt = new Event('submit', {bubbles: false, cancelable: true});
        evt.submitter = submitter || null;
        if (this.dispatchEvent(evt)) {
            this.submit();
        }
    };
}

// ═══════════════════════════════════════════════════════════════
// 12. SELECTION / CARET APIs
// ═══════════════════════════════════════════════════════════════

if (typeof HTMLInputElement !== 'undefined' && HTMLInputElement.prototype && !HTMLInputElement.prototype.setSelectionRange) {
    [HTMLInputElement, HTMLTextAreaElement].forEach(function(Ctor) {
        if (!Ctor || !Ctor.prototype) return;

        // Selection properties (in-memory tracking)
        if (!('selectionStart' in Ctor.prototype)) {
            Object.defineProperty(Ctor.prototype, 'selectionStart', {
                get: function() { return this._selStart || 0; },
                set: function(v) { this._selStart = v; },
                configurable: true
            });
            Object.defineProperty(Ctor.prototype, 'selectionEnd', {
                get: function() { return this._selEnd || (this.value || '').length; },
                set: function(v) { this._selEnd = v; },
                configurable: true
            });
            Object.defineProperty(Ctor.prototype, 'selectionDirection', {
                get: function() { return this._selDir || 'forward'; },
                set: function(v) { this._selDir = v; },
                configurable: true
            });
        }

        Ctor.prototype.setSelectionRange = function(start, end, direction) {
            this._selStart = start;
            this._selEnd = end;
            this._selDir = direction || 'none';
        };

        Ctor.prototype.select = function() {
            this._selStart = 0;
            this._selEnd = (this.value || '').length;
            this.dispatchEvent(new Event('select'));
        };
    });
}

// document.execCommand (limited — insertText only)
if (typeof document !== 'undefined') {
    // Always override — happy-dom's execCommand is a no-op stub.
    document.execCommand = function(cmd, showUI, value) {
        if (cmd === 'insertText' && document.activeElement) {
            var el = document.activeElement;
            if (el.value !== undefined) {
                var start = el.selectionStart || 0;
                var end = el.selectionEnd || start;
                var before = el.value.substring(0, start);
                var after = el.value.substring(end);
                el.value = before + value + after;
                el.selectionStart = el.selectionEnd = start + value.length;
                el.dispatchEvent(new InputEvent('input', {data: value, inputType: 'insertText', bubbles: true}));
                return true;
            }
            if (el.contentEditable === 'true' || el.isContentEditable) {
                // For contenteditable (ProseMirror, etc):
                // 1. Insert text into the first <p> or create one
                var target = el.querySelector('p') || el;
                if (target.tagName === 'P' && target.getAttribute('data-placeholder')) {
                    target.removeAttribute('data-placeholder');
                }
                // Append text node
                target.textContent = (target.textContent || '') + value;
                // 2. Dispatch beforeinput + input (ProseMirror listens for both)
                el.dispatchEvent(new InputEvent('beforeinput', {
                    data: value, inputType: 'insertText', bubbles: true, cancelable: true
                }));
                el.dispatchEvent(new InputEvent('input', {
                    data: value, inputType: 'insertText', bubbles: true
                }));
                return true;
            }
        }
        return false;
    };
}

// ═══════════════════════════════════════════════════════════════
// 13. DOMPARSER — enhanced with content-type support
// ═══════════════════════════════════════════════════════════════

// Override bootstrap's basic DOMParser with content-type-aware version
globalThis.DOMParser = class DOMParser {
    parseFromString(str, type) {
        if (type === 'text/html' || type === 'text/xml' || type === 'application/xml' || type === 'application/xhtml+xml') {
            return __linkedom_parseHTML(str).document;
        }
        throw new Error('DOMParser: unsupported type: ' + type);
    }
};

// ═══════════════════════════════════════════════════════════════
// 14. DOM IDL SHIMS — input.labels, label.control, select.selectedOptions
// ═══════════════════════════════════════════════════════════════

// input.labels — returns labels associated with an input element
if (typeof HTMLInputElement !== 'undefined' && HTMLInputElement.prototype && !('labels' in HTMLInputElement.prototype)) {
    Object.defineProperty(HTMLInputElement.prototype, 'labels', {
        get: function() {
            var id = this.id;
            if (!id) {
                // Check ancestor label only
                var ancestor = this.closest ? this.closest('label') : null;
                return ancestor ? [ancestor] : [];
            }
            var byFor = document.querySelectorAll('label[for="' + id + '"]');
            var ancestor = this.closest ? this.closest('label') : null;
            var result = Array.from(byFor);
            if (ancestor && result.indexOf(ancestor) < 0) result.push(ancestor);
            return result;
        },
        configurable: true
    });
}

// label.control — returns the form control associated with a label
if (typeof HTMLLabelElement !== 'undefined' && HTMLLabelElement.prototype && !('control' in HTMLLabelElement.prototype)) {
    Object.defineProperty(HTMLLabelElement.prototype, 'control', {
        get: function() {
            var forId = this.getAttribute('for') || this.htmlFor;
            if (forId) return document.getElementById(forId);
            return this.querySelector('input,select,textarea,button');
        },
        configurable: true
    });
}

// select.selectedOptions — returns currently selected option elements
if (typeof HTMLSelectElement !== 'undefined' && HTMLSelectElement.prototype && !('selectedOptions' in HTMLSelectElement.prototype)) {
    Object.defineProperty(HTMLSelectElement.prototype, 'selectedOptions', {
        get: function() {
            return Array.from(this.options || this.querySelectorAll('option')).filter(function(o) { return o.__idl_selected || o.selected; });
        },
        configurable: true
    });
}

// ═══════════════════════════════════════════════════════════════
// 15. IDL ATTRIBUTE SYNCHRONIZATION — React controlled inputs
// ═══════════════════════════════════════════════════════════════
// Real browsers separate content attributes (getAttribute/setAttribute) from
// IDL properties (el.value, el.checked). happy-dom conflates them. React reads
// IDL properties to determine current state and uses setAttribute for initial
// render — if they aren't independent, controlled inputs break.

// 15a. input.value / textarea.value — independent of the value attribute
(function() {
    [typeof HTMLInputElement !== 'undefined' ? HTMLInputElement : null,
     typeof HTMLTextAreaElement !== 'undefined' ? HTMLTextAreaElement : null].forEach(function(Ctor) {
        if (!Ctor || !Ctor.prototype) return;
        Object.defineProperty(Ctor.prototype, 'value', {
            get: function() {
                if ('__idl_value' in this) return this.__idl_value;
                return this.getAttribute('value') || '';
            },
            set: function(v) {
                this.__idl_value = String(v);
                // Do NOT call setAttribute — the content attribute stays independent
            },
            configurable: true,
            enumerable: true,
        });
    });
})();

// 15b. input.checked — independent of the checked attribute
(function() {
    if (typeof HTMLInputElement === 'undefined' || !HTMLInputElement.prototype) return;

    Object.defineProperty(HTMLInputElement.prototype, 'checked', {
        get: function() {
            if ('__idl_checked' in this) return this.__idl_checked;
            return this.hasAttribute('checked');
        },
        set: function(v) {
            this.__idl_checked = !!v;
        },
        configurable: true,
        enumerable: true,
    });

    // defaultChecked maps to the checked CONTENT ATTRIBUTE
    Object.defineProperty(HTMLInputElement.prototype, 'defaultChecked', {
        get: function() { return this.hasAttribute('checked'); },
        set: function(v) { if (v) this.setAttribute('checked', ''); else this.removeAttribute('checked'); },
        configurable: true,
    });

    // defaultValue maps to the value CONTENT ATTRIBUTE
    Object.defineProperty(HTMLInputElement.prototype, 'defaultValue', {
        get: function() { return this.getAttribute('value') || ''; },
        set: function(v) { this.setAttribute('value', String(v)); },
        configurable: true,
    });
})();

// 15c. select.value / select.selectedIndex / option.selected
(function() {
    if (typeof HTMLSelectElement === 'undefined' || !HTMLSelectElement.prototype) return;

    Object.defineProperty(HTMLSelectElement.prototype, 'value', {
        get: function() {
            var opts = this.querySelectorAll('option');
            for (var i = 0; i < opts.length; i++) {
                if (opts[i].__idl_selected || opts[i].hasAttribute('selected')) return opts[i].value || opts[i].textContent;
            }
            return opts.length ? (opts[0].value || opts[0].textContent) : '';
        },
        set: function(v) {
            var opts = this.querySelectorAll('option');
            for (var i = 0; i < opts.length; i++) {
                opts[i].__idl_selected = (opts[i].value === v || opts[i].textContent.trim() === v);
            }
        },
        configurable: true,
        enumerable: true,
    });

    Object.defineProperty(HTMLSelectElement.prototype, 'selectedIndex', {
        get: function() {
            var opts = this.querySelectorAll('option');
            for (var i = 0; i < opts.length; i++) {
                if (opts[i].__idl_selected || opts[i].hasAttribute('selected')) return i;
            }
            return 0;
        },
        set: function(idx) {
            var opts = this.querySelectorAll('option');
            for (var i = 0; i < opts.length; i++) {
                opts[i].__idl_selected = (i === idx);
            }
        },
        configurable: true,
        enumerable: true,
    });

    // form.elements — returns form controls
    if (typeof HTMLFormElement !== 'undefined' && HTMLFormElement.prototype) {
        Object.defineProperty(HTMLFormElement.prototype, 'elements', {
            get: function() {
                return this.querySelectorAll('input,select,textarea,button');
            },
            configurable: true,
        });
    }
})();

// 15d. Boolean IDL properties — disabled, required, readOnly, multiple, autofocus
(function() {
    ['disabled', 'required', 'readOnly', 'multiple', 'autofocus'].forEach(function(prop) {
        var attr = prop.toLowerCase();
        [typeof HTMLInputElement !== 'undefined' ? HTMLInputElement : null,
         typeof HTMLSelectElement !== 'undefined' ? HTMLSelectElement : null,
         typeof HTMLTextAreaElement !== 'undefined' ? HTMLTextAreaElement : null,
         typeof HTMLButtonElement !== 'undefined' ? HTMLButtonElement : null].forEach(function(Ctor) {
            if (!Ctor || !Ctor.prototype) return;
            // Only define if not already a proper boolean IDL property
            var existing = Object.getOwnPropertyDescriptor(Ctor.prototype, prop);
            if (existing && existing.get) return; // already has getter
            Object.defineProperty(Ctor.prototype, prop, {
                get: function() { return this.hasAttribute(attr); },
                set: function(v) { if (v) this.setAttribute(attr, ''); else this.removeAttribute(attr); },
                configurable: true,
                enumerable: true,
            });
        });
    });
})();

// 15e. input.type — defaults to 'text' per spec (happy-dom may return null instead)
(function() {
    if (typeof HTMLInputElement === 'undefined' || !HTMLInputElement.prototype) return;
    // Always override — happy-dom's getter may return null when no type attribute is set
    Object.defineProperty(HTMLInputElement.prototype, 'type', {
        get: function() { return this.getAttribute('type') || 'text'; },
        set: function(v) { this.setAttribute('type', v); },
        configurable: true,
        enumerable: true,
    });
})();

// ─── Legacy IE API stubs (React 18 production uses attachEvent) ───
if (typeof HTMLElement !== 'undefined' && HTMLElement.prototype && !HTMLElement.prototype.attachEvent) {
    HTMLElement.prototype.attachEvent = function(evt, fn) {
        this.addEventListener(evt.replace(/^on/, ''), fn);
    };
    HTMLElement.prototype.detachEvent = function(evt, fn) {
        this.removeEventListener(evt.replace(/^on/, ''), fn);
    };
}
if (typeof Document !== 'undefined' && Document.prototype && !Document.prototype.attachEvent) {
    Document.prototype.attachEvent = function(evt, fn) {
        this.addEventListener(evt.replace(/^on/, ''), fn);
    };
}

// ─── Fix read-only toString on prototypes (happy-dom makes some non-writable) ───
// Real browsers have toString as writable on all standard prototypes.
// Some bundled JS (Vite/ChatGPT) assigns custom toString to objects.
try {
    [Object.prototype, Function.prototype, Error.prototype, RegExp.prototype, Date.prototype, Array.prototype, Number.prototype, String.prototype, Boolean.prototype].forEach(function(proto) {
        if (!proto) return;
        var desc = Object.getOwnPropertyDescriptor(proto, 'toString');
        if (desc && !desc.writable) {
            Object.defineProperty(proto, 'toString', {
                value: desc.value,
                writable: true,
                configurable: true,
                enumerable: false
            });
        }
    });
    // Also fix valueOf
    [Object.prototype, Number.prototype, String.prototype, Boolean.prototype, Date.prototype].forEach(function(proto) {
        if (!proto) return;
        var desc = Object.getOwnPropertyDescriptor(proto, 'valueOf');
        if (desc && !desc.writable) {
            Object.defineProperty(proto, 'valueOf', {
                value: desc.value,
                writable: true,
                configurable: true,
                enumerable: false
            });
        }
    });
} catch(e) {}

// ─── Request API (used by fetch-dependent apps) ───
if (!globalThis.Request) {
    globalThis.Request = class Request {
        constructor(input, init) {
            if (typeof input === 'string') {
                this.url = input;
            } else if (input && input.url) {
                this.url = input.url;
            } else {
                this.url = String(input);
            }
            init = init || {};
            this.method = (init.method || 'GET').toUpperCase();
            this.headers = new Headers(init.headers || {});
            this.body = init.body || null;
            this.mode = init.mode || 'cors';
            this.credentials = init.credentials || 'same-origin';
            this.cache = init.cache || 'default';
            this.redirect = init.redirect || 'follow';
            this.referrer = init.referrer || '';
            this.signal = init.signal || null;
        }
        clone() { return new Request(this.url, {method:this.method,headers:this.headers,body:this.body}); }
    };
}

// ═══════════════════════════════════════════════════════════════
// 15. EVENT SYSTEM FIXES — stopImmediatePropagation, composedPath, SubmitEvent, isConnected
// ═══════════════════════════════════════════════════════════════

// 15a. stopImmediatePropagation — happy-dom doesn't implement the flag check
(function() {
    if (!Event.prototype) return;
    // Only patch once
    if (Event.prototype.stopImmediatePropagation && Event.prototype.stopImmediatePropagation.__real) return;

    var origSIP = Event.prototype.stopImmediatePropagation;
    Event.prototype.stopImmediatePropagation = function() {
        this._immediateStopped = true;
        if (origSIP) origSIP.call(this);
    };
    Event.prototype.stopImmediatePropagation.__real = true;

    // Wrap addEventListener so listeners respect _immediateStopped
    var origAddEventListener = EventTarget.prototype.addEventListener;
    EventTarget.prototype.addEventListener = function(type, listener, options) {
        if (typeof listener !== 'function') {
            return origAddEventListener.call(this, type, listener, options);
        }
        var wrappedListener = function(event) {
            if (event._immediateStopped) return;
            return listener.call(this, event);
        };
        wrappedListener._original = listener;
        // Store mapping for removeEventListener
        if (!this.__neo_listener_map) this.__neo_listener_map = [];
        this.__neo_listener_map.push({ type: type, original: listener, wrapped: wrappedListener, options: options });
        return origAddEventListener.call(this, type, wrappedListener, options);
    };

    var origRemoveEventListener = EventTarget.prototype.removeEventListener;
    EventTarget.prototype.removeEventListener = function(type, listener, options) {
        // Find the wrapped version
        if (this.__neo_listener_map) {
            for (var i = 0; i < this.__neo_listener_map.length; i++) {
                var entry = this.__neo_listener_map[i];
                if (entry.type === type && entry.original === listener) {
                    this.__neo_listener_map.splice(i, 1);
                    return origRemoveEventListener.call(this, type, entry.wrapped, options);
                }
            }
        }
        return origRemoveEventListener.call(this, type, listener, options);
    };
})();

// 15b. Event.composedPath() — build path from target to window
if (Event.prototype && !Event.prototype.composedPath) {
    Event.prototype.composedPath = function() {
        var path = [];
        var node = this.target;
        while (node) {
            path.push(node);
            node = node.parentNode;
        }
        if (path.length > 0) {
            if (typeof document !== 'undefined') path.push(document);
            if (typeof window !== 'undefined') path.push(window);
        }
        return path;
    };
}

// 15c. SubmitEvent constructor
if (!globalThis.SubmitEvent) {
    globalThis.SubmitEvent = class SubmitEvent extends Event {
        constructor(type, init) {
            super(type, init);
            this.submitter = (init && init.submitter) || null;
        }
    };
}

// 15d. Node.isConnected property
(function() {
    if (typeof Node === 'undefined' || !Node.prototype) return;
    if ('isConnected' in Node.prototype) return;
    Object.defineProperty(Node.prototype, 'isConnected', {
        get: function() {
            var node = this;
            while (node) {
                if (node === document) return true;
                node = node.parentNode;
            }
            return false;
        },
        configurable: true,
    });
})();

// 15e. Element.closest and Element.matches verification + polyfill
(function() {
    if (typeof Element === 'undefined' || !Element.prototype) return;

    if (!Element.prototype.matches) {
        Element.prototype.matches =
            Element.prototype.msMatchesSelector ||
            Element.prototype.webkitMatchesSelector ||
            function(sel) {
                var matches = (this.ownerDocument || document).querySelectorAll(sel);
                for (var i = 0; i < matches.length; i++) {
                    if (matches[i] === this) return true;
                }
                return false;
            };
    }

    if (!Element.prototype.closest) {
        Element.prototype.closest = function(sel) {
            var el = this;
            while (el && el.nodeType === 1) {
                if (el.matches(sel)) return el;
                el = el.parentNode;
            }
            return null;
        };
    }
})();

// ═══════════════════════════════════════════════════════════════
// REACT EVENT PIPELINE — dispatchEvent monkeypatch
// ═══════════════════════════════════════════════════════════════
//
// Problem: React 16/17/18 installs event listeners on document or root
// container via listenToAllSupportedEvents(). In happy-dom this installation
// silently fails — React's internal event plugin system never fires.
//
// Fix: monkeypatch EventTarget.prototype.dispatchEvent so that when ANY
// bubbling event is dispatched, we SYNCHRONOUSLY walk the DOM from target
// up to root and invoke React handlers found in __reactEventHandlers (v16)
// or __reactProps (v17/18). This runs DURING the dispatch, before any
// event loop pump can reset controlled component values.
//
// This is the ONLY reliable way to connect React's handler system without
// modifying React internals or depending on listenToAllSupportedEvents.
(function installReactEventPipeline() {
    // Native event type → React handler prop name(s)
    var EVENT_MAP = {
        click: ['onClick'],
        dblclick: ['onDoubleClick'],
        mousedown: ['onMouseDown'],
        mouseup: ['onMouseUp'],
        mousemove: ['onMouseMove'],
        mouseenter: ['onMouseEnter'],
        mouseleave: ['onMouseLeave'],
        keydown: ['onKeyDown'],
        keyup: ['onKeyUp'],
        keypress: ['onKeyPress'],
        input: ['onInput', 'onChange'],  // React fires onChange on input events
        change: ['onChange'],
        focus: ['onFocus'],
        blur: ['onBlur'],
        focusin: ['onFocus'],
        focusout: ['onBlur'],
        submit: ['onSubmit'],
        reset: ['onReset'],
        scroll: ['onScroll'],
        wheel: ['onWheel'],
        touchstart: ['onTouchStart'],
        touchend: ['onTouchEnd'],
        touchmove: ['onTouchMove'],
        pointerdown: ['onPointerDown'],
        pointerup: ['onPointerUp'],
        compositionstart: ['onCompositionStart'],
        compositionend: ['onCompositionEnd'],
        compositionupdate: ['onCompositionUpdate'],
    };

    // Find React handler keys on an element (cached per suffix)
    var _handlerKeySuffix = null;
    function findHandlerKey(el) {
        // Fast path: cached suffix
        if (_handlerKeySuffix) {
            var k = '__reactEventHandlers' + _handlerKeySuffix;
            if (el[k]) return k;
            k = '__reactProps' + _handlerKeySuffix;
            if (el[k]) return k;
        }
        // Slow path: scan keys
        var keys = Object.keys(el);
        for (var i = 0; i < keys.length; i++) {
            if (keys[i].startsWith('__reactEventHandlers') || keys[i].startsWith('__reactProps')) {
                // Cache the suffix for fast future lookups
                _handlerKeySuffix = keys[i].replace(/^__react(?:EventHandlers|Props)/, '');
                return keys[i];
            }
        }
        return null;
    }

    // Build synthetic event that React handlers expect
    function makeSynthetic(nativeEvent, currentTarget) {
        return {
            target: nativeEvent.target,
            currentTarget: currentTarget,
            type: nativeEvent.type,
            bubbles: nativeEvent.bubbles,
            cancelable: nativeEvent.cancelable,
            defaultPrevented: nativeEvent.defaultPrevented,
            timeStamp: nativeEvent.timeStamp || Date.now(),
            nativeEvent: nativeEvent,
            preventDefault: function() { nativeEvent.preventDefault(); this.defaultPrevented = true; },
            stopPropagation: function() { nativeEvent.stopPropagation(); this._stopped = true; },
            stopImmediatePropagation: function() { nativeEvent.stopImmediatePropagation(); this._stopped = true; },
            persist: function() {},
            isPersistent: function() { return true; },
            isDefaultPrevented: function() { return this.defaultPrevented; },
            isPropagationStopped: function() { return !!this._stopped; },
            // Copy common properties
            key: nativeEvent.key,
            code: nativeEvent.code,
            charCode: nativeEvent.charCode,
            keyCode: nativeEvent.keyCode,
            which: nativeEvent.which,
            data: nativeEvent.data,
            inputType: nativeEvent.inputType,
            button: nativeEvent.button,
            buttons: nativeEvent.buttons,
            clientX: nativeEvent.clientX,
            clientY: nativeEvent.clientY,
            screenX: nativeEvent.screenX,
            screenY: nativeEvent.screenY,
            altKey: nativeEvent.altKey,
            ctrlKey: nativeEvent.ctrlKey,
            metaKey: nativeEvent.metaKey,
            shiftKey: nativeEvent.shiftKey,
            detail: nativeEvent.detail,
        };
    }

    // The core: monkeypatch dispatchEvent
    var _origDispatchEvent = EventTarget.prototype.dispatchEvent;
    EventTarget.prototype.dispatchEvent = function(event) {
        // Interaction trace — only high-signal events
        if (['click','submit','input','change'].includes(event.type)) {
            __neo_traceStep('event', {type: event.type, target: (event.target?.tagName || '?') + '#' + (event.target?.id || '')});
        }

        // 1. Call original dispatchEvent (happy-dom's native bubbling)
        var result = _origDispatchEvent.call(this, event);

        // 2. If this is a bubbling event and has React handler mappings, walk the tree
        var reactHandlerNames = EVENT_MAP[event.type];
        if (!reactHandlerNames || !event.bubbles) return result;

        // 3. Walk from target up through DOM, invoking React handlers
        var el = event.target || this;
        var stopped = false;
        while (el && !stopped) {
            var hk = findHandlerKey(el);
            if (hk) {
                var handlers = el[hk];
                if (handlers) {
                    var synth = makeSynthetic(event, el);
                    for (var i = 0; i < reactHandlerNames.length && !stopped; i++) {
                        var handler = handlers[reactHandlerNames[i]];
                        if (typeof handler === 'function') {
                            try {
                                handler(synth);
                            } catch (e) {
                                console.error('[react-event] ' + reactHandlerNames[i] + ' on <' +
                                    (el.tagName||'?').toLowerCase() + '>: ' + e.message);
                            }
                            if (synth._stopped) stopped = true;
                        }
                    }
                }
            }
            // Walk up (but stop at document — don't go to window)
            if (el === document || el === document.documentElement) break;
            el = el.parentNode || el.parentElement;
        }

        return result;
    };
})();

// ═══════════════════════════════════════════════════════════════
// LAYOUT STUBS — plausible non-zero values without layout engine
// ═══════════════════════════════════════════════════════════════
//
// Many SPAs use getBoundingClientRect, offsetWidth, scrollHeight etc.
// for responsive logic, virtualized lists, lazy loading, visibility checks.
// Without values, this logic fails silently. We provide heuristic sizing.

(function installLayoutStubs() {
    const VP_W = 1920, VP_H = 1080;
    const BLOCK_TAGS = new Set([
        'div','p','section','article','main','header','footer','nav',
        'form','ul','ol','li','h1','h2','h3','h4','h5','h6',
        'table','tr','tbody','thead','tfoot','body','html',
        'details','summary','dialog','aside','figure','figcaption'
    ]);

    // getBoundingClientRect — heuristic based on tag and content
    const _origGBCR = Element.prototype.getBoundingClientRect;
    Element.prototype.getBoundingClientRect = function() {
        const tag = (this.tagName || '').toLowerCase();
        const isBlock = BLOCK_TAGS.has(tag);
        const textLen = (this.textContent || '').length;
        const childCount = this.children?.length || 0;

        let w, h;
        if (tag === 'body' || tag === 'html') {
            w = VP_W; h = VP_H;
        } else if (isBlock) {
            w = VP_W;
            h = Math.max(20, Math.min(childCount * 30 + textLen * 0.3, 2000));
        } else {
            w = Math.min(Math.max(textLen * 8, 20), VP_W);
            h = 20;
        }

        return {
            top: 0, left: 0, right: w, bottom: h,
            width: w, height: h, x: 0, y: 0,
            toJSON() { return { top:0, left:0, right:w, bottom:h, width:w, height:h, x:0, y:0 }; }
        };
    };

    // offset* properties
    function defineLayoutProp(proto, prop, fallback) {
        // Override even if happy-dom already defines a getter (it returns 0)
        Object.defineProperty(proto, prop, {
            get() {
                const rect = this.getBoundingClientRect();
                return fallback(rect);
            },
            configurable: true,
        });
    }

    defineLayoutProp(HTMLElement.prototype, 'offsetWidth', r => r.width);
    defineLayoutProp(HTMLElement.prototype, 'offsetHeight', r => r.height);
    defineLayoutProp(HTMLElement.prototype, 'offsetTop', r => r.top);
    defineLayoutProp(HTMLElement.prototype, 'offsetLeft', r => r.left);
    defineLayoutProp(HTMLElement.prototype, 'clientWidth', r => r.width);
    defineLayoutProp(HTMLElement.prototype, 'clientHeight', r => r.height);
    defineLayoutProp(HTMLElement.prototype, 'scrollWidth', r => r.width);
    defineLayoutProp(HTMLElement.prototype, 'scrollHeight', r => Math.max(r.height, r.width));
    defineLayoutProp(HTMLElement.prototype, 'scrollTop', () => 0);
    defineLayoutProp(HTMLElement.prototype, 'scrollLeft', () => 0);

    // window.innerWidth/Height
    if (!globalThis.innerWidth) {
        Object.defineProperty(globalThis, 'innerWidth', { value: VP_W, writable: true });
        Object.defineProperty(globalThis, 'innerHeight', { value: VP_H, writable: true });
        Object.defineProperty(globalThis, 'outerWidth', { value: VP_W, writable: true });
        Object.defineProperty(globalThis, 'outerHeight', { value: VP_H, writable: true });
    }

    // screen
    globalThis.screen = globalThis.screen || {
        width: VP_W, height: VP_H,
        availWidth: VP_W, availHeight: VP_H,
        colorDepth: 24, pixelDepth: 24,
        orientation: { type: 'landscape-primary', angle: 0 },
    };

    // window.devicePixelRatio
    if (!globalThis.devicePixelRatio) globalThis.devicePixelRatio = 2;

    // scrollTo / scrollBy / scroll — no-ops
    if (!globalThis.scrollTo) globalThis.scrollTo = function() {};
    if (!globalThis.scrollBy) globalThis.scrollBy = function() {};
    if (!globalThis.scroll) globalThis.scroll = function() {};
    if (!Element.prototype.scrollIntoView) Element.prototype.scrollIntoView = function() {};
})();
