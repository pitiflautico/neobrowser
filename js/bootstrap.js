// NeoRender Bootstrap — universal browser environment for AI.
// Connects linkedom (real DOM) + deno_core ops to create a headless browser.
// Runs AFTER linkedom.js. Expects __linkedom_parseHTML on globalThis.

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
// document.cookie must be a string (linkedom leaves it undefined)
if (document.cookie === undefined) document.cookie = '';

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
    }
    async text() { return this._body; }
    async json() { return JSON.parse(this._body); }
    async arrayBuffer() { return new TextEncoder().encode(this._body).buffer; }
    async blob() { return new Blob([this._body]); }
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

globalThis.setTimeout = function(fn, ms, ...args) {
    if (typeof fn !== 'function') return 0;
    const id = __timerNextId++;
    __timerCallbacks.set(id, true);
    ops.op_neorender_timer(ms || 0).then(() => {
        if (__timerCallbacks.delete(id)) try { fn(...args); } catch(e) {}
    });
    return id;
};
globalThis.clearTimeout = (id) => __timerCallbacks.delete(id);

globalThis.setInterval = function(fn, ms, ...args) {
    if (typeof fn !== 'function') return 0;
    const id = __timerNextId++;
    __timerCallbacks.set(id, true);
    function tick() {
        if (!__timerCallbacks.has(id)) return;
        try { fn(...args); } catch(e) {}
        ops.op_neorender_timer(ms || 0).then(tick);
    }
    ops.op_neorender_timer(ms || 0).then(tick);
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

// Storage (localStorage / sessionStorage)
globalThis.localStorage = globalThis.localStorage || new (class Storage {
    constructor() { this.__d = {}; }
    getItem(k) { return this.__d[k] !== undefined ? this.__d[k] : null; }
    setItem(k, v) { this.__d[k] = String(v); }
    removeItem(k) { delete this.__d[k]; }
    clear() { this.__d = {}; }
    get length() { return Object.keys(this.__d).length; }
    key(i) { return Object.keys(this.__d)[i] || null; }
})();
globalThis.sessionStorage = globalThis.sessionStorage || new globalThis.localStorage.constructor();

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
globalThis.MessageChannel = globalThis.MessageChannel || class { constructor(){ const noop={postMessage(){},addEventListener(){},removeEventListener(){},onmessage:null}; this.port1=noop; this.port2=noop; } };
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

// ═══════════════════════════════════════════════════════════════
// 7b. STREAMS + CRYPTO — required by ChatGPT, Next.js, modern SPAs
// ═══════════════════════════════════════════════════════════════

// ReadableStream (minimal but functional for fetch response streaming)
if (typeof globalThis.ReadableStream === 'undefined') {
    globalThis.ReadableStream = class ReadableStream {
        constructor(source) {
            this._source = source;
            this._controller = { enqueue(chunk) { this._chunks = this._chunks || []; this._chunks.push(chunk); }, close() { this._closed = true; }, error(e) { this._error = e; }, _chunks: [], _closed: false };
            if (source && source.start) try { source.start(this._controller); } catch {}
        }
        getReader() {
            const ctrl = this._controller;
            let idx = 0;
            return {
                read() {
                    if (idx < (ctrl._chunks||[]).length) return Promise.resolve({value: ctrl._chunks[idx++], done: false});
                    if (ctrl._closed) return Promise.resolve({value: undefined, done: true});
                    return Promise.resolve({value: undefined, done: true});
                },
                releaseLock() {},
                cancel() { return Promise.resolve(); }
            };
        }
        [Symbol.asyncIterator]() {
            const reader = this.getReader();
            return { next() { return reader.read(); }, return() { reader.releaseLock(); return Promise.resolve({done:true}); } };
        }
        tee() { return [this, this]; }
        pipeTo() { return Promise.resolve(); }
        pipeThrough(transform) { return transform.readable || this; }
    };
    globalThis.WritableStream = class WritableStream { constructor(){} getWriter(){ return {write(){return Promise.resolve()},close(){return Promise.resolve()},releaseLock(){}}; } };
    globalThis.TransformStream = class TransformStream { constructor(){ this.readable = new ReadableStream(); this.writable = new WritableStream(); } };
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
// 9. EXPORT — render DOM as HTML for Rust to extract
// ═══════════════════════════════════════════════════════════════

globalThis.__neorender_export = function() {
    return document.documentElement.outerHTML;
};
