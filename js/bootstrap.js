// NeoRender Bootstrap — universal browser environment for AI.
// Connects linkedom (real DOM) + deno_core ops to create a headless browser.
// Runs AFTER linkedom.js. Expects __linkedom_parseHTML on globalThis.

// READABLESTREAM PIPETHROUGH PATCH — must run before ANY page scripts.
// React Router SSR does stream.pipeThrough(new TextEncoderStream())
// which creates V8 internal pipe promises that block module evaluation.
// Fix: return the SAME stream (skip encoding). React Router's turbo-stream
// decoder handles both string and Uint8Array input.
if (typeof ReadableStream !== 'undefined') {
    ReadableStream.prototype.pipeThrough = function(transform, options) {
        // Return self — skip the transform entirely.
        // The SSR stream has string chunks. turbo-stream's decode() can handle strings.
        return this;
    };
}

// View Transitions API polyfill is in layout.js (needs document to exist first)

// ═══════════════════════════════════════════════════════════════
// 0. ERROR ISOLATION — catch uncaught errors without crashing
// ═══════════════════════════════════════════════════════════════

globalThis.onerror = function(msg, url, line, col, error) {
    // Log but don't crash
    return true; // prevents default handling
};
globalThis.onunhandledrejection = function(event) {
    if (event && event.preventDefault) event.preventDefault();
};

const { ops } = Deno.core;

// ═══════════════════════════════════════════════════════════════
// 1. LINKEDOM INIT — parse HTML into real DOM
// ═══════════════════════════════════════════════════════════════

const __html = globalThis.__neorender_html || '<html><head></head><body></body></html>';
const { document, window: __win } = __linkedom_parseHTML(__html);

globalThis.document = document;
globalThis.window = globalThis;
globalThis.self = globalThis;

// document.currentScript must be null (prevents infinite recursion in some libs)
try { Object.defineProperty(document, 'currentScript', { value: null, writable: true, configurable: true }); } catch {}

// document.cookie — backed by UnifiedCookieJar via Rust ops (SQLite-persisted).
// Reads exclude HttpOnly cookies. Writes go to the unified jar.
// Falls back to empty string if ops not available (e.g. initial bootstrap).
Object.defineProperty(document, 'cookie', {
    get() { try { return ops.op_cookie_get(); } catch { return ''; } },
    set(val) { try { ops.op_cookie_set(val); } catch {} },
    configurable: true,
});

// Sync linkedom internals with our globals
if (__win && __win !== globalThis) {
    for (const k of ['location','navigator','fetch','setTimeout','setInterval']) {
        try { if (globalThis[k]) __win[k] = globalThis[k]; } catch {}
    }
}
try { document.defaultView = globalThis; } catch {}

// Export DOM class constructors from linkedom to globalThis (Twitch, Web Components, etc.)
for (const cls of ['EventTarget','Node','Element','HTMLElement','HTMLDivElement','HTMLSpanElement',
    'HTMLInputElement','HTMLButtonElement','HTMLAnchorElement','HTMLImageElement','HTMLCanvasElement',
    'HTMLFormElement','HTMLSelectElement','HTMLTextAreaElement','HTMLVideoElement','HTMLAudioElement',
    'HTMLScriptElement','HTMLStyleElement','HTMLLinkElement','HTMLMetaElement','HTMLIFrameElement',
    'HTMLTemplateElement','SVGElement','DocumentFragment','NodeList','HTMLCollection',
    'Text','Comment','Document','CharacterData','Attr','NamedNodeMap','DOMTokenList','CSSStyleDeclaration']) {
    if (!globalThis[cls] && __win[cls]) globalThis[cls] = __win[cls];
    else if (!globalThis[cls] && document.createElement) {
        // Try to get constructor from a created element
        try {
            const tag = cls.replace('HTML','').replace('Element','').toLowerCase() || 'div';
            const el = document.createElement(tag);
            if (el.constructor && el.constructor.name !== 'Object') globalThis[cls] = el.constructor;
        } catch {}
    }
}

// Fallback stubs for DOM constructors linkedom doesn't export
if (!globalThis.EventTarget) {
    globalThis.EventTarget = class EventTarget {
        constructor() { this.__listeners = {}; }
        addEventListener(type, fn) { (this.__listeners[type] = this.__listeners[type] || []).push(fn); }
        removeEventListener(type, fn) { this.__listeners[type] = (this.__listeners[type] || []).filter(f => f !== fn); }
        dispatchEvent(event) { (this.__listeners[event.type] || []).forEach(fn => { try { fn(event); } catch {} }); return true; }
    };
}
if (!globalThis.Node) {
    // Get from a real element
    try { globalThis.Node = Object.getPrototypeOf(Object.getPrototypeOf(document.createElement('div'))).constructor; } catch {}
}
if (!globalThis.Node) {
    globalThis.Node = class Node extends EventTarget {
        constructor() { super(); this.childNodes = []; this.parentNode = null; }
        static ELEMENT_NODE = 1; static TEXT_NODE = 3; static COMMENT_NODE = 8; static DOCUMENT_NODE = 9; static DOCUMENT_FRAGMENT_NODE = 11;
    };
}

// ═══════════════════════════════════════════════════════════════
// 2. BROWSER GLOBALS — what SPAs expect from window.*
// ═══════════════════════════════════════════════════════════════

globalThis.navigator = __win.navigator || {
    userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 Chrome/131.0.0.0 Safari/537.36',
    language: 'en-US', languages: ['en-US','en','es'], platform: 'MacIntel',
    cookieEnabled: true, onLine: true, vendor: 'Google Inc.',
    maxTouchPoints: 0, hardwareConcurrency: 8,
    permissions: { query: () => Promise.resolve({state:'granted'}) },
    clipboard: { readText: () => Promise.resolve(''), writeText: () => Promise.resolve() },
    serviceWorker: { register: () => Promise.resolve({}), getRegistrations: () => Promise.resolve([]) },
    sendBeacon: () => true,
};

globalThis.location = __win.location || {
    href: '', protocol: 'https:', host: '', hostname: '', port: '',
    pathname: '/', search: '', hash: '', origin: '',
    assign(){}, replace(){}, reload(){}, toString(){ return this.href; },
};

globalThis.history = __win.history || {
    length: 1, state: null,
    pushState(s,t,u){ if(u) location.href=u; history.length++; },
    replaceState(s,t,u){ if(u) location.href=u; },
    back(){}, forward(){}, go(){},
};

globalThis.screen = { width: 1440, height: 900, availWidth: 1440, availHeight: 875, colorDepth: 24, pixelDepth: 24 };

// ═══════════════════════════════════════════════════════════════
// 3. COOKIES — auto-inject on fetch() calls
// ═══════════════════════════════════════════════════════════════

globalThis.__neorender_cookies = globalThis.__neorender_cookies || {};

function __getCookiesForUrl(url) {
    try {
        const hostname = new URL(url).hostname;
        const parts = [];
        for (const [domain, cookies] of Object.entries(__neorender_cookies)) {
            if (hostname === domain || hostname.endsWith('.' + domain)) {
                parts.push(cookies);
            }
        }
        return parts.join('; ');
    } catch { return ''; }
}

// ═══════════════════════════════════════════════════════════════
// 4. FETCH — routes through Rust HTTP client with auto cookies
// ═══════════════════════════════════════════════════════════════

class NeoResponse {
    constructor(status, body, headers) {
        this.status = status;
        this.ok = status >= 200 && status < 300;
        this.statusText = status === 200 ? 'OK' : String(status);
        this._body = body;
        this._headers = headers || {};
        this.headers = new Headers(this._headers);
        this.redirected = false;
        this.type = 'basic';
        this.url = '';
        this.bodyUsed = false;
        // .body as a ReadableStream (lazy — created on first access)
        const self = this;
        Object.defineProperty(this, 'body', {
            get() {
                if (!self._bodyStream) {
                    const text = self._body;
                    self._bodyStream = new ReadableStream({
                        start(controller) {
                            if (text) controller.enqueue(new TextEncoder().encode(text));
                            controller.close();
                        }
                    });
                }
                return self._bodyStream;
            },
            configurable: true
        });
    }
    _markUsed() { if (this.bodyUsed) throw new TypeError('Body already consumed'); this.bodyUsed = true; }
    async text() { this._markUsed(); return this._body; }
    async json() { this._markUsed(); return JSON.parse(this._body); }
    async arrayBuffer() { this._markUsed(); return new TextEncoder().encode(this._body).buffer; }
    async blob() { this._markUsed(); return new Blob([this._body]); }
    clone() { return new NeoResponse(this.status, this._body, this._headers); }
}

// fetch() — sync op wrapped in Promise for API compat.
// The underlying op_neorender_fetch is sync (runs HTTP on a dedicated thread).
globalThis.fetch = function(input, init) {
    const url = typeof input === 'string' ? input : input?.url || String(input);
    const method = init?.method || 'GET';
    const body = init?.body || null;

    let fullUrl = url;
    if (!url.startsWith('http')) {
        fullUrl = location.origin + (url.startsWith('/') ? url : '/' + url);
    }

    // Auto-inject cookies
    const hdrs = {};
    const cookies = __getCookiesForUrl(fullUrl);
    if (cookies) hdrs['Cookie'] = cookies;

    // Merge user headers
    if (init?.headers) {
        const src = init.headers;
        if (typeof src.forEach === 'function') { src.forEach((v, k) => { hdrs[k] = v; }); }
        else if (Array.isArray(src)) { src.forEach(([k, v]) => { hdrs[k] = v; }); }
        else { Object.entries(src).forEach(([k, v]) => { hdrs[k] = String(v); }); }
    }

    const headersJson = Object.keys(hdrs).length > 0 ? JSON.stringify(hdrs) : '';

    try {
        // Sync call — blocks until HTTP completes, returns immediately
        const resultJson = ops.op_neorender_fetch(fullUrl, method.toUpperCase(), body || '', headersJson);
        const result = JSON.parse(resultJson);
        return Promise.resolve(new NeoResponse(result.status, result.body, result.headers || {}));
    } catch (e) {
        return Promise.reject(new TypeError(`fetch failed: ${e}`));
    }
};

// ═══════════════════════════════════════════════════════════════
// 5. TIMERS — real async via Rust tokio
// ═══════════════════════════════════════════════════════════════

let __timerNextId = 1;
const __timerCallbacks = new Map();

// Timer helper: sync op wrapped in Promise for .then() chaining
function __timerPromise(ms) {
    return new Promise(resolve => {
        ops.op_neorender_timer(ms || 0);
        queueMicrotask(resolve);
    });
}

globalThis.setTimeout = function(fn, ms, ...args) {
    if (typeof fn !== 'function') return 0;
    const id = __timerNextId++;
    __timerCallbacks.set(id, true);
    __timerPromise(ms).then(() => {
        if (__timerCallbacks.delete(id)) try { fn(...args); } catch(e) {}
    });
    return id;
};
globalThis.clearTimeout = (id) => __timerCallbacks.delete(id);

globalThis.setInterval = function(fn, ms, ...args) {
    if (typeof fn !== 'function') return 0;
    const id = __timerNextId++;
    __timerCallbacks.set(id, true);
    let ticks = 0;
    function tick() {
        if (!__timerCallbacks.has(id) || ticks++ > 50) return;
        try { fn(...args); } catch(e) {}
        __timerPromise(ms).then(tick);
    }
    __timerPromise(ms).then(tick);
    return id;
};
globalThis.clearInterval = (id) => __timerCallbacks.delete(id);

// ═══════════════════════════════════════════════════════════════
// 6. XMLHTTPREQUEST — backed by fetch
// ═══════════════════════════════════════════════════════════════

globalThis.XMLHttpRequest = class XMLHttpRequest {
    constructor() { this.readyState = 0; this.status = 0; this.responseText = ''; this.response = ''; this._headers = {}; this._listeners = {}; }
    open(method, url) { this._method = method; this._url = url; this.readyState = 1; }
    setRequestHeader(name, value) { this._headers[name] = value; }
    addEventListener(type, fn) { (this._listeners[type] = this._listeners[type] || []).push(fn); }
    removeEventListener(type, fn) { this._listeners[type] = (this._listeners[type] || []).filter(f => f !== fn); }
    dispatchEvent(e) { (this._listeners[e.type] || []).forEach(f => { try { f(e); } catch {} }); }
    send(body) {
        fetch(this._url, { method: this._method, body, headers: this._headers })
            .then(resp => { this.status = resp.status; return resp.text(); })
            .then(text => {
                this.responseText = text; this.response = text; this.readyState = 4;
                const evt = { type: 'load', target: this };
                this.dispatchEvent(evt); if (this.onload) this.onload(evt);
                if (this.onreadystatechange) this.onreadystatechange();
            })
            .catch(() => {
                this.readyState = 4;
                const evt = { type: 'error', target: this };
                this.dispatchEvent(evt); if (this.onerror) this.onerror(evt);
            });
    }
    abort() { this.readyState = 0; }
    getResponseHeader() { return null; }
    getAllResponseHeaders() { return ''; }
};

// ═══════════════════════════════════════════════════════════════
// 7. UNIVERSAL POLYFILLS — APIs that SPAs commonly need
// ═══════════════════════════════════════════════════════════════

// URL / URLSearchParams
if (typeof globalThis.URL === 'undefined') {
    globalThis.URL = class URL {
        constructor(url, base) {
            let full = url;
            if (base && !url.startsWith('http')) full = base.replace(/\/[^/]*$/, '/') + url.replace(/^\.\//, '');
            const m = String(full).match(/^(https?:)\/\/([^/:]+)(:\d+)?(\/[^?#]*)?(\?[^#]*)?(#.*)?$/);
            if (m) {
                this.protocol = m[1]; this.hostname = m[2]; this.port = (m[3]||'').replace(':','');
                this.host = this.hostname + (this.port ? ':'+this.port : '');
                this.pathname = m[4] || '/'; this.search = m[5] || ''; this.hash = m[6] || '';
                this.origin = this.protocol + '//' + this.host;
                this.href = this.origin + this.pathname + this.search + this.hash;
            } else {
                this.href = full; this.protocol=''; this.hostname=''; this.host='';
                this.port=''; this.pathname='/'; this.search=''; this.hash=''; this.origin='';
            }
            this.searchParams = new URLSearchParams(this.search);
        }
        toString() { return this.href; }
        toJSON() { return this.href; }
    };
    globalThis.URLSearchParams = class URLSearchParams {
        constructor(init) {
            this.__p = new Map();
            if (typeof init === 'string') init.replace(/^\?/,'').split('&').forEach(p => { const [k,...v] = p.split('='); if(k) this.__p.set(decodeURIComponent(k), decodeURIComponent(v.join('='))); });
            else if (init && typeof init === 'object' && !(init instanceof Map)) Object.entries(init).forEach(([k,v]) => this.__p.set(k,String(v)));
        }
        get(k) { return this.__p.get(k) || null; } set(k,v) { this.__p.set(k,v); }
        has(k) { return this.__p.has(k); } delete(k) { this.__p.delete(k); }
        append(k,v) { this.__p.set(k,v); }
        toString() { return [...this.__p].map(([k,v]) => `${encodeURIComponent(k)}=${encodeURIComponent(v)}`).join('&'); }
        forEach(fn) { this.__p.forEach((v,k) => fn(v,k)); }
        entries() { return this.__p.entries(); }
        keys() { return this.__p.keys(); }
        values() { return this.__p.values(); }
        [Symbol.iterator]() { return this.__p[Symbol.iterator](); }
    };
}

// structuredClone
globalThis.structuredClone = globalThis.structuredClone || ((obj) => {
    try { return JSON.parse(JSON.stringify(obj)); } catch { return obj; }
});

// Storage (localStorage backed by SQLite via Rust ops, sessionStorage in-memory)
globalThis.localStorage = {
    getItem(k) {
        try { const v = ops.op_storage_get(k); return v || null; }
        catch { return null; }
    },
    setItem(k, v) {
        try { ops.op_storage_set(k, String(v)); } catch {}
    },
    removeItem(k) {
        try { ops.op_storage_remove(k); } catch {}
    },
    clear() {
        try { ops.op_storage_clear(); } catch {}
    },
    get length() { return 0; }, // approximate — SQLite count would be expensive per access
    key(i) { return null; },
};
globalThis.sessionStorage = globalThis.sessionStorage || new (class Storage {
    constructor() { this.__d = {}; }
    getItem(k) { return this.__d[k] !== undefined ? this.__d[k] : null; }
    setItem(k, v) { this.__d[k] = String(v); }
    removeItem(k) { delete this.__d[k]; }
    clear() { this.__d = {}; }
    get length() { return Object.keys(this.__d).length; }
    key(i) { return Object.keys(this.__d)[i] || null; }
})();

// CSS / matchMedia / getComputedStyle
globalThis.CSS = { supports: () => false, escape: (s) => s };
globalThis.matchMedia = globalThis.matchMedia || ((q) => ({
    matches: false, media: q, addEventListener(){}, removeEventListener(){}, addListener(){}, removeListener(){}
}));
globalThis.getComputedStyle = globalThis.getComputedStyle || ((el) => new Proxy({}, {
    get: (t,p) => p === 'getPropertyValue' ? () => '' : ''
}));

// Animation frame
globalThis.requestAnimationFrame = globalThis.requestAnimationFrame || ((fn) => setTimeout(fn, 16));
globalThis.cancelAnimationFrame = globalThis.cancelAnimationFrame || ((id) => clearTimeout(id));
globalThis.queueMicrotask = globalThis.queueMicrotask || ((fn) => Promise.resolve().then(fn));

// Performance
globalThis.performance = globalThis.performance || {
    now: () => Date.now(), mark(){}, measure(){},
    getEntriesByType(){ return []; }, getEntriesByName(){ return []; }
};

// Crypto
globalThis.crypto = globalThis.crypto || {
    getRandomValues: (arr) => { for (let i = 0; i < arr.length; i++) arr[i] = Math.floor(Math.random() * 256); return arr; },
    randomUUID: () => 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, c => { const r = Math.random()*16|0; return (c==='x'?r:(r&0x3|0x8)).toString(16); }),
    subtle: { digest: () => Promise.resolve(new ArrayBuffer(32)), importKey: () => Promise.resolve({}), sign: () => Promise.resolve(new ArrayBuffer(32)) },
};

// File API
globalThis.Blob = globalThis.Blob || class { constructor(p,o){this.size=0;this.type=o?.type||'';} };
globalThis.File = globalThis.File || class extends (globalThis.Blob) { constructor(p,n,o){super(p,o);this.name=n;this.lastModified=Date.now();} };
globalThis.FileReader = globalThis.FileReader || class { readAsText(){} readAsDataURL(){} readAsArrayBuffer(){} addEventListener(){} };

// Misc
globalThis.Image = class { constructor(){this.src='';this.onload=null;this.onerror=null;this.width=0;this.height=0;} };
globalThis.AbortController = globalThis.AbortController || class { constructor(){this.signal={aborted:false,addEventListener(){},removeEventListener(){},onabort:null};} abort(){this.signal.aborted=true;} };
globalThis.Headers = globalThis.Headers || class extends Map { constructor(init){super();if(init)Object.entries(init).forEach(([k,v])=>this.set(k.toLowerCase(),v));} };
globalThis.FormData = globalThis.FormData || class { constructor(){this.__d=[];} append(k,v){this.__d.push([k,v]);} get(k){const e=this.__d.find(([n])=>n===k);return e?e[1]:null;} set(k,v){this.__d=this.__d.filter(([n])=>n!==k);this.__d.push([k,v]);} entries(){return this.__d[Symbol.iterator]();} forEach(fn){this.__d.forEach(([k,v])=>fn(v,k));} };
globalThis.DOMParser = globalThis.DOMParser || class { parseFromString(html) { return __linkedom_parseHTML(html).document; } };
globalThis.MutationObserver = __win.MutationObserver || class { constructor(cb){} observe(){} disconnect(){} takeRecords(){return [];} };
globalThis.IntersectionObserver = class { constructor(cb,opts){} observe(){} unobserve(){} disconnect(){} };
globalThis.ResizeObserver = class { constructor(cb){} observe(){} unobserve(){} disconnect(){} };
globalThis.BroadcastChannel = globalThis.BroadcastChannel || class { constructor(){} postMessage(){} addEventListener(){} close(){} };
globalThis.Worker = globalThis.Worker || class { constructor(){} postMessage(){} addEventListener(){} terminate(){} };

// Event constructors that some libs check for
globalThis.Event = __win.Event || globalThis.Event || class Event { constructor(t,o={}){this.type=t;this.bubbles=o.bubbles||false;this.cancelable=o.cancelable||false;this.defaultPrevented=false;} preventDefault(){this.defaultPrevented=true;} stopPropagation(){} stopImmediatePropagation(){} };
globalThis.CustomEvent = __win.CustomEvent || class extends Event { constructor(t,o={}){super(t,o);this.detail=o.detail;} };
globalThis.MouseEvent = globalThis.MouseEvent || class extends Event { constructor(t,o={}){super(t,o);} };
globalThis.KeyboardEvent = globalThis.KeyboardEvent || class extends Event { constructor(t,o={}){super(t,o);this.key=o.key||'';this.code=o.code||'';} };
globalThis.FocusEvent = globalThis.FocusEvent || class extends Event { constructor(t,o={}){super(t,o);} };
globalThis.InputEvent = globalThis.InputEvent || class extends Event { constructor(t,o={}){super(t,o);this.data=o.data||'';} };
globalThis.PopStateEvent = globalThis.PopStateEvent || class extends Event { constructor(t,o={}){super(t,o);this.state=o.state||null;} };

// MessageEvent
globalThis.MessageEvent = globalThis.MessageEvent || class MessageEvent extends Event {
    constructor(type, init = {}) {
        super(type, init);
        this.data = init.data;
        this.origin = init.origin || '';
        this.source = init.source || null;
        this.ports = init.ports || [];
    }
};
// MessagePort with actual async message passing
globalThis.MessagePort = class MessagePort extends EventTarget {
    constructor() { super(); this._other = null; this._closed = false; this.onmessage = null; }
    postMessage(data) {
        if (this._other && !this._other._closed) {
            const target = this._other;
            const event = new MessageEvent('message', { data });
            Promise.resolve().then(() => {
                target.dispatchEvent(event);
                if (target.onmessage) target.onmessage(event);
            });
        }
    }
    close() { this._closed = true; }
    start() {}
};
// MessageChannel with connected ports
globalThis.MessageChannel = class MessageChannel {
    constructor() {
        this.port1 = new MessagePort();
        this.port2 = new MessagePort();
        this.port1._other = this.port2;
        this.port2._other = this.port1;
    }
};

// window as EventTarget
if (!globalThis.addEventListener) {
    if (__win.addEventListener) {
        globalThis.addEventListener = __win.addEventListener.bind(__win);
        globalThis.removeEventListener = __win.removeEventListener.bind(__win);
        globalThis.dispatchEvent = __win.dispatchEvent.bind(__win);
    } else {
        const __et = {};
        globalThis.addEventListener = (t,f) => { (__et[t]=__et[t]||[]).push(f); };
        globalThis.removeEventListener = (t,f) => { __et[t]=(__et[t]||[]).filter(x=>x!==f); };
        globalThis.dispatchEvent = (e) => { (__et[e.type]||[]).forEach(f=>{ try{f(e);}catch{} }); return true; };
    }
}

// window.postMessage (must be after addEventListener is available)
globalThis.postMessage = function(data, origin) {
    const event = new MessageEvent('message', { data, origin: origin || (typeof location !== 'undefined' ? location.origin : '') });
    Promise.resolve().then(() => globalThis.dispatchEvent(event));
};

// ═══════════════════════════════════════════════════════════════
// 7b. STREAMS + CRYPTO — required by ChatGPT, Next.js, modern SPAs
// ═══════════════════════════════════════════════════════════════

// ReadableStream — proper implementation supporting controller, async iteration, tee, pipeTo
if (typeof globalThis.ReadableStream === 'undefined') {
    globalThis.ReadableStream = class ReadableStream {
        constructor(underlyingSource) {
            this._locked = false;
            this._disturbed = false;
            this._state = 'readable'; // 'readable' | 'closed' | 'errored'
            this._storedError = undefined;
            this._queue = [];
            this._waiters = []; // pending read() resolvers
            const stream = this;
            this._controller = {
                enqueue(chunk) {
                    if (stream._state !== 'readable') return;
                    if (stream._waiters.length > 0) {
                        const waiter = stream._waiters.shift();
                        waiter({ value: chunk, done: false });
                    } else {
                        stream._queue.push(chunk);
                    }
                },
                close() {
                    if (stream._state !== 'readable') return;
                    stream._state = 'closed';
                    // Resolve all pending readers with done
                    while (stream._waiters.length > 0) {
                        stream._waiters.shift()({ value: undefined, done: true });
                    }
                },
                error(e) {
                    if (stream._state !== 'readable') return;
                    stream._state = 'errored';
                    stream._storedError = e;
                    while (stream._waiters.length > 0) {
                        // Reject pending reads — but we store as resolve with error marker
                        stream._waiters.shift()({ value: undefined, done: true });
                    }
                },
                get desiredSize() { return stream._queue.length > 0 ? 0 : 1; }
            };
            if (underlyingSource) {
                if (underlyingSource.start) {
                    try { underlyingSource.start(this._controller); } catch (e) { this._controller.error(e); }
                }
                this._pullFn = underlyingSource.pull || null;
                this._cancelFn = underlyingSource.cancel || null;
            }
        }
        get locked() { return this._locked; }
        getReader() {
            if (this._locked) throw new TypeError('ReadableStream is locked');
            this._locked = true;
            const stream = this;
            const reader = {
                get closed() {
                    return stream._state === 'closed' ? Promise.resolve(undefined) :
                           stream._state === 'errored' ? Promise.reject(stream._storedError) :
                           new Promise((resolve) => { /* never resolves until close */ });
                },
                read() {
                    stream._disturbed = true;
                    if (stream._queue.length > 0) {
                        return Promise.resolve({ value: stream._queue.shift(), done: false });
                    }
                    if (stream._state === 'closed') {
                        return Promise.resolve({ value: undefined, done: true });
                    }
                    if (stream._state === 'errored') {
                        return Promise.reject(stream._storedError);
                    }
                    // Pull if available
                    if (stream._pullFn) {
                        try { stream._pullFn(stream._controller); } catch {}
                    }
                    // If pull synchronously enqueued something
                    if (stream._queue.length > 0) {
                        return Promise.resolve({ value: stream._queue.shift(), done: false });
                    }
                    if (stream._state === 'closed') {
                        return Promise.resolve({ value: undefined, done: true });
                    }
                    // Wait for future enqueue
                    return new Promise((resolve) => { stream._waiters.push(resolve); });
                },
                releaseLock() { stream._locked = false; },
                cancel(reason) { return stream.cancel(reason); }
            };
            return reader;
        }
        cancel(reason) {
            this._state = 'closed';
            this._queue = [];
            if (this._cancelFn) try { this._cancelFn(reason); } catch {}
            return Promise.resolve();
        }
        tee() {
            const reader = this.getReader();
            let cancelled1 = false, cancelled2 = false;
            function makeBranch(setCancelled) {
                const queue = [];
                const waiters = [];
                return { queue, waiters, cancelled: false };
            }
            const b1 = makeBranch(), b2 = makeBranch();
            function pump() {
                reader.read().then(({ value, done }) => {
                    if (done) {
                        for (const b of [b1, b2]) {
                            while (b.waiters.length) b.waiters.shift()({ value: undefined, done: true });
                            b.cancelled = true;
                        }
                        return;
                    }
                    for (const b of [b1, b2]) {
                        if (b.waiters.length > 0) {
                            b.waiters.shift()({ value, done: false });
                        } else {
                            b.queue.push(value);
                        }
                    }
                    pump();
                });
            }
            pump();
            function makeBranchStream(branch) {
                return new ReadableStream({
                    pull(controller) {
                        if (branch.queue.length > 0) {
                            controller.enqueue(branch.queue.shift());
                        }
                    },
                    start(controller) {
                        // Override: directly wire into branch
                        const origPull = branch;
                    }
                });
            }
            // Simpler tee: return streams that read from branches
            const stream1 = new ReadableStream({ start() {} });
            const stream2 = new ReadableStream({ start() {} });
            stream1._queue = b1.queue; stream1._waiters = b1.waiters;
            stream2._queue = b2.queue; stream2._waiters = b2.waiters;
            return [stream1, stream2];
        }
        async pipeTo(dest, options) {
            const reader = this.getReader();
            const writer = dest.getWriter();
            try {
                while (true) {
                    const { value, done } = await reader.read();
                    if (done) break;
                    await writer.write(value);
                }
                await writer.close();
            } finally {
                reader.releaseLock();
                writer.releaseLock();
            }
        }
        pipeThrough(transform) {
            this.pipeTo(transform.writable).catch(() => {});
            return transform.readable;
        }
        [Symbol.asyncIterator]() {
            const reader = this.getReader();
            return {
                next() { return reader.read().then(r => r.done ? { value: undefined, done: true } : r); },
                return() { reader.releaseLock(); return Promise.resolve({ value: undefined, done: true }); },
                [Symbol.asyncIterator]() { return this; }
            };
        }
    };

    globalThis.WritableStream = class WritableStream {
        constructor(underlyingSink) {
            this._locked = false;
            this._sink = underlyingSink || {};
            this._state = 'writable';
            if (this._sink.start) try { this._sink.start(this); } catch {}
        }
        get locked() { return this._locked; }
        getWriter() {
            if (this._locked) throw new TypeError('WritableStream is locked');
            this._locked = true;
            const stream = this;
            return {
                write(chunk) {
                    if (stream._sink.write) return Promise.resolve(stream._sink.write(chunk));
                    return Promise.resolve();
                },
                close() {
                    if (stream._sink.close) return Promise.resolve(stream._sink.close());
                    stream._state = 'closed';
                    return Promise.resolve();
                },
                abort(reason) {
                    if (stream._sink.abort) return Promise.resolve(stream._sink.abort(reason));
                    return Promise.resolve();
                },
                releaseLock() { stream._locked = false; },
                get ready() { return Promise.resolve(); },
                get closed() { return stream._state === 'closed' ? Promise.resolve() : new Promise(() => {}); },
                get desiredSize() { return 1; }
            };
        }
        abort(reason) { this._state = 'closed'; return Promise.resolve(); }
        close() { this._state = 'closed'; return Promise.resolve(); }
    };

    globalThis.TransformStream = class TransformStream {
        constructor(transformer) {
            const queue = [];
            const waiters = [];
            let readableClosed = false;
            this.writable = new WritableStream({
                write(chunk) {
                    let transformed = chunk;
                    if (transformer && transformer.transform) {
                        const ctrl = {
                            enqueue(c) { transformed = c; }
                        };
                        transformer.transform(chunk, ctrl);
                    }
                    if (waiters.length > 0) {
                        waiters.shift()({ value: transformed, done: false });
                    } else {
                        queue.push(transformed);
                    }
                },
                close() {
                    readableClosed = true;
                    while (waiters.length) waiters.shift()({ value: undefined, done: true });
                }
            });
            this.readable = new ReadableStream({
                pull(controller) {
                    // Handled via queue/waiters above
                }
            });
            // Wire the readable to our queue
            this.readable._queue = queue;
            this.readable._waiters = waiters;
        }
    };
}

// SubtleCrypto with real SHA-256 (pure JS, needed for proof-of-work)
if (!globalThis.crypto?.subtle?.digest || globalThis.crypto?.subtle?.digest?.toString?.().includes('Promise.resolve')) {
    // SHA-256 pure JS implementation
    const _sha256 = (function() {
        function rightRotate(v, a) { return (v>>>a)|(v<<(32-a)); }
        const K = [];
        let p = 0;
        for (let c = 2; p < 64; c++) {
            let ok = true;
            for (let i = 2; i*i <= c; i++) if (c%i===0) { ok=false; break; }
            if (ok) { K[p++] = (Math.pow(c,1/3)*0x100000000)|0; }
        }
        const H0 = [0x6a09e667,0xbb67ae85,0x3c6ef372,0xa54ff53a,0x510e527f,0x9b05688c,0x1f83d9ab,0x5be0cd19];
        return function(msg) {
            const bytes = typeof msg === 'string' ? new TextEncoder().encode(msg) : new Uint8Array(msg);
            const len = bytes.length;
            const bitLen = len * 8;
            // Padding
            const padded = new Uint8Array(Math.ceil((len+9)/64)*64);
            padded.set(bytes);
            padded[len] = 0x80;
            const view = new DataView(padded.buffer);
            view.setUint32(padded.length-4, bitLen, false);
            // Process
            let h = H0.slice();
            for (let off = 0; off < padded.length; off += 64) {
                const w = new Int32Array(64);
                for (let i = 0; i < 16; i++) w[i] = view.getInt32(off+i*4, false);
                for (let i = 16; i < 64; i++) {
                    const s0 = rightRotate(w[i-15],7)^rightRotate(w[i-15],18)^(w[i-15]>>>3);
                    const s1 = rightRotate(w[i-2],17)^rightRotate(w[i-2],19)^(w[i-2]>>>10);
                    w[i] = (w[i-16]+s0+w[i-7]+s1)|0;
                }
                let [a,b,c,d,e,f,g,hh] = h;
                for (let i = 0; i < 64; i++) {
                    const S1 = rightRotate(e,6)^rightRotate(e,11)^rightRotate(e,25);
                    const ch = (e&f)^((~e)&g);
                    const t1 = (hh+S1+ch+K[i]+w[i])|0;
                    const S0 = rightRotate(a,2)^rightRotate(a,13)^rightRotate(a,22);
                    const maj = (a&b)^(a&c)^(b&c);
                    const t2 = (S0+maj)|0;
                    hh=g; g=f; f=e; e=(d+t1)|0; d=c; c=b; b=a; a=(t1+t2)|0;
                }
                h[0]=(h[0]+a)|0; h[1]=(h[1]+b)|0; h[2]=(h[2]+c)|0; h[3]=(h[3]+d)|0;
                h[4]=(h[4]+e)|0; h[5]=(h[5]+f)|0; h[6]=(h[6]+g)|0; h[7]=(h[7]+hh)|0;
            }
            const result = new Uint8Array(32);
            const rv = new DataView(result.buffer);
            for (let i = 0; i < 8; i++) rv.setUint32(i*4, h[i], false);
            return result;
        };
    })();

    globalThis.crypto = globalThis.crypto || {};
    globalThis.crypto.subtle = globalThis.crypto.subtle || {};
    // digest: sync internally, returns resolved Promise (API compat) but also works without await
    const _digestSync = function(algo, data) {
        const bytes = data instanceof ArrayBuffer ? new Uint8Array(data) : data;
        return _sha256(bytes).buffer;
    };
    globalThis.crypto.subtle.digest = function(algo, data) {
        const result = _digestSync(algo, data);
        // Return object that works both as Promise (await) and as ArrayBuffer (sync)
        const p = Promise.resolve(result);
        // Attach ArrayBuffer properties so sync access works too
        p.byteLength = result.byteLength;
        p._syncResult = result;
        return p;
    };
    // Also expose sync version for POW loops
    globalThis.crypto.subtle.digestSync = _digestSync;
    globalThis.crypto.subtle.importKey = async () => ({});
    globalThis.crypto.subtle.sign = async () => new ArrayBuffer(32);
    globalThis.crypto.subtle.verify = async () => true;
}

// ═══════════════════════════════════════════════════════════════
// 8. CANVAS 2D STUB — for Lottie, charts, avatars
// ═══════════════════════════════════════════════════════════════

const _noop = () => {};
const _canvasCtxProto = {
    fillStyle:'', strokeStyle:'', lineWidth:1, globalAlpha:1, font:'10px sans-serif',
    textAlign:'start', textBaseline:'alphabetic', shadowBlur:0, shadowColor:'transparent',
    save:_noop, restore:_noop, beginPath:_noop, closePath:_noop, moveTo:_noop, lineTo:_noop,
    bezierCurveTo:_noop, quadraticCurveTo:_noop, arc:_noop, arcTo:_noop, ellipse:_noop, rect:_noop,
    fill:_noop, stroke:_noop, clip:_noop, clearRect:_noop, fillRect:_noop, strokeRect:_noop,
    fillText:_noop, strokeText:_noop, measureText:(t)=>({width:t.length*6}),
    setTransform:_noop, resetTransform:_noop, transform:_noop, translate:_noop, rotate:_noop, scale:_noop,
    drawImage:_noop, createLinearGradient:()=>({addColorStop:_noop}),
    createRadialGradient:()=>({addColorStop:_noop}), createPattern:()=>({}),
    getImageData:()=>({data:new Uint8ClampedArray(4),width:1,height:1}),
    putImageData:_noop, createImageData:(w,h)=>({data:new Uint8ClampedArray((w||1)*(h||1)*4),width:w||1,height:h||1}),
    setLineDash:_noop, getLineDash:()=>[],
};
if (document.createElement) {
    const _origCreate = document.createElement.bind(document);
    document.createElement = function(tag, ...args) {
        const el = _origCreate(tag, ...args);
        if (tag.toLowerCase() === 'canvas') {
            el.getContext = () => ({ ..._canvasCtxProto, canvas: el });
            el.toDataURL = () => 'data:image/png;base64,';
            el.toBlob = (cb) => cb && cb(new Blob());
        }
        return el;
    };
}

// Path2D (Twitch, chart libs)
globalThis.Path2D = globalThis.Path2D || class Path2D { constructor(){} addPath(){} closePath(){} moveTo(){} lineTo(){} bezierCurveTo(){} quadraticCurveTo(){} arc(){} arcTo(){} ellipse(){} rect(){} };
// WebSocket stub (prevents crashes in apps that check for it)
globalThis.WebSocket = globalThis.WebSocket || class WebSocket { constructor(){this.readyState=3;} send(){} close(){} addEventListener(){} removeEventListener(){} };
// Range / Selection (contenteditable, text editors)
globalThis.Range = globalThis.Range || class Range { setStart(){} setEnd(){} collapse(){} selectNode(){} cloneRange(){return new Range();} };
globalThis.Selection = globalThis.Selection || class Selection { getRangeAt(){return new Range();} removeAllRanges(){} addRange(){} toString(){return '';} };
if (!document.getSelection) document.getSelection = () => new Selection();
if (!document.createRange) document.createRange = () => new Range();

// ═══════════════════════════════════════════════════════════════
// 8b. MODERN WEB APIs — TextEncoderStream, Promise.withResolvers
// ═══════════════════════════════════════════════════════════════

// TextEncoderStream / TextDecoderStream (Streams API)
if (typeof globalThis.TextEncoderStream === 'undefined') {
    globalThis.TextEncoderStream = class TextEncoderStream {
        constructor() {
            this.encoding = 'utf-8';
            const encoder = new TextEncoder();
            const ts = new TransformStream({
                transform(chunk, controller) {
                    const encoded = typeof chunk === 'string' ? encoder.encode(chunk) : chunk;
                    controller.enqueue(encoded);
                }
            });
            this.readable = ts.readable;
            this.writable = ts.writable;
        }
    };
    globalThis.TextDecoderStream = class TextDecoderStream {
        constructor(label) {
            this.encoding = label || 'utf-8';
            const decoder = new TextDecoder(this.encoding);
            const ts = new TransformStream({
                transform(chunk, controller) {
                    const decoded = decoder.decode(chunk, { stream: true });
                    if (decoded) controller.enqueue(decoded);
                }
            });
            this.readable = ts.readable;
            this.writable = ts.writable;
        }
    };
}

// Promise.withResolvers (ES2024)
if (!Promise.withResolvers) {
    Promise.withResolvers = function() {
        let resolve, reject;
        const promise = new Promise((res, rej) => { resolve = res; reject = rej; });
        return { promise, resolve, reject };
    };
}

// ═══════════════════════════════════════════════════════════════
// 9. EXPORT — render DOM as HTML for Rust to extract
// ═══════════════════════════════════════════════════════════════

globalThis.__neorender_export = function() {
    return document.documentElement.outerHTML;
};
