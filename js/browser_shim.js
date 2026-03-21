// NeoRender V2 Browser Shim — intercepts browser behaviors that linkedom can't handle.
// Loaded AFTER bootstrap.js. Overrides basic stubs with navigation-aware versions.
// Hooks into Rust ops: op_navigation_request, op_cookie_get, op_cookie_set.

// Use __neo_ops saved by bootstrap.js (Deno is deleted for sandbox security).
const _shimOps = globalThis.__neo_ops;

// ═══════════════════════════════════════════════════════════════
// 1. NAVIGATION INTERCEPTION — capture form.submit(), location changes
// ═══════════════════════════════════════════════════════════════

// Form submit helper — extracts form data and sends navigation request.
function __neoFormSubmit() {
    var formData = {};
    var inputs = this.querySelectorAll('input, select, textarea');
    inputs.forEach(function(el) {
        if (el.name) {
            if (el.type === 'checkbox' || el.type === 'radio') {
                if (el.checked) formData[el.name] = el.value || 'on';
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

// Intercept form.submit() — linkedom's form elements may have a different
// prototype chain than globalThis.HTMLFormElement, so we patch both the
// exported prototype AND the actual prototype from a real form element.
if (typeof HTMLFormElement !== 'undefined' && HTMLFormElement.prototype) {
    HTMLFormElement.prototype.submit = __neoFormSubmit;
}
// Also patch the actual prototype chain used by linkedom's form elements.
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

// Helper: resolve a URL (absolute or relative) and update __neo_location fields.
function __neoUpdateLocation(url) {
    var loc = globalThis.__neo_location;
    if (url.indexOf('://') !== -1) {
        // Absolute URL — parse directly
        var m = url.match(/^(https?:)\/\/([^/:]+)(:\d+)?(\/[^?#]*)?(\?[^#]*)?(#.*)?$/);
        if (m) {
            loc.protocol = m[1];
            loc.hostname = m[2];
            loc.port = (m[3] || '').replace(':', '');
            loc.host = loc.hostname + (loc.port ? ':' + loc.port : '');
            loc.pathname = m[4] || '/';
            loc.search = m[5] || '';
            loc.hash = m[6] || '';
            loc.origin = loc.protocol + '//' + loc.host;
            loc.href = loc.origin + loc.pathname + loc.search + loc.hash;
        }
    } else if (url.charAt(0) === '/') {
        // Root-relative: /path
        loc.pathname = url.split('?')[0].split('#')[0];
        loc.search = url.indexOf('?') !== -1 ? '?' + url.split('?')[1].split('#')[0] : '';
        loc.hash = url.indexOf('#') !== -1 ? '#' + url.split('#')[1] : '';
        loc.href = loc.origin + loc.pathname + loc.search + loc.hash;
    } else if (url.charAt(0) === '#') {
        // Hash only
        loc.hash = url;
        loc.href = loc.origin + loc.pathname + loc.search + loc.hash;
    } else if (url.charAt(0) === '?') {
        // Search only
        loc.search = url.split('#')[0];
        loc.hash = url.indexOf('#') !== -1 ? '#' + url.split('#')[1] : '';
        loc.href = loc.origin + loc.pathname + loc.search + loc.hash;
    } else {
        // Relative path
        var base = loc.pathname.replace(/\/[^/]*$/, '/');
        loc.pathname = base + url.split('?')[0].split('#')[0];
        loc.search = url.indexOf('?') !== -1 ? '?' + url.split('?')[1].split('#')[0] : '';
        loc.hash = url.indexOf('#') !== -1 ? '#' + url.split('#')[1] : '';
        loc.href = loc.origin + loc.pathname + loc.search + loc.hash;
    }
}

// Location object with navigation interception
globalThis.__neo_location = {
    href: '', origin: '', protocol: 'https:', host: '', hostname: '',
    port: '', pathname: '/', search: '', hash: '',
    assign: function(url) {
        try {
            _shimOps.op_navigation_request(JSON.stringify({
                url: String(url), method: 'GET', type: 'location_assign'
            }));
        } catch(e) {}
    },
    replace: function(url) {
        try {
            _shimOps.op_navigation_request(JSON.stringify({
                url: String(url), method: 'GET', type: 'location_replace'
            }));
        } catch(e) {}
    },
    reload: function() {
        try {
            _shimOps.op_navigation_request(JSON.stringify({
                url: globalThis.__neo_location.href, method: 'GET', type: 'reload'
            }));
        } catch(e) {}
    },
    toString: function() { return this.href; }
};

// Override globalThis.location with intercepting proxy
Object.defineProperty(globalThis, 'location', {
    get: function() { return globalThis.__neo_location; },
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
            get: function() { return globalThis.__neo_location; },
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
// 2. HISTORY API — tracked with state management
// ═══════════════════════════════════════════════════════════════

globalThis.__neo_history = { entries: [], index: -1 };

globalThis.history = {
    pushState: function(state, title, url) {
        var h = globalThis.__neo_history;
        // Truncate forward entries
        h.entries.length = h.index + 1;
        h.entries.push({ state: state, title: title, url: url, nav_type: 'synthetic' });
        h.index = h.entries.length - 1;
        if (url) __neoUpdateLocation(url);
    },
    replaceState: function(state, title, url) {
        var h = globalThis.__neo_history;
        if (h.entries.length > 0 && h.index >= 0) {
            h.entries[h.index] = { state: state, title: title, url: url, nav_type: 'synthetic' };
        } else {
            h.entries.push({ state: state, title: title, url: url, nav_type: 'synthetic' });
            h.index = 0;
        }
        if (url) __neoUpdateLocation(url);
    },
    back: function() {
        var h = globalThis.__neo_history;
        if (h.index > 0) {
            h.index--;
            var entry = h.entries[h.index];
            if (entry && entry.url) __neoUpdateLocation(entry.url);
            try {
                globalThis.dispatchEvent(new PopStateEvent('popstate', { state: entry ? entry.state : null }));
            } catch(e) {}
        }
    },
    forward: function() {
        var h = globalThis.__neo_history;
        if (h.index < h.entries.length - 1) {
            h.index++;
            var entry = h.entries[h.index];
            if (entry && entry.url) __neoUpdateLocation(entry.url);
            try {
                globalThis.dispatchEvent(new PopStateEvent('popstate', { state: entry ? entry.state : null }));
            } catch(e) {}
        }
    },
    go: function(delta) {
        if (!delta) return;
        var h = globalThis.__neo_history;
        var target = h.index + delta;
        if (target >= 0 && target < h.entries.length) {
            h.index = target;
            var entry = h.entries[h.index];
            try {
                globalThis.dispatchEvent(new PopStateEvent('popstate', { state: entry ? entry.state : null }));
            } catch(e) {}
        }
    },
    get length() { return globalThis.__neo_history.entries.length || 1; },
    get state() {
        var h = globalThis.__neo_history;
        if (h.index >= 0 && h.index < h.entries.length) {
            return h.entries[h.index].state;
        }
        return null;
    },
    get scrollRestoration() { return 'auto'; },
    set scrollRestoration(v) {}
};

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

if (typeof HTMLElement !== 'undefined' && HTMLElement.prototype) {
    HTMLElement.prototype.focus = function() { __activeElement = this; };
    HTMLElement.prototype.blur = function() { if (__activeElement === this) __activeElement = null; };
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

// MutationObserver — use linkedom's if available, otherwise stub
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
// 7. SCROLL STUBS — no-op, fake layout geometry
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
