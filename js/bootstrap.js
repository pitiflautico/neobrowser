// NeoRender V2 Bootstrap — universal browser environment for AI.
// Connects happy-dom (full DOM) + deno_core ops to create a headless browser.
// Runs AFTER happy-dom.bundle.js. Expects `happydom` global with Window class.
// V2: uses op_fetch/op_console_log + deno_core native WebTimers (not op_neorender_*).

// ═══════════════════════════════════════════════════════════════
// REACT INTERCEPTION PRIMITIVES — must run BEFORE any page scripts.
// ═══════════════════════════════════════════════════════════════

// Note: ReadableStream.pipeThrough patch moved to after section 7b (streams polyfill).

// Object.prototype.getAll — React Router Early Hints calls getAll() on
// SSR response context (not Headers). Return empty array (no hints in headless).
if (!Object.prototype.getAll) {
    Object.defineProperty(Object.prototype, 'getAll', {
        value: function() { return []; },
        configurable: true, writable: true, enumerable: false,
    });
}

// HEADERS.GETALL POLYFILL — React Router uses getAll() which was removed from Fetch spec.
// Must be added via defineProperty since deno_core's Headers is a native object.
try {
    Object.defineProperty(Headers.prototype, 'getAll', {
        value: function(name) {
            const r = [];
            this.forEach((v, k) => { if (k.toLowerCase() === name.toLowerCase()) r.push(v); });
            return r;
        }, configurable: true, writable: true
    });
} catch {}

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

// Route console.log through Rust op for capture.
const _origConsole = globalThis.console || {};
globalThis.console = {
    log: (...args) => { try { ops.op_console_log(args.map(String).join(' ')); } catch {} },
    warn: (...args) => { try { ops.op_console_log('[warn] ' + args.map(String).join(' ')); } catch {} },
    error: (...args) => { try { ops.op_console_log('[error] ' + args.map(String).join(' ')); } catch {} },
    info: (...args) => { try { ops.op_console_log(args.map(String).join(' ')); } catch {} },
    debug: () => {},
    trace: () => {},
    dir: () => {},
    table: () => {},
    group: () => {},
    groupEnd: () => {},
    groupCollapsed: () => {},
    assert: () => {},
    count: () => {},
    countReset: () => {},
    time: () => {},
    timeEnd: () => {},
    timeLog: () => {},
    clear: () => {},
};

// Trace helper — only emits when NEORENDER_TRACE=1 (Rust sets __neorender_trace).
function neo_trace(msg) {
    if (globalThis.__neorender_trace && ops.op_console_log) {
        try { ops.op_console_log('[TRACE] ' + msg); } catch {}
    }
}

// ═══════════════════════════════════════════════════════════════
// 1. HAPPY-DOM INIT — create Window + parse HTML into real DOM
// ═══════════════════════════════════════════════════════════════

const __html = globalThis.__neorender_html || '<html><head></head><body></body></html>';
const __url = globalThis.__neorender_url || 'about:blank';

// Create happy-dom Window (full browser environment)
const __hdWindow = new happydom.Window({ url: __url });
const __hdDocument = __hdWindow.document;

// Parse HTML into happy-dom document
__hdDocument.documentElement.innerHTML = '';
__hdDocument.write(__html);

// Install happy-dom document and classes as globals
globalThis.document = __hdDocument;
globalThis.window = globalThis;
globalThis.self = globalThis;

// Export happy-dom's DOM classes to global scope so page scripts can use them
const __hdClasses = [
    'Node', 'Element', 'HTMLElement', 'Text', 'Comment', 'DocumentFragment',
    'HTMLDivElement', 'HTMLSpanElement', 'HTMLInputElement', 'HTMLButtonElement',
    'HTMLAnchorElement', 'HTMLFormElement', 'HTMLSelectElement', 'HTMLOptionElement',
    'HTMLTextAreaElement', 'HTMLImageElement', 'HTMLScriptElement', 'HTMLStyleElement',
    'HTMLLinkElement', 'HTMLMetaElement', 'HTMLHeadElement', 'HTMLBodyElement',
    'HTMLTableElement', 'HTMLTableRowElement', 'HTMLTableCellElement',
    'HTMLLabelElement', 'HTMLFieldSetElement', 'HTMLLegendElement',
    'HTMLIFrameElement', 'HTMLCanvasElement', 'HTMLVideoElement', 'HTMLAudioElement',
    'HTMLTemplateElement', 'HTMLSlotElement', 'HTMLDialogElement',
    'SVGElement', 'SVGSVGElement',
    'Event', 'CustomEvent', 'MouseEvent', 'KeyboardEvent', 'FocusEvent',
    'InputEvent', 'SubmitEvent', 'UIEvent', 'ErrorEvent', 'ProgressEvent',
    'DragEvent', 'AnimationEvent', 'TransitionEvent', 'WheelEvent',
    'MutationObserver', 'IntersectionObserver', 'ResizeObserver',
    'DOMParser', 'XMLSerializer', 'Range', 'Selection',
    'NodeList', 'HTMLCollection', 'DOMTokenList', 'NamedNodeMap',
    'Attr', 'CSSStyleDeclaration', 'CSSStyleSheet',
    'AbortController', 'AbortSignal',
    'Blob', 'File', 'FileReader', 'FormData',
    'Headers', 'Request', 'Response',
    'URL', 'URLSearchParams',
    'MediaQueryList',
    'Storage',
];
for (const name of __hdClasses) {
    if (__hdWindow[name] && !globalThis[name]) {
        globalThis[name] = __hdWindow[name];
    }
}

// Also provide the parseHTML function for DOMParser and re-navigation
globalThis.__linkedom_parseHTML = function(html) {
    const w = new happydom.Window({ url: __url });
    w.document.write(html);
    return { document: w.document, window: w };
};

// Self-test: verify happy-dom basics work
(function selfTest() {
    const el = document.createElement("div");
    el.setAttribute("class", "test-cls");
    if (el.className !== "test-cls") {
        console.error("[happy-dom selftest] className sync FAILED: " + el.className);
    }
    el.innerText = "hello";
    if (el.textContent !== "hello") {
        console.error("[happy-dom selftest] innerText setter FAILED: " + el.textContent);
    }
    if (!document.querySelector) {
        console.error("[happy-dom selftest] querySelector missing");
    }
})();

// document.currentScript must be null (prevents infinite recursion in some libs)
try { Object.defineProperty(document, 'currentScript', { value: null, writable: true, configurable: true }); } catch {}

// document.cookie — in-memory store (V2 has no cookie ops yet).
let __cookie_store = '';
Object.defineProperty(document, 'cookie', {
    get() { return __cookie_store; },
    set(val) { __cookie_store = val; },
    configurable: true,
});

// Sync happy-dom window internals with our globals
try { document.defaultView = globalThis; } catch {}

// ViewTransition API — React 19 uses document.startViewTransition for route changes.
if (typeof document !== 'undefined' && !document.startViewTransition) {
    document.startViewTransition = function(cbOrOpts) {
        const cb = typeof cbOrOpts === 'function' ? cbOrOpts : cbOrOpts?.update;
        const result = cb ? cb() : undefined;
        const done = result instanceof Promise ? result : Promise.resolve();
        return { finished: done, ready: Promise.resolve(), updateCallbackDone: done, skipTransition: function() {} };
    };
}

// DOM class constructors already exported from happy-dom in section 1 above.

// ═══════════════════════════════════════════════════════════════
// 2. BROWSER GLOBALS — what SPAs expect from window.*
// ═══════════════════════════════════════════════════════════════

globalThis.navigator = __hdWindow.navigator || {
    userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 Chrome/131.0.0.0 Safari/537.36',
    language: 'en-US', languages: ['en-US','en','es'], platform: 'MacIntel',
    cookieEnabled: true, onLine: true, vendor: 'Google Inc.',
    maxTouchPoints: 0, hardwareConcurrency: 8,
    permissions: { query: () => Promise.resolve({state:'granted'}) },
    clipboard: { readText: () => Promise.resolve(''), writeText: () => Promise.resolve() },
    serviceWorker: { register: () => Promise.resolve({}), getRegistrations: () => Promise.resolve([]) },
    sendBeacon: () => true,
};

globalThis.location = __hdWindow.location || {
    href: '', protocol: 'https:', host: '', hostname: '', port: '',
    pathname: '/', search: '', hash: '', origin: '',
    assign(){}, replace(){}, reload(){}, toString(){ return this.href; },
};

globalThis.history = __hdWindow.history || {
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
    constructor(bodyBytes, init) {
        this._body = bodyBytes;           // Uint8Array
        this._bodyText = init._text !== undefined ? init._text : null; // pre-decoded text (optimization)
        this._bodyUsed = false;
        this._url = init.url || '';
        this.status = init.status || 200;
        this.statusText = init.statusText || '';
        this.ok = this.status >= 200 && this.status < 300;
        this.headers = new Headers(init.headers || {});
        this._rawHeaders = init.headers || {};
        this.redirected = init.redirected || false;
        this.type = 'basic';
        this._stream = null; // lazy
    }

    get bodyUsed() { return this._bodyUsed; }
    get url() { return this._url; }

    get body() {
        if (!this._stream) {
            neo_trace('[FETCH] response.body accessed for ' + this._url);
            const bytes = this._body;
            this._stream = new ReadableStream({
                start(controller) {
                    if (bytes && bytes.length > 0) controller.enqueue(bytes);
                    controller.close();
                }
            });
        }
        return this._stream;
    }

    _consumeCheck() {
        if (this._bodyUsed) throw new TypeError('body already consumed');
        this._bodyUsed = true;
    }

    async text() {
        this._consumeCheck();
        if (this._bodyText !== null) return this._bodyText;
        return new TextDecoder().decode(this._body);
    }

    async json() {
        const t = await this.text();
        return JSON.parse(t);
    }

    async arrayBuffer() {
        this._consumeCheck();
        return this._body.buffer.slice(0);
    }

    async blob() {
        this._consumeCheck();
        return new Blob([this._body]);
    }

    clone() {
        if (this._bodyUsed) throw new TypeError('cannot clone consumed response');
        return new NeoResponse(
            new Uint8Array(this._body),
            { status: this.status, statusText: this.statusText, headers: this._rawHeaders, url: this._url, _text: this._bodyText, redirected: this.redirected }
        );
    }
}
// Make `response instanceof Response` work
globalThis.Response = NeoResponse;

// fetch() — sync op wrapped in Promise for API compat.
// The underlying op_fetch is sync (runs HTTP on a dedicated thread).
globalThis.fetch = function(input, init) {
    const url = typeof input === 'string' ? input : input?.url || String(input);
    const method = init?.method || 'GET';
    const body = init?.body || null;
    __pendingFetches++;
    __neo_markActivity('fetch-start');

    let fullUrl = url;
    if (!url.startsWith('http')) {
        fullUrl = location.origin + (url.startsWith('/') ? url : '/' + url);
    }

    neo_trace('[FETCH] ' + method + ' ' + fullUrl);

    // Rich fetch trace
    const fetchId = ++__fetchIdCounter;
    const fetchEntry = {
        id: fetchId,
        url: fullUrl.substring(0, 120),
        method: method,
        startMs: Date.now(),
        endMs: null,
        status: null,
        bodyConsumed: false,
        error: null,
    };
    __fetchLog.push(fetchEntry);
    if (__fetchLog.length > 50) __fetchLog.shift(); // cap memory
    console.log('[FETCH-TRACE] #' + fetchId + ' START ' + method + ' ' + fullUrl.substring(0, 80));

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

    // Async op — yields to event loop during I/O. The Promise resolves
    // in a FUTURE tick, not the current one. This is critical for React:
    // click → fetch → .then(setState) must have a yield between them.
    // In deno_core 0.311, async ops called via ops.X() return a Promise directly.
    return ops.op_fetch(fullUrl, method.toUpperCase(), body || '', headersJson)
        .then(resultJson => {
            __pendingFetches--;
            __neo_markActivity('fetch-end');
            fetchEntry.endMs = Date.now();
            const result = JSON.parse(resultJson);
            fetchEntry.status = result.status;
            console.log('[FETCH-TRACE] #' + fetchId + ' END ' + result.status + ' (' + (fetchEntry.endMs - fetchEntry.startMs) + 'ms)');
            const bodyText = result.body || '';
            const bodyBytes = new TextEncoder().encode(bodyText);
            const resp = new NeoResponse(bodyBytes, {
                status: result.status,
                statusText: result.statusText || (result.status === 200 ? 'OK' : String(result.status)),
                headers: result.headers || {},
                url: fullUrl,
                _text: bodyText,
            });
            // Wrap body consumption methods for tracing
            const origText = resp.text.bind(resp);
            const origJson = resp.json.bind(resp);
            resp.text = async function() {
                fetchEntry.bodyConsumed = true;
                console.log('[FETCH-TRACE] #' + fetchId + ' BODY text() consumed');
                return origText();
            };
            resp.json = async function() {
                fetchEntry.bodyConsumed = true;
                console.log('[FETCH-TRACE] #' + fetchId + ' BODY json() consumed');
                return origJson();
            };
            return resp;
        })
        .catch(e => {
            __pendingFetches--;
            fetchEntry.endMs = Date.now();
            fetchEntry.error = String(e);
            console.log('[FETCH-TRACE] #' + fetchId + ' ERROR ' + e);
            throw new TypeError(`fetch failed: ${e}`);
        });
};

// ═══════════════════════════════════════════════════════════════
// 5. TIMERS — REAL async via deno_core WebTimers (tokio-backed)
// ═══════════════════════════════════════════════════════════════
//
// deno_core 0.311 has a built-in WebTimers system (BTreeMap + tokio::time::Sleep)
// that integrates directly with the V8 event loop. Timer callbacks are fired
// by the event loop as true macrotasks (between microtask checkpoints).
//
// API: Deno.core.queueUserTimer(depth, repeat, timeoutMs, callback) -> id
//      Deno.core.cancelTimer(id)
//      Deno.core.getTimerDepth() -> nesting depth
//
// This replaces our old sync op_timer approach (thread::sleep) which blocked
// the V8 thread and couldn't integrate with the async event loop.

const __intervalMaxTicks = ops.op_scheduler_config();

// Capture deno_core timer API before Deno gets deleted (section 9).
const __coreQueueTimer = Deno.core.queueUserTimer;
const __coreCancelTimer = Deno.core.cancelTimer;
const __coreTimerDepth = Deno.core.getTimerDepth;

// Map: our external timer ID -> deno_core internal timer ID
// We maintain our own ID space so page JS can't guess internal IDs.
let __timerNextId = 1;
const __timerMap = new Map();  // externalId -> coreId

// ── Global callback budget ──
// Covers ALL async entrypoints: setTimeout, setInterval, queueMicrotask,
// MessageChannel, requestAnimationFrame, requestIdleCallback, Promise.then.
// Without this, scripts can create infinite microtask storms that hang V8
// (tokio timeout can't interrupt V8 microtasks — only terminate_execution can).
let __callbackBudget = 5000;  // max callbacks — V8 watchdog is the real safety net
let __callbackCount = 0;
let __budgetExhausted = false;
function __checkBudget(source) {
    if (__budgetExhausted) return false;
    __callbackCount++;
    if (__callbackCount > __callbackBudget) {
        __budgetExhausted = true;
        if (typeof __neo_ops !== 'undefined' && __neo_ops.op_console_log) {
            __neo_ops.op_console_log('[BUDGET] callback budget exhausted at ' + __callbackCount + ' (last source: ' + source + ')');
        }
        return false;
    }
    return true;
}
// Reset budget (called on re-navigation)
globalThis.__neo_resetBudget = function() {
    __callbackCount = 0;
    __budgetExhausted = false;
    for (const [extId, coreId] of __timerMap) {
        __coreCancelTimer(coreId);
    }
    __timerMap.clear();
};

// Pending timer count — queried by Rust settle loop to know if work is pending.
globalThis.__neo_pendingTimers = function() { return __timerMap.size; };

// ── Activity tracker for quiescence detection ──
// Every async work unit updates __neo_lastActivity. The Rust settle loop
// queries this to implement a "quiet window" criterion: only declare settled
// if no activity for N ms AND no pending work sources.
let __activityTs = Date.now();    // ms since epoch of last work
let __domMutations = 0;           // mutation count since last reset
let __pendingFetches = 0;         // in-flight fetch count
let __fetchLog = [];              // rich fetch trace log
let __fetchIdCounter = 0;         // monotonic fetch ID
let __pendingModules = 0;         // in-flight module evals (JS-side tracking)
let __schedulerCallbacks = 0;     // pending MessageChannel/rAF/microtask callbacks
let __totalModulesRequested = 0;  // lifetime module request counter
let __totalModulesLoaded = 0;     // lifetime module success counter
let __totalModulesFailed = 0;     // lifetime module failure counter

function __neo_markActivity(source) {
    __activityTs = Date.now();
}

// Module lifecycle tracking — called from Rust via execute_script or
// usable by any JS code that wraps dynamic import().
globalThis.__neo_moduleRequested = function(url) {
    __pendingModules++;
    __totalModulesRequested++;
    __neo_markActivity('module-request');
    neo_trace('[MODULE-JS] requested: ' + url + ' (pending=' + __pendingModules + ')');
};

globalThis.__neo_moduleLoaded = function(url) {
    __pendingModules = Math.max(0, __pendingModules - 1);
    __totalModulesLoaded++;
    __neo_markActivity('module-loaded');
    neo_trace('[MODULE-JS] loaded: ' + url + ' (pending=' + __pendingModules + ')');
};

globalThis.__neo_moduleFailed = function(url, error) {
    __pendingModules = Math.max(0, __pendingModules - 1);
    __totalModulesFailed++;
    __neo_markActivity('module-failed');
    neo_trace('[MODULE-JS] failed: ' + url + ' — ' + (error || 'unknown') + ' (pending=' + __pendingModules + ')');
};

// Module graph stats — Rust calls this for diagnostics.
globalThis.__neo_moduleStats = function() {
    return JSON.stringify({
        pending: __pendingModules,
        total_requested: __totalModulesRequested,
        total_loaded: __totalModulesLoaded,
        total_failed: __totalModulesFailed,
    });
};

// Fetch trace query — returns last 20 entries as JSON.
globalThis.__neo_fetchLog = function() { return JSON.stringify(__fetchLog.slice(-20)); };
// Pending fetch count from trace log (entries with no endMs and no error).
globalThis.__neo_fetchPending = function() { return __fetchLog.filter(function(f) { return !f.endMs && !f.error; }).length; };

// Quiescence query — Rust calls this. Returns JSON with all signals.
globalThis.__neo_quiescence = function() {
    const now = Date.now();
    const tracePending = __fetchLog.filter(function(f) { return !f.endMs && !f.error; }).length;
    return JSON.stringify({
        idle_ms: now - __activityTs,
        pending_timers: __timerMap.size,
        pending_fetches: tracePending,
        pending_modules: __pendingModules,
        dom_mutations: __domMutations,
        callback_count: __callbackCount,
        modules_requested: __totalModulesRequested,
        modules_loaded: __totalModulesLoaded,
        modules_failed: __totalModulesFailed,
        total_fetches: __fetchIdCounter,
    });
};

// Reset quiescence counters (called between settle checks)
globalThis.__neo_resetMutationCount = function() {
    const c = __domMutations;
    __domMutations = 0;
    return c;
};

// Global MutationObserver — tracks ALL DOM mutations for quiescence.
// Framework-agnostic: any DOM change resets the quiet window.
try {
    const __neoMO = new MutationObserver(function(records) {
        __domMutations += records.length;
        __neo_markActivity('mutation');
    });
    // Observe after document.body exists (deferred to after bootstrap completes)
    globalThis.__neo_startMutationWatch = function() {
        if (document.documentElement) {
            __neoMO.observe(document.documentElement, {
                childList: true, subtree: true,
                attributes: true, characterData: true,
            });
        }
    };
} catch(e) {}

// Wrap queueMicrotask with budget
const __origQueueMicrotask = globalThis.queueMicrotask;
globalThis.queueMicrotask = function(fn) {
    if (__budgetExhausted) return;
    __origQueueMicrotask(function() {
        if (__checkBudget('microtask')) {
            __neo_markActivity('microtask');
            try { fn(); } catch(e) {}
        }
    });
};

globalThis.setTimeout = function(fn, ms, ...args) {
    if (typeof fn !== 'function') return 0;
    if (__budgetExhausted) return 0;
    const extId = __timerNextId++;
    const delay = Math.max(0, ms || 0);
    const depth = __coreTimerDepth();
    // queueUserTimer(depth, repeat, timeoutMs, callback) -> coreId
    const coreId = __coreQueueTimer(depth, false, delay, function() {
        __timerMap.delete(extId);
        if (__checkBudget('setTimeout-' + delay)) {
            __neo_markActivity('setTimeout');
            try { fn(...args); } catch(e) {}
        }
    });
    __timerMap.set(extId, coreId);
    return extId;
};
globalThis.clearTimeout = function(id) {
    const coreId = __timerMap.get(id);
    if (coreId !== undefined) {
        __timerMap.delete(id);
        __coreCancelTimer(coreId);
    }
};

globalThis.setInterval = function(fn, ms, ...args) {
    if (typeof fn !== 'function') return 0;
    if (__budgetExhausted) return 0;
    const extId = __timerNextId++;
    const delay = Math.max(1, ms || 1);  // min 1ms for intervals
    const depth = __coreTimerDepth();
    let ticks = 0;
    // queueUserTimer(depth, repeat=true, timeoutMs, callback) -> coreId
    const coreId = __coreQueueTimer(depth, true, delay, function() {
        ticks++;
        if (ticks >= __intervalMaxTicks || __budgetExhausted) {
            // Auto-clear after max ticks or budget exhaustion
            __timerMap.delete(extId);
            __coreCancelTimer(coreId);
            return;
        }
        if (!__checkBudget('setInterval-' + delay)) {
            __timerMap.delete(extId);
            __coreCancelTimer(coreId);
            return;
        }
        __neo_markActivity('setInterval');
        try { fn(...args); } catch(e) {}
    });
    __timerMap.set(extId, coreId);
    return extId;
};
globalThis.clearInterval = function(id) {
    const coreId = __timerMap.get(id);
    if (coreId !== undefined) {
        __timerMap.delete(id);
        __coreCancelTimer(coreId);
    }
};

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
globalThis.requestAnimationFrame = globalThis.requestAnimationFrame || ((fn) => setTimeout(() => { __neo_markActivity('rAF'); fn(performance.now()); }, 16));
globalThis.cancelAnimationFrame = globalThis.cancelAnimationFrame || ((id) => clearTimeout(id));
globalThis.queueMicrotask = globalThis.queueMicrotask || ((fn) => Promise.resolve().then(fn));

// Performance — now() must be relative to page load, NOT Date.now()
// React scheduler uses performance.now() to calculate deadlines.
// If it returns absolute epoch time (~1.7 trillion ms), the scheduler
// thinks every frame has exceeded its deadline and never processes work.
const __perfOrigin = Date.now();
globalThis.performance = globalThis.performance || {};
globalThis.performance.now = globalThis.performance.now || function() { return Date.now() - __perfOrigin; };
globalThis.performance.mark = globalThis.performance.mark || function(){};
globalThis.performance.measure = globalThis.performance.measure || function(){};
globalThis.performance.getEntriesByType = globalThis.performance.getEntriesByType || function(){ return []; };
globalThis.performance.getEntriesByName = globalThis.performance.getEntriesByName || function(){ return []; };
globalThis.performance.timeOrigin = globalThis.performance.timeOrigin || __perfOrigin;

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
globalThis.MutationObserver = __hdWindow.MutationObserver || class { constructor(cb){} observe(){} disconnect(){} takeRecords(){return [];} };
globalThis.IntersectionObserver = class { constructor(cb,opts){} observe(){} unobserve(){} disconnect(){} };
globalThis.ResizeObserver = class { constructor(cb){} observe(){} unobserve(){} disconnect(){} };
globalThis.BroadcastChannel = globalThis.BroadcastChannel || class { constructor(){} postMessage(){} addEventListener(){} close(){} };
globalThis.Worker = globalThis.Worker || class { constructor(){} postMessage(){} addEventListener(){} terminate(){} };

// Event constructors that some libs check for
globalThis.Event = __hdWindow.Event || globalThis.Event || class Event { constructor(t,o={}){this.type=t;this.bubbles=o.bubbles||false;this.cancelable=o.cancelable||false;this.defaultPrevented=false;} preventDefault(){this.defaultPrevented=true;} stopPropagation(){} stopImmediatePropagation(){} };
globalThis.CustomEvent = __hdWindow.CustomEvent || class extends Event { constructor(t,o={}){super(t,o);this.detail=o.detail;} };
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
// MessagePort with actual async message passing.
// Now uses REAL async timers (deno_core WebTimers) so macrotask delivery works.
globalThis.MessagePort = class MessagePort extends EventTarget {
    constructor() { super(); this._other = null; this._closed = false; this.onmessage = null; }
    postMessage(data) {
        if (this._other && !this._other._closed && !__budgetExhausted) {
            const target = this._other;
            const event = new MessageEvent('message', { data });
            // Real macrotask delivery via deno_core WebTimers.
            // setTimeout(fn, 0) now goes through queueUserTimer which is a true
            // async timer polled by the event loop (not microtask, not thread::sleep).
            // delay=1 (not 0) to guarantee delivery in a SEPARATE event loop tick.
            // Real browsers deliver MessageChannel in a distinct macrotask.
            // setTimeout(fn, 0) in deno_core may collapse into the current tick.
            setTimeout(() => {
                if (__checkBudget('MessageChannel')) {
                    __neo_markActivity('MessageChannel');
                    target.dispatchEvent(event);
                    if (target.onmessage) target.onmessage(event);
                }
            }, 1);
        }
    }
    close() { this._closed = true; }
    start() {}
};
// MessageChannel with connected ports.
// Now ENABLED as global — React scheduler detects it and uses it for scheduling.
// With real async timers backed by deno_core WebTimers, the macrotask delivery
// that React needs works correctly. The event loop yields between timer callbacks,
// letting React check performance.now() deadlines.
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
    if (__hdWindow.addEventListener) {
        globalThis.addEventListener = __hdWindow.addEventListener.bind(__hdWindow);
        globalThis.removeEventListener = __hdWindow.removeEventListener.bind(__hdWindow);
        globalThis.dispatchEvent = __hdWindow.dispatchEvent.bind(__hdWindow);
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
            neo_trace('[STREAM] getReader() called');
            const stream = this;
            const reader = {
                get closed() {
                    return stream._state === 'closed' ? Promise.resolve(undefined) :
                           stream._state === 'errored' ? Promise.reject(stream._storedError) :
                           new Promise((resolve) => { /* never resolves until close */ });
                },
                read() {
                    stream._disturbed = true;
                    neo_trace('[STREAM] read() called, remaining: ' + stream._queue.length + ' chunks');
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
            neo_trace('[STREAM] pipeThrough() called with ' + (transform && transform.constructor ? transform.constructor.name : 'unknown'));
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
    // Mark our polyfill so the pipeThrough patch below knows to skip it
    ReadableStream.prototype._neo_polyfill = true;
}

// TextDecoderStream / TextEncoderStream — used by pipeThrough patterns in modern SPAs
if (typeof globalThis.TextDecoderStream === 'undefined' && typeof TransformStream !== 'undefined') {
    globalThis.TextDecoderStream = class TextDecoderStream extends TransformStream {
        constructor(encoding) {
            const _enc = encoding || 'utf-8';
            neo_trace('[STREAM] TextDecoderStream created, encoding=' + _enc);
            const decoder = new TextDecoder(_enc);
            super({
                transform(chunk, controller) {
                    controller.enqueue(decoder.decode(chunk, { stream: true }));
                },
                flush(controller) {
                    const final_ = decoder.decode();
                    if (final_) controller.enqueue(final_);
                }
            });
        }
    };
}
if (typeof globalThis.TextEncoderStream === 'undefined' && typeof TransformStream !== 'undefined') {
    globalThis.TextEncoderStream = class TextEncoderStream extends TransformStream {
        constructor() {
            const encoder = new TextEncoder();
            super({
                transform(chunk, controller) {
                    controller.enqueue(encoder.encode(chunk));
                }
            });
        }
    };
}

// READABLESTREAM PIPETHROUGH PATCH — React Router SSR does
// stream.pipeThrough(new TextEncoderStream()) which creates V8 internal
// pipe promises that block module evaluation. For native ReadableStream,
// override pipeThrough to return self (skip encoding that blocks).
// Our polyfill's pipeThrough already works correctly.
if (typeof ReadableStream !== 'undefined' && !ReadableStream.prototype._neo_polyfill) {
    const _origPipeThrough = ReadableStream.prototype.pipeThrough;
    ReadableStream.prototype.pipeThrough = function(transform, opts) {
        neo_trace('[STREAM] pipeThrough() called (native) with ' + (transform && transform.constructor ? transform.constructor.name : 'unknown'));
        // Use our polyfill behavior for safety — native pipeThrough can create
        // blocking internal pipe promises.
        try {
            this.pipeTo(transform.writable).catch(() => {});
            return transform.readable;
        } catch (e) {
            // Fallback: return self if piping fails
            return this;
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
// 8c. ALERT/CONFIRM/PROMPT — noop stubs for headless rendering
// ═══════════════════════════════════════════════════════════════

// Pages that call alert/confirm/prompt would block in a real browser.
// In headless mode we auto-dismiss: alert is noop, confirm always accepts,
// prompt returns the default value.
globalThis.alert = function(msg) {
    try { ops.op_console_log('[alert] ' + msg); } catch {}
};
globalThis.confirm = function(msg) {
    try { ops.op_console_log('[confirm] ' + msg); } catch {}
    return true; // always accept
};
globalThis.prompt = function(msg, def) {
    try { ops.op_console_log('[prompt] ' + msg); } catch {}
    return def || '';
};

// ═══════════════════════════════════════════════════════════════
// 9. SECURITY BOUNDARY — prevent page JS from escaping sandbox
// ═══════════════════════════════════════════════════════════════

// Block access to runtime internals that page JS should never see.
// Preserve ops reference for browser_shim.js (loaded after bootstrap).
globalThis.__neo_ops = Deno.core.ops;
// Preserve core timer API for internal use (browser_shim.js etc.)
globalThis.__neo_ops.__coreQueueTimer = Deno.core.queueUserTimer;
globalThis.__neo_ops.__coreCancelTimer = Deno.core.cancelTimer;
delete globalThis.Deno;
Object.defineProperty(globalThis, 'process', {
    value: undefined,
    writable: false,
    configurable: false,
});

// Seal (NOT freeze) core prototypes.
// seal = prevents adding/deleting properties, but ALLOWS writing existing ones.
// freeze = seal + makes all properties non-writable → breaks MobX, lodash, etc.
// MobX does: instance.toString = fn (shadowing inherited toString).
// With freeze: fails because inherited toString is non-writable.
// With seal: works because existing properties remain writable.
// NO freeze — real browsers never freeze prototypes.
// Hang protection comes from global callback budget below.

// ═══════════════════════════════════════════════════════════════
// 10. EXPORT — render DOM as HTML for Rust to extract
// ═══════════════════════════════════════════════════════════════

globalThis.__neorender_export = function() {
    return document.documentElement.outerHTML;
};

// Start global MutationObserver for quiescence detection.
if (typeof __neo_startMutationWatch === 'function') __neo_startMutationWatch();

// Mark bootstrap init time as last activity
__neo_markActivity('bootstrap-done');

// NOTE: Promise.allSettled is handled via source-level transform in the Rust
// module loader (v8_runtime.rs). Polyfill injection doesn't work in deno_core
// 0.311 module evaluation contexts.
