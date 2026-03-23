// NeoRender V2 Bootstrap — universal browser environment for AI.
// Connects happy-dom (full DOM) + deno_core ops to create a headless browser.
// Runs AFTER happy-dom.bundle.js. Expects `happydom` global with Window class.
// V2: uses op_fetch/op_console_log + deno_core native WebTimers (not op_neorender_*).

// ═══════════════════════════════════════════════════════════════
// 0. MICROTASK DRAIN — Chromium-compatible.
// ═══════════════════════════════════════════════════════════════
// deno_core 0.311 does NOT drain V8 microtasks between execute_script calls.
// Chromium does via MicrotasksScope(kRunMicrotasks) around script.Run().
//
// Workaround: wrap eval code so __neo_drainMicrotasks() runs AFTER the user
// code but WITHIN the same script execution. V8 with kAuto DOES drain
// microtasks when Script::Run returns. So microtasks from the DRAIN script
// itself will execute. We use this to recursively process pending microtasks
// that were queued by the user code.
//
// The technique: the Rust eval wrapper calls __neo_drainMicrotasks() which
// repeatedly calls a resolved Promise.then() chain to trigger V8's internal
// microtask processing. This is a no-op since we're already inside a script
// execution context — but it forces V8 to notice and process pending
// microtasks before returning to Rust.
//
// NOTE: This is a placeholder. The real fix is upgrading deno_core.
globalThis.__neo_drainMicrotasks = function() {
    // No-op for now. V8's kAuto drains microtasks when the wrapping
    // eval script returns to Rust. Side-effect async work (fetch, timers)
    // is handled by pump_after_interaction which runs the event loop.
};

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
    try { Deno.core.ops.op_console_log('[uncaught] ' + msg + ' @ ' + url + ':' + line + ':' + col + (error?.stack ? ' | ' + error.stack.split('\n')[1] : '')); } catch {}
    return true;
};
globalThis.onunhandledrejection = function(event) {
    try { var r = event?.reason; Deno.core.ops.op_console_log('[unhandled-rejection] ' + (r?.message || r) + (r?.stack ? ' | ' + r.stack.split('\n')[1] : '')); } catch {}
    if (event && event.preventDefault) event.preventDefault();
};

const { ops } = Deno.core;

// Route console.log through Rust op for capture.
const _origConsole = globalThis.console || {};
globalThis.console = {
    log: (...args) => { try { ops.op_console_log(args.map(String).join(' ')); } catch {} },
    warn: (...args) => { try { ops.op_console_log('[warn] ' + args.map(String).join(' ')); } catch {} },
    error: (...args) => { try { const msg = args.map(a => { if (a instanceof Error) return a.message + ' @ ' + (a.stack||'').split('\n').slice(0,3).join(' | '); return String(a); }).join(' '); ops.op_console_log('[error] ' + msg); } catch {} },
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

// CRITICAL: document.defaultView MUST === window for React Router.
// happy-dom sets it to its own Window object. Override immediately.
try { Object.defineProperty(__hdDocument, 'defaultView', { value: globalThis, writable: true, configurable: true }); } catch {}
// Also ensure onpopstate/onhashchange exist on window (React Router checks)
if (!('onpopstate' in globalThis)) globalThis.onpopstate = null;
if (!('onhashchange' in globalThis)) globalThis.onhashchange = null;

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

// ── P0 Navigation invariants ──
// These MUST be correct for React Router and any SPA to navigate.

// 1. document.defaultView === window (React Router checks this)
try {
    Object.defineProperty(document, 'defaultView', {
        value: globalThis, writable: true, configurable: true
    });
} catch {}

// 2. popstate + hashchange support on window
// React Router creates BrowserHistory which calls window.addEventListener('popstate', ...).
// Without these, the router can't detect back/forward navigation.
if (!('onpopstate' in globalThis)) {
    globalThis.onpopstate = null;
}
if (!('onhashchange' in globalThis)) {
    globalThis.onhashchange = null;
}
// PopStateEvent constructor
if (typeof globalThis.PopStateEvent === 'undefined') {
    globalThis.PopStateEvent = class PopStateEvent extends Event {
        constructor(type, init) { super(type, init); this.state = init?.state || null; }
    };
}
// HashChangeEvent constructor
if (typeof globalThis.HashChangeEvent === 'undefined') {
    globalThis.HashChangeEvent = class HashChangeEvent extends Event {
        constructor(type, init) { super(type, init); this.oldURL = init?.oldURL || ''; this.newURL = init?.newURL || ''; }
    };
}
// Ensure history.pushState dispatches popstate (for React Router sync)
// Note: real browsers do NOT dispatch popstate on pushState — only on back/forward.
// But our history.back/forward shim SHOULD dispatch it.

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
    userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/139.0.0.0 Safari/537.36',
    language: 'en-US', languages: ['en-US','en','es'], platform: 'MacIntel',
    cookieEnabled: true, onLine: true, vendor: 'Google Inc.',
    maxTouchPoints: 0, hardwareConcurrency: 8,
    permissions: { query: () => Promise.resolve({state:'granted'}) },
    clipboard: { readText: () => Promise.resolve(''), writeText: () => Promise.resolve() },
    serviceWorker: { register: () => Promise.resolve({}), getRegistrations: () => Promise.resolve([]) },
    sendBeacon: () => true,
};
// Force Chrome UA — happy-dom sets "HappyDOM/x.y.z" which trips bot detection.
// Must match the TLS fingerprint emulation (wreq Chrome 139).
try {
    Object.defineProperty(globalThis.navigator, 'userAgent', {
        value: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/139.0.0.0 Safari/537.36',
        writable: false, configurable: true
    });
    Object.defineProperty(globalThis.navigator, 'appVersion', {
        value: '5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/139.0.0.0 Safari/537.36',
        writable: false, configurable: true
    });
    Object.defineProperty(globalThis.navigator, 'vendor', {
        value: 'Google Inc.',
        writable: false, configurable: true
    });
} catch(e) {}

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
        // Try JS-side cookies first (manually injected via __neorender_cookies).
        const hostname = new URL(url).hostname;
        const parts = [];
        for (const [domain, cookies] of Object.entries(__neorender_cookies)) {
            const d = domain.startsWith('.') ? domain.slice(1) : domain;
            if (hostname === d || hostname.endsWith('.' + d) || hostname === domain) {
                parts.push(cookies);
            }
        }
        if (parts.length > 0) return parts.join('; ');
        // Fallback: read from Rust SQLite cookie store via op.
        if (typeof ops?.op_cookie_get_for_url === 'function') {
            return ops.op_cookie_get_for_url(url);
        }
        return '';
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
        this._sseEvents = init._sseEvents || null; // parsed SSE events from Rust
    }

    get bodyUsed() { return this._bodyUsed; }
    get url() { return this._url; }

    get body() {
        if (!this._stream) {
            neo_trace('[FETCH] response.body accessed for ' + this._url);
            const sseEvents = this._sseEvents;
            if (sseEvents && sseEvents.length > 0) {
                // SSE: deliver each event as a SEPARATE chunk with microtask yields.
                // CRITICAL: Real browsers deliver SSE chunks with network delays.
                // React Router + turbo-stream processes each chunk incrementally,
                // extracting conversation IDs and triggering navigate() from early chunks.
                // Delivering all at once kills this pipeline — navigate() never fires.
                const encoder = new TextEncoder();
                const events = sseEvents.slice();
                neo_trace('[FETCH] SSE body: delivering ' + events.length + ' events incrementally');
                this._stream = new ReadableStream({
                    async pull(controller) {
                        if (events.length === 0) {
                            controller.close();
                            return;
                        }
                        const evt = events.shift();
                        controller.enqueue(encoder.encode('data: ' + evt + '\n\n'));
                        // Yield to event loop between chunks — lets React process each one
                        await new Promise(r => setTimeout(r, 1));
                    }
                });
            } else {
                // Non-SSE: also deliver incrementally if body is large
                const bytes = this._body;
                if (bytes && bytes.length > 65536) {
                    // Large body: deliver in 32KB chunks with yields
                    let offset = 0;
                    const chunkSize = 32768;
                    this._stream = new ReadableStream({
                        async pull(controller) {
                            if (offset >= bytes.length) {
                                controller.close();
                                return;
                            }
                            const end = Math.min(offset + chunkSize, bytes.length);
                            controller.enqueue(bytes.slice(offset, end));
                            offset = end;
                            if (offset < bytes.length) {
                                await new Promise(r => setTimeout(r, 0));
                            }
                        }
                    });
                } else {
                    this._stream = new ReadableStream({
                        start(controller) {
                            if (bytes && bytes.length > 0) controller.enqueue(bytes);
                            controller.close();
                        }
                    });
                }
            }
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
        if (!t || !t.trim()) return null;
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
            { status: this.status, statusText: this.statusText, headers: this._rawHeaders, url: this._url, _text: this._bodyText, redirected: this.redirected, _sseEvents: this._sseEvents }
        );
    }
}
// Make `response instanceof Response` work
globalThis.Response = NeoResponse;

// ── NeoStreamResponse — lazy body via streaming ops ──
// Body is read on demand from Rust via op_fetch_read_chunk.
// Data arrives as UTF-8 text chunks (String::from_utf8_lossy on Rust side).
class NeoStreamResponse {
    constructor(streamId, init) {
        this._streamId = streamId;
        this.status = init.status || 200;
        this.statusText = init.status === 200 ? 'OK' : String(init.status);
        this.ok = this.status >= 200 && this.status < 300;
        this.headers = new Headers(init.headers || {});
        this._rawHeaders = init.headers || {};
        this._url = init.url || '';
        this._bodyUsed = false;
        this._stream = null;
        this._cachedText = null;
        this.redirected = false;
        this.type = 'basic';
    }

    get url() { return this._url; }
    get bodyUsed() { return this._bodyUsed; }

    get body() {
        if (!this._stream) {
            const sid = this._streamId;
            neo_trace('[FETCH] response.body stream accessed for ' + this._url);
            this._stream = new ReadableStream({
                async pull(controller) {
                    try {
                        const chunkJson = await ops.op_fetch_read_chunk(sid);
                        const chunk = JSON.parse(chunkJson);
                        if (chunk.done) {
                            controller.close();
                            return;
                        }
                        if (chunk.error) {
                            controller.error(new Error(chunk.error));
                            return;
                        }
                        // Data is UTF-8 text from Rust (String::from_utf8_lossy)
                        const bytes = new TextEncoder().encode(chunk.data);
                        controller.enqueue(bytes);
                    } catch(e) {
                        controller.error(e);
                    }
                },
                cancel() {
                    try { ops.op_fetch_close(sid); } catch(e) {}
                }
            });
        }
        return this._stream;
    }

    async text() {
        if (this._cachedText !== null) return this._cachedText;
        this._bodyUsed = true;
        // Read directly from stream op to avoid ReadableStream lock issues.
        // If body was already accessed via .body getter, the stream may be locked.
        if (this._streamId != null && !this._stream) {
            // Fast path: read all chunks directly from Rust without ReadableStream
            const chunks = [];
            while (true) {
                const chunkJson = await ops.op_fetch_read_chunk(this._streamId);
                const chunk = JSON.parse(chunkJson);
                if (chunk.done || chunk.error) break;
                chunks.push(chunk.data);
            }
            this._cachedText = chunks.join('');
            return this._cachedText;
        }
        // Slow path: go through ReadableStream (may fail if locked)
        let reader;
        try { reader = this.body.getReader(); }
        catch(e) {
            // Stream locked — try direct op read
            if (this._streamId != null) {
                const chunks = [];
                while (true) {
                    try {
                        const chunkJson = await ops.op_fetch_read_chunk(this._streamId);
                        const chunk = JSON.parse(chunkJson);
                        if (chunk.done || chunk.error) break;
                        chunks.push(chunk.data);
                    } catch(e2) { break; }
                }
                this._cachedText = chunks.join('');
                return this._cachedText;
            }
            throw e;
        }
        const chunks = [];
        while (true) {
            const { done, value } = await reader.read();
            if (done) break;
            chunks.push(value);
        }
        if (chunks.length === 0) {
            this._cachedText = '';
            return '';
        }
        if (chunks.length === 1) {
            this._cachedText = new TextDecoder().decode(chunks[0]);
            return this._cachedText;
        }
        const totalLen = chunks.reduce((a, c) => a + c.length, 0);
        const merged = new Uint8Array(totalLen);
        let offset = 0;
        for (const chunk of chunks) {
            merged.set(chunk, offset);
            offset += chunk.length;
        }
        this._cachedText = new TextDecoder().decode(merged);
        return this._cachedText;
    }

    async json() {
        const t = await this.text();
        if (!t || !t.trim()) return null;
        return JSON.parse(t);
    }

    async arrayBuffer() {
        const t = await this.text();
        return new TextEncoder().encode(t).buffer;
    }

    async blob() {
        return new Blob([await this.arrayBuffer()]);
    }

    clone() {
        // If body was consumed and cached, return a NeoResponse with the text
        if (this._cachedText !== null) {
            const bytes = new TextEncoder().encode(this._cachedText);
            return new NeoResponse(bytes, {
                status: this.status, statusText: this.statusText,
                headers: this._rawHeaders, url: this._url, _text: this._cachedText,
            });
        }
        // Not consumed yet — can't truly clone a stream. Pragmatic: return self.
        return this;
    }
}

// fetch() — streaming: sends request via op_fetch_start, body read lazily.
// Save original ref so scripts that override fetch() (DataDog, Sentry, etc.)
// don't break our internal fetch calls (e.g., __chatgpt_send, sentinel).
globalThis.fetch = async function(input, init) {
    const url = typeof input === 'string' ? input : input?.url || String(input);
    const method = init?.method || (input instanceof Request ? input.method : null) || 'GET';
    const body = init?.body !== undefined ? init.body : (input instanceof Request ? input.body : null);

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
    if (__fetchLog.length > 50) __fetchLog.shift();
    console.log('[FETCH-TRACE] #' + fetchId + ' START ' + method + ' ' + fullUrl.substring(0, 80));
    if (typeof __neo_traceStep === 'function') __neo_traceStep('fetch_start', {url: fullUrl, method: method});

    // Auto-inject cookies
    const hdrs = {};
    const cookies = __getCookiesForUrl(fullUrl);
    if (cookies) hdrs['Cookie'] = cookies;

    // Merge user headers (from init or Request object)
    const headerSrc = init?.headers || (input instanceof Request ? input.headers : null);
    if (headerSrc) {
        if (typeof headerSrc.forEach === 'function') { headerSrc.forEach((v, k) => { hdrs[k] = v; }); }
        else if (Array.isArray(headerSrc)) { headerSrc.forEach(([k, v]) => { hdrs[k] = v; }); }
        else if (typeof headerSrc === 'object') { Object.entries(headerSrc).forEach(([k, v]) => { hdrs[k] = String(v); }); }
    }

    const headersJson = Object.keys(hdrs).length > 0 ? JSON.stringify(hdrs) : '';

    __pendingFetches++;
    __neo_markActivity('fetch-start');

    // Decide: use streaming (op_fetch_start) for SSE, or complete (op_fetch) for normal requests.
    // Streaming keeps the response open as pending async work which prevents the event loop
    // from settling. Only use streaming when the consumer will actually read incrementally.
    const wantsStream = (hdrs['Accept'] || '').includes('text/event-stream');

    try {
        let resp;
        if (wantsStream && typeof ops.op_fetch_start === 'function') {
            // Streaming path: returns headers immediately, body read lazily
            const resultJson = await ops.op_fetch_start(fullUrl, method.toUpperCase(), body || '', headersJson);
            const result = JSON.parse(resultJson);
            __pendingFetches--;
            __neo_markActivity('fetch-end');
            fetchEntry.endMs = Date.now();
            fetchEntry.status = result.status;
            console.log('[FETCH-TRACE] #' + fetchId + ' END status=' + result.status + ' stream=' + result.stream_id + ' (' + (fetchEntry.endMs - fetchEntry.startMs) + 'ms)');
            if (typeof __neo_traceStep === 'function') __neo_traceStep('fetch_end', {url: fullUrl, status: result.status});
            resp = new NeoStreamResponse(result.stream_id, {
                status: result.status,
                headers: result.headers || {},
                url: result.url || fullUrl,
            });
        } else {
            // Complete path: reads entire body, returns NeoResponse (old behavior)
            const resultJson = await ops.op_fetch(fullUrl, method.toUpperCase(), body || '', headersJson);
            const result = JSON.parse(resultJson);
            __pendingFetches--;
            __neo_markActivity('fetch-end');
            fetchEntry.endMs = Date.now();
            fetchEntry.status = result.status;
            console.log('[FETCH-TRACE] #' + fetchId + ' END status=' + result.status + ' (' + (fetchEntry.endMs - fetchEntry.startMs) + 'ms)');
            if (typeof __neo_traceStep === 'function') __neo_traceStep('fetch_end', {url: fullUrl, status: result.status});
            const bodyText = result.body || '';
            const bodyBytes = new TextEncoder().encode(bodyText);
            resp = new NeoResponse(bodyBytes, {
                status: result.status,
                statusText: result.statusText || (result.status === 200 ? 'OK' : String(result.status)),
                headers: result.headers || {},
                url: result.url || fullUrl,
                _text: bodyText,
                _sseEvents: result.sse_events || null,
            });
        }

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
    } catch(e) {
        __pendingFetches--;
        fetchEntry.endMs = Date.now();
        fetchEntry.error = String(e);
        console.log('[FETCH-TRACE] #' + fetchId + ' ERROR ' + e);
        throw new TypeError('fetch failed: ' + e);
    }
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
    if (typeof __neo_traceStep === 'function') __neo_traceStep('timer', 'setTimeout-' + delay);
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
    if (typeof __neo_traceStep === 'function') __neo_traceStep('timer', 'setInterval-' + delay);
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

// PerformanceObserver
if (typeof globalThis.PerformanceObserver === 'undefined') {
    globalThis.PerformanceObserver = class PerformanceObserver {
        constructor(cb) {}
        observe() {}
        disconnect() {}
        takeRecords() { return []; }
        static supportedEntryTypes = [];
    };
}

// Worker stub
if (typeof globalThis.Worker === 'undefined') {
    globalThis.Worker = class Worker extends EventTarget {
        constructor() { super(); }
        postMessage() {}
        terminate() {}
    };
}

// SharedWorker stub
if (typeof globalThis.SharedWorker === 'undefined') {
    globalThis.SharedWorker = class SharedWorker extends EventTarget {
        constructor() { super(); this.port = new MessagePort(); }
    };
}

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
// AbortController + AbortSignal — full implementation (not stub).
// ChatGPT's React Router uses signal.aborted, addEventListener, throwIfAborted, reason.
if (!globalThis.AbortSignal || !globalThis.AbortSignal.prototype?.throwIfAborted) {
    class NeoAbortSignal extends EventTarget {
        constructor() { super(); this._aborted = false; this._reason = undefined; }
        get aborted() { return this._aborted; }
        get reason() { return this._reason; }
        throwIfAborted() { if (this._aborted) throw this._reason; }
        static abort(reason) { const s = new NeoAbortSignal(); s._aborted = true; s._reason = reason || new DOMException('The operation was aborted.', 'AbortError'); return s; }
        static timeout(ms) { const c = new AbortController(); setTimeout(() => c.abort(new DOMException('The operation timed out.', 'TimeoutError')), ms); return c.signal; }
        static any(signals) { const c = new AbortController(); signals.forEach(s => { if (s.aborted) c.abort(s.reason); else s.addEventListener('abort', () => c.abort(s.reason), { once: true }); }); return c.signal; }
    }
    globalThis.AbortSignal = NeoAbortSignal;
    globalThis.AbortController = class NeoAbortController {
        constructor() { this._signal = new NeoAbortSignal(); }
        get signal() { return this._signal; }
        abort(reason) {
            if (this._signal._aborted) return;
            this._signal._aborted = true;
            this._signal._reason = reason || new DOMException('The operation was aborted.', 'AbortError');
            this._signal.dispatchEvent(new Event('abort'));
        }
    };
}
// DOMException — needed by AbortController
if (typeof globalThis.DOMException === 'undefined') {
    globalThis.DOMException = class DOMException extends Error {
        constructor(message, name) { super(message); this.name = name || 'DOMException'; this.code = 0; }
    };
}
// Promise.prototype.finally — ChatGPT code uses .finally() extensively
if (typeof Promise.prototype.finally !== 'function') {
    Promise.prototype.finally = function(onFinally) {
        return this.then(
            value => Promise.resolve(onFinally()).then(() => value),
            reason => Promise.resolve(onFinally()).then(() => { throw reason; })
        );
    };
}
// CompositionEvent — needed for ProseMirror/Tiptap text input
if (typeof globalThis.CompositionEvent === 'undefined') {
    globalThis.CompositionEvent = class CompositionEvent extends Event {
        constructor(type, init) { super(type, init); this.data = init?.data || ''; }
    };
}
// Request — happy-dom's Request crashes ("outside Window context").
// Override with minimal spec-compliant Request that doesn't need Window.
globalThis.Request = class Request {
    constructor(input, init) {
        if (input instanceof Request) {
            this.url = input.url;
            this.method = input.method;
            this.headers = new Headers(input.headers);
            this.body = input.body;
        } else {
            this.url = String(input);
            this.method = init?.method || 'GET';
            this.headers = new Headers(init?.headers || {});
            this.body = init?.body || null;
        }
        this.mode = init?.mode || 'cors';
        this.credentials = init?.credentials || 'same-origin';
        this.cache = init?.cache || 'default';
        this.redirect = init?.redirect || 'follow';
        this.referrer = init?.referrer || '';
        this.signal = init?.signal || (typeof AbortSignal !== 'undefined' ? new AbortController().signal : null);
        this.integrity = init?.integrity || '';
    }
    clone() { return new Request(this); }
    async text() { return typeof this.body === 'string' ? this.body : ''; }
    async json() { return JSON.parse(await this.text()); }
    async arrayBuffer() { return new TextEncoder().encode(await this.text()).buffer; }
};
// CSS.supports — containerQuery polyfill check uses this
if (typeof globalThis.CSS === 'undefined') {
    globalThis.CSS = { supports: () => false, escape: (s) => s };
}
// structuredClone — used by React Router for state management
if (typeof globalThis.structuredClone === 'undefined') {
    globalThis.structuredClone = function(obj) { return JSON.parse(JSON.stringify(obj)); };
}
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
        const bytes = data instanceof ArrayBuffer ? new Uint8Array(data) : new Uint8Array(data.buffer || data);
        const name = typeof algo === 'string' ? algo : algo?.name || 'SHA-256';
        if (name === 'SHA-512') return _sha512(bytes).buffer;
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
    // ── SHA-512 (pure JS) — needed for HMAC-SHA-512 (ChatGPT) ──
    const _sha512 = (function() {
        // 64-bit math via pairs of 32-bit ints [hi, lo]
        function add64(a,b){var lo=(a[1]>>>0)+(b[1]>>>0),hi=(a[0]>>>0)+(b[0]>>>0)+((lo/0x100000000)>>>0);return[hi>>>0,lo>>>0]}
        function rotr64(v,n){if(n<32)return[(v[0]>>>n|v[1]<<(32-n))>>>0,(v[1]>>>n|v[0]<<(32-n))>>>0];return[(v[1]>>>(n-32)|v[0]<<(64-n))>>>0,(v[0]>>>(n-32)|v[1]<<(64-n))>>>0]}
        function shr64(v,n){if(n<32)return[(v[0]>>>n)>>>0,(v[1]>>>n|v[0]<<(32-n))>>>0];return[0,(v[0]>>>(n-32))>>>0]}
        function xor64(a,b){return[(a[0]^b[0])>>>0,(a[1]^b[1])>>>0]}
        function and64(a,b){return[(a[0]&b[0])>>>0,(a[1]&b[1])>>>0]}
        function not64(a){return[(~a[0])>>>0,(~a[1])>>>0]}
        const K=[[0x428a2f98,0xd728ae22],[0x71374491,0x23ef65cd],[0xb5c0fbcf,0xec4d3b2f],[0xe9b5dba5,0x8189dbbc],[0x3956c25b,0xf348b538],[0x59f111f1,0xb605d019],[0x923f82a4,0xaf194f9b],[0xab1c5ed5,0xda6d8118],[0xd807aa98,0xa3030242],[0x12835b01,0x45706fbe],[0x243185be,0x4ee4b28c],[0x550c7dc3,0xd5ffb4e2],[0x72be5d74,0xf27b896f],[0x80deb1fe,0x3b1696b1],[0x9bdc06a7,0x25c71235],[0xc19bf174,0xcf692694],[0xe49b69c1,0x9ef14ad2],[0xefbe4786,0x384f25e3],[0x0fc19dc6,0x8b8cd5b5],[0x240ca1cc,0x77ac9c65],[0x2de92c6f,0x592b0275],[0x4a7484aa,0x6ea6e483],[0x5cb0a9dc,0xbd41fbd4],[0x76f988da,0x831153b5],[0x983e5152,0xee66dfab],[0xa831c66d,0x2db43210],[0xb00327c8,0x98fb213f],[0xbf597fc7,0xbeef0ee4],[0xc6e00bf3,0x3da88fc2],[0xd5a79147,0x930aa725],[0x06ca6351,0xe003826f],[0x14292967,0x0a0e6e70],[0x27b70a85,0x46d22ffc],[0x2e1b2138,0x5c26c926],[0x4d2c6dfc,0x5ac42aed],[0x53380d13,0x9d95b3df],[0x650a7354,0x8baf63de],[0x766a0abb,0x3c77b2a8],[0x81c2c92e,0x47edaee6],[0x92722c85,0x1482353b],[0xa2bfe8a1,0x4cf10364],[0xa81a664b,0xbc423001],[0xc24b8b70,0xd0f89791],[0xc76c51a3,0x0654be30],[0xd192e819,0xd6ef5218],[0xd6990624,0x5565a910],[0xf40e3585,0x5771202a],[0x106aa070,0x32bbd1b8],[0x19a4c116,0xb8d2d0c8],[0x1e376c08,0x5141ab53],[0x2748774c,0xdf8eeb99],[0x34b0bcb5,0xe19b48a8],[0x391c0cb3,0xc5c95a63],[0x4ed8aa4a,0xe3418acb],[0x5b9cca4f,0x7763e373],[0x682e6ff3,0xd6b2b8a3],[0x748f82ee,0x5defb2fc],[0x78a5636f,0x43172f60],[0x84c87814,0xa1f0ab72],[0x8cc70208,0x1a6439ec],[0x90befffa,0x23631e28],[0xa4506ceb,0xde82bde9],[0xbef9a3f7,0xb2c67915],[0xc67178f2,0xe372532b],[0xca273ece,0xea26619c],[0xd186b8c7,0x21c0c207],[0xeada7dd6,0xcde0eb1e],[0xf57d4f7f,0xee6ed178],[0x06f067aa,0x72176fba],[0x0a637dc5,0xa2c898a6],[0x113f9804,0xbef90dae],[0x1b710b35,0x131c471b],[0x28db77f5,0x23047d84],[0x32caab7b,0x40c72493],[0x3c9ebe0a,0x15c9bebc],[0x431d67c4,0x9c100d4c],[0x4cc5d4be,0xcb3e42b6],[0x597f299c,0xfc657e2a],[0x5fcb6fab,0x3ad6faec],[0x6c44198c,0x4a475817]];
        const H0=[[0x6a09e667,0xf3bcc908],[0xbb67ae85,0x84caa73b],[0x3c6ef372,0xfe94f82b],[0xa54ff53a,0x5f1d36f1],[0x510e527f,0xade682d1],[0x9b05688c,0x2b3e6c1f],[0x1f83d9ab,0xfb41bd6b],[0x5be0cd19,0x137e2179]];
        return function(msg){
            var bytes=typeof msg==='string'?new TextEncoder().encode(msg):new Uint8Array(msg);
            var len=bytes.length,bitLen=len*8;
            var padLen=Math.ceil((len+17)/128)*128;
            var padded=new Uint8Array(padLen);padded.set(bytes);padded[len]=0x80;
            var dv=new DataView(padded.buffer);dv.setUint32(padLen-4,bitLen,false);
            var h=H0.map(function(x){return x.slice()});
            for(var off=0;off<padLen;off+=128){
                var w=[];
                for(var i=0;i<16;i++)w[i]=[dv.getUint32(off+i*8,false),dv.getUint32(off+i*8+4,false)];
                for(var i=16;i<80;i++){
                    var s0=xor64(xor64(rotr64(w[i-15],1),rotr64(w[i-15],8)),shr64(w[i-15],7));
                    var s1=xor64(xor64(rotr64(w[i-2],19),rotr64(w[i-2],61)),shr64(w[i-2],6));
                    w[i]=add64(add64(add64(w[i-16],s0),w[i-7]),s1);
                }
                var a=h[0],b=h[1],c=h[2],d=h[3],e=h[4],f=h[5],g=h[6],hh=h[7];
                for(var i=0;i<80;i++){
                    var S1=xor64(xor64(rotr64(e,14),rotr64(e,18)),rotr64(e,41));
                    var ch=xor64(and64(e,f),and64(not64(e),g));
                    var t1=add64(add64(add64(add64(hh,S1),ch),K[i]),w[i]);
                    var S0=xor64(xor64(rotr64(a,28),rotr64(a,34)),rotr64(a,39));
                    var maj=xor64(xor64(and64(a,b),and64(a,c)),and64(b,c));
                    var t2=add64(S0,maj);
                    hh=g;g=f;f=e;e=add64(d,t1);d=c;c=b;b=a;a=add64(t1,t2);
                }
                for(var i=0;i<8;i++)h[i]=add64(h[i],[a,b,c,d,e,f,g,hh][i]);
            }
            var result=new Uint8Array(64);var rv=new DataView(result.buffer);
            for(var i=0;i<8;i++){rv.setUint32(i*8,h[i][0],false);rv.setUint32(i*8+4,h[i][1],false)}
            return result;
        };
    })();

    // ── HMAC (SHA-256/SHA-512) ──
    function _hmac(hashFn, blockSize, keyBytes, msgBytes) {
        var key = new Uint8Array(keyBytes);
        if (key.length > blockSize) key = hashFn(key);
        var padded = new Uint8Array(blockSize);
        padded.set(key);
        var ipad = new Uint8Array(blockSize), opad = new Uint8Array(blockSize);
        for (var i = 0; i < blockSize; i++) { ipad[i] = padded[i] ^ 0x36; opad[i] = padded[i] ^ 0x5c; }
        var inner = new Uint8Array(blockSize + msgBytes.length);
        inner.set(ipad); inner.set(new Uint8Array(msgBytes), blockSize);
        var innerHash = hashFn(inner);
        var outer = new Uint8Array(blockSize + innerHash.length);
        outer.set(opad); outer.set(innerHash, blockSize);
        return hashFn(outer);
    }

    // ── CryptoKey model ──
    class NeoCryptoKey {
        constructor(type, extractable, algorithm, usages, raw) {
            this.type = type;
            this.extractable = extractable;
            this.algorithm = algorithm;
            this.usages = usages;
            this._raw = raw; // Uint8Array
        }
    }

    // ── crypto.subtle methods (real HMAC-SHA-256/512) ──
    function _getHash(algo) {
        var name = typeof algo === 'string' ? algo : algo?.hash?.name || algo?.hash || algo?.name || 'SHA-256';
        if (name === 'SHA-256' || name === 'SHA-256') return { fn: _sha256, size: 32, block: 64, name: 'SHA-256' };
        if (name === 'SHA-512') return { fn: _sha512, size: 64, block: 128, name: 'SHA-512' };
        return { fn: _sha256, size: 32, block: 64, name: 'SHA-256' };
    }

    globalThis.crypto.subtle.generateKey = async function(algo, extractable, usages) {
        var h = _getHash(algo);
        var raw = new Uint8Array(h.block);
        crypto.getRandomValues(raw);
        return new NeoCryptoKey('secret', extractable, { name: algo?.name || 'HMAC', hash: { name: h.name } }, usages, raw);
    };

    globalThis.crypto.subtle.importKey = async function(format, keyData, algo, extractable, usages) {
        var raw;
        if (format === 'raw') {
            raw = new Uint8Array(keyData instanceof ArrayBuffer ? keyData : keyData.buffer || keyData);
        } else if (format === 'jwk') {
            // JWK: decode k field (base64url)
            var k = keyData.k || '';
            var b64 = k.replace(/-/g, '+').replace(/_/g, '/');
            while (b64.length % 4) b64 += '=';
            raw = new Uint8Array(atob(b64).split('').map(function(c) { return c.charCodeAt(0); }));
        } else {
            raw = new Uint8Array(64);
        }
        return new NeoCryptoKey('secret', extractable, { name: algo?.name || 'HMAC', hash: { name: _getHash(algo).name } }, usages, raw);
    };

    globalThis.crypto.subtle.exportKey = async function(format, key) {
        if (!(key instanceof NeoCryptoKey)) return new ArrayBuffer(0);
        if (format === 'raw') return key._raw.buffer.slice(0);
        if (format === 'jwk') {
            var b64 = btoa(String.fromCharCode.apply(null, key._raw)).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
            return { kty: 'oct', k: b64, alg: 'HS' + (_getHash(key.algorithm).size * 8), ext: key.extractable, key_ops: key.usages };
        }
        return key._raw.buffer.slice(0);
    };

    globalThis.crypto.subtle.sign = async function(algo, key, data) {
        // Prefer key's stored algorithm (has hash info), fall back to algo param
        var h = _getHash(key?.algorithm?.hash || algo?.hash || key?.algorithm || algo);
        var raw = (key instanceof NeoCryptoKey) ? key._raw : new Uint8Array(h.block);
        var msg = data instanceof ArrayBuffer ? new Uint8Array(data) : new Uint8Array(data.buffer || data);
        return _hmac(h.fn, h.block, raw, msg).buffer;
    };

    globalThis.crypto.subtle.verify = async function(algo, key, signature, data) {
        // sign() already reads from key.algorithm, just delegate
        var signed = await globalThis.crypto.subtle.sign(algo, key, data);
        var a = new Uint8Array(signed), b = new Uint8Array(signature instanceof ArrayBuffer ? signature : signature.buffer || signature);
        if (a.length !== b.length) return false;
        for (var i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
        return true;
    };

    globalThis.crypto.subtle.encrypt = async function() { return new ArrayBuffer(32); };
    globalThis.crypto.subtle.decrypt = async function() { return new ArrayBuffer(0); };
    globalThis.crypto.subtle.deriveBits = async function() { return new ArrayBuffer(32); };
    globalThis.crypto.subtle.deriveKey = async function() { return new NeoCryptoKey('secret', true, {name:'HMAC'}, ['sign'], new Uint8Array(64)); };
}

// ═══════════════════════════════════════════════════════════════
// 7c. EVENTSOURCE — SSE polyfill using fetch() with SSE parsing
// ═══════════════════════════════════════════════════════════════

// EventSource polyfill — ChatGPT and other SPAs use EventSource or
// fetch+ReadableStream for Server-Sent Events. This uses our fetch()
// which now returns structured sse_events from Rust-side parsing.
globalThis.EventSource = globalThis.EventSource || class EventSource extends EventTarget {
    constructor(url, options) {
        super();
        this.url = url;
        this.readyState = 0; // CONNECTING
        this.withCredentials = options?.withCredentials || false;
        this.onopen = null;
        this.onmessage = null;
        this.onerror = null;
        this._connect();
    }

    _connect() {
        this.readyState = 1; // OPEN
        const openEvt = new Event('open');
        this.dispatchEvent(openEvt);
        if (this.onopen) this.onopen(openEvt);

        fetch(this.url, {
            headers: { 'Accept': 'text/event-stream', 'Cache-Control': 'no-cache' }
        }).then(resp => {
            const body = resp._bodyText || '';
            // Parse SSE events from raw body
            const rawEvents = body.split('\n\n');
            for (const raw of rawEvents) {
                if (!raw.trim()) continue;
                const lines = raw.split('\n');
                let data = '', eventType = 'message', id = '';
                for (const line of lines) {
                    if (line.startsWith('data: ')) data += (data ? '\n' : '') + line.slice(6);
                    else if (line.startsWith('data:')) data += (data ? '\n' : '') + line.slice(5);
                    else if (line.startsWith('event: ')) eventType = line.slice(7);
                    else if (line.startsWith('event:')) eventType = line.slice(6);
                    else if (line.startsWith('id: ')) id = line.slice(4);
                    else if (line.startsWith('id:')) id = line.slice(3);
                }
                if (!data || data === '[DONE]') continue;
                const evt = new MessageEvent(eventType, { data, lastEventId: id });
                this.dispatchEvent(evt);
                if (eventType === 'message' && this.onmessage) this.onmessage(evt);
                // For named events, also fire onmessage as fallback
                if (eventType !== 'message' && this.onmessage) this.onmessage(evt);
            }
            this.readyState = 2; // CLOSED
        }).catch(err => {
            this.readyState = 2;
            const errEvt = new Event('error');
            this.dispatchEvent(errEvt);
            if (this.onerror) this.onerror(errEvt);
        });
    }

    close() { this.readyState = 2; }

    static get CONNECTING() { return 0; }
    static get OPEN() { return 1; }
    static get CLOSED() { return 2; }
};

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
// Save original fetch so monitoring wrappers (DataDog, Sentry) don't break
// internal API calls. Use __neo_fetch() to bypass instrumentation.
globalThis.__neo_fetch = globalThis.fetch;

__neo_markActivity('bootstrap-done');

// NOTE: Promise.allSettled is handled via source-level transform in the Rust
// module loader (v8_runtime.rs). Polyfill injection doesn't work in deno_core
// 0.311 module evaluation contexts.
