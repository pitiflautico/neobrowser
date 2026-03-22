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
if (typeof document !== 'undefined' && !document.execCommand) {
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
                var sel = globalThis.getSelection();
                if (sel && sel.rangeCount) {
                    var range = sel.getRangeAt(0);
                    range.deleteContents();
                    range.insertNode(document.createTextNode(value));
                }
                el.dispatchEvent(new InputEvent('input', {data: value, inputType: 'insertText', bubbles: true}));
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
            return Array.from(this.options || this.querySelectorAll('option')).filter(function(o) { return o.selected; });
        },
        configurable: true
    });
}

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

// ─── Fix read-only toString on prototypes (linkedom makes some non-writable) ───
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
