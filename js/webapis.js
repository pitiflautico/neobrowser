// ═══════════════════════════════════════════════════════════════
// MISC WEB APIs — common APIs that sites check for
// ═══════════════════════════════════════════════════════════════

// Permissions API
if (!navigator.permissions) {
    navigator.permissions = {
        query(desc) { return Promise.resolve({ state: 'granted', onchange: null, addEventListener() {} }); }
    };
}

// Clipboard API
if (!navigator.clipboard) {
    let _clip = '';
    navigator.clipboard = {
        writeText(text) { _clip = text; return Promise.resolve(); },
        readText() { return Promise.resolve(_clip); },
        write() { return Promise.resolve(); },
        read() { return Promise.resolve([]); },
    };
}

// Intl (often checked by date/number formatting libs)
// V8 has Intl built-in, but ensure common methods exist
if (!Intl.Segmenter) {
    Intl.Segmenter = class Segmenter {
        constructor(locale, opts) { this.locale = locale; this.granularity = opts?.granularity || 'grapheme'; }
        segment(str) {
            const segments = [];
            for (let i = 0; i < str.length; i++) segments.push({ segment: str[i], index: i, input: str });
            return { [Symbol.iterator]() { return segments[Symbol.iterator](); }, containing(i) { return segments[i]; } };
        }
    };
}

// ResizeObserver (functional — calls callback immediately with empty entries)
globalThis.ResizeObserver = globalThis.ResizeObserver || class ResizeObserver {
    constructor(cb) { this._cb = cb; this._elements = []; }
    observe(el) { this._elements.push(el); }
    unobserve(el) { this._elements = this._elements.filter(e => e !== el); }
    disconnect() { this._elements = []; }
};

// IntersectionObserver (functional — marks all as visible)
globalThis.IntersectionObserver = class IntersectionObserver {
    constructor(cb, opts) {
        this._cb = cb; this._elements = [];
        this.root = opts?.root || null;
        this.rootMargin = opts?.rootMargin || '0px';
        this.thresholds = opts?.threshold ? [].concat(opts.threshold) : [0];
    }
    observe(el) {
        this._elements.push(el);
        // Mark as visible immediately (no layout = everything visible)
        Promise.resolve().then(() => {
            this._cb([{
                target: el, isIntersecting: true,
                intersectionRatio: 1, boundingClientRect: {}, intersectionRect: {},
                rootBounds: null, time: Date.now(),
            }], this);
        });
    }
    unobserve(el) { this._elements = this._elements.filter(e => e !== el); }
    disconnect() { this._elements = []; }
    takeRecords() { return []; }
};

// matchMedia (functional — returns based on common queries)
globalThis.matchMedia = function(query) {
    let matches = false;
    if (query.includes('prefers-color-scheme: dark')) matches = false;
    if (query.includes('prefers-color-scheme: light')) matches = true;
    if (query.includes('prefers-reduced-motion')) matches = false;
    if (query.includes('min-width')) {
        const w = parseInt(query.match(/min-width:\s*(\d+)/)?.[1] || '0');
        matches = 1920 >= w;
    }
    if (query.includes('max-width')) {
        const w = parseInt(query.match(/max-width:\s*(\d+)/)?.[1] || '99999');
        matches = 1920 <= w;
    }
    const mql = {
        matches, media: query,
        addEventListener(type, fn) { /* static */ },
        removeEventListener() {},
        addListener(fn) { /* deprecated */ },
        removeListener() {},
        onchange: null,
    };
    return mql;
};

// getComputedStyle (returns empty-ish style)
globalThis.getComputedStyle = function(el, pseudo) {
    const styles = {};
    return new Proxy(styles, {
        get(target, prop) {
            if (prop === 'getPropertyValue') return (name) => '';
            if (prop === 'length') return 0;
            if (prop === 'cssText') return '';
            if (prop === Symbol.iterator) return function*() {};
            // Common properties with defaults
            if (prop === 'display') return el?.style?.display || 'block';
            if (prop === 'visibility') return 'visible';
            if (prop === 'opacity') return '1';
            if (prop === 'position') return 'static';
            if (prop === 'overflow') return 'visible';
            return '';
        }
    });
};

// requestIdleCallback
globalThis.requestIdleCallback = globalThis.requestIdleCallback || function(cb, opts) {
    return setTimeout(() => cb({ didTimeout: false, timeRemaining: () => 50 }), 1);
};
globalThis.cancelIdleCallback = globalThis.cancelIdleCallback || clearTimeout;

// VisualViewport
globalThis.visualViewport = globalThis.visualViewport || {
    width: 1920, height: 1080, offsetLeft: 0, offsetTop: 0,
    pageLeft: 0, pageTop: 0, scale: 1,
    addEventListener() {}, removeEventListener() {},
};

// DevicePixelRatio
if (!globalThis.devicePixelRatio) globalThis.devicePixelRatio = 2;

// Focus/blur events
globalThis.focus = function() {};
globalThis.blur = function() {};

// Print
globalThis.print = function() {};

// Alert/confirm/prompt
globalThis.alert = function() {};
globalThis.confirm = function() { return true; };
globalThis.prompt = function(msg, def) { return def || ''; };

// opener/parent/top/frameElement
globalThis.opener = null;
globalThis.parent = globalThis;
globalThis.top = globalThis;
globalThis.frameElement = null;

// innerWidth/innerHeight
Object.defineProperty(globalThis, 'innerWidth', { get: () => 1920, configurable: true });
Object.defineProperty(globalThis, 'innerHeight', { get: () => 1080, configurable: true });
Object.defineProperty(globalThis, 'outerWidth', { get: () => 1920, configurable: true });
Object.defineProperty(globalThis, 'outerHeight', { get: () => 1120, configurable: true });
Object.defineProperty(globalThis, 'scrollX', { get: () => 0, configurable: true });
Object.defineProperty(globalThis, 'scrollY', { get: () => 0, configurable: true });
Object.defineProperty(globalThis, 'pageXOffset', { get: () => 0, configurable: true });
Object.defineProperty(globalThis, 'pageYOffset', { get: () => 0, configurable: true });

// scroll functions
globalThis.scrollTo = function() {};
globalThis.scrollBy = function() {};
globalThis.scroll = function() {};

// DOMRect
globalThis.DOMRect = globalThis.DOMRect || class DOMRect {
    constructor(x,y,w,h) { this.x=x||0; this.y=y||0; this.width=w||0; this.height=h||0; this.top=this.y; this.left=this.x; this.bottom=this.y+(this.height||0); this.right=this.x+(this.width||0); }
    toJSON() { return {x:this.x,y:this.y,width:this.width,height:this.height,top:this.top,left:this.left,bottom:this.bottom,right:this.right}; }
};
globalThis.DOMRectReadOnly = globalThis.DOMRect;

// Element.getBoundingClientRect (no layout, return 0s)
if (typeof Element !== 'undefined' && !Element.prototype.getBoundingClientRect) {
    Element.prototype.getBoundingClientRect = function() { return new DOMRect(0,0,0,0); };
    Element.prototype.getClientRects = function() { return [new DOMRect(0,0,0,0)]; };
}

// window.open (no-op)
globalThis.open = function(url) { return null; };

// Notification API
globalThis.Notification = globalThis.Notification || class Notification {
    static permission = 'denied';
    static requestPermission() { return Promise.resolve('denied'); }
    constructor() {}
};

// SpeechSynthesis
globalThis.speechSynthesis = globalThis.speechSynthesis || {
    speak() {}, cancel() {}, pause() {}, resume() {},
    getVoices() { return []; }, speaking: false, pending: false, paused: false,
};
