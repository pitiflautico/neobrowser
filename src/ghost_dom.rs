//! Ghost DOM — minimal browser DOM for JS execution.
//!
//! Provides document/window/navigator APIs so that SPAs can boot.
//! No rendering, no layout, no paint. Just the DOM tree that JS manipulates,
//! then we read it as a map.

use boa_engine::prelude::*;
use boa_engine::{js_string, JsValue, Context, Source, JsResult, JsArgs};
use boa_engine::object::builtins::JsFunction;
use boa_engine::property::Attribute;
use boa_engine::native_function::NativeFunction;

/// The JS source that defines our minimal DOM environment.
/// This runs BEFORE any page scripts.
pub fn dom_polyfill() -> &'static str {
    r#"
    // ─── Minimal DOM for SPAs ───

    // Node storage — all created elements live here
    const __nodes = new Map();
    let __nextId = 1;

    class EventTarget {
        constructor() {
            this.__listeners = {};
        }
        addEventListener(type, fn, opts) {
            if (!this.__listeners[type]) this.__listeners[type] = [];
            this.__listeners[type].push(fn);
        }
        removeEventListener(type, fn) {
            if (!this.__listeners[type]) return;
            this.__listeners[type] = this.__listeners[type].filter(f => f !== fn);
        }
        dispatchEvent(evt) {
            const listeners = this.__listeners[evt.type] || [];
            for (const fn of listeners) {
                try { fn(evt); } catch(e) {}
            }
            return true;
        }
    }

    class Node extends EventTarget {
        constructor(nodeType, nodeName) {
            super();
            this.__id = __nextId++;
            this.nodeType = nodeType;
            this.nodeName = nodeName;
            this.childNodes = [];
            this.parentNode = null;
            this.ownerDocument = typeof document !== 'undefined' ? document : null;
            __nodes.set(this.__id, this);
        }
        get firstChild() { return this.childNodes[0] || null; }
        get lastChild() { return this.childNodes[this.childNodes.length - 1] || null; }
        get nextSibling() {
            if (!this.parentNode) return null;
            const idx = this.parentNode.childNodes.indexOf(this);
            return this.parentNode.childNodes[idx + 1] || null;
        }
        get previousSibling() {
            if (!this.parentNode) return null;
            const idx = this.parentNode.childNodes.indexOf(this);
            return this.parentNode.childNodes[idx - 1] || null;
        }
        appendChild(child) {
            if (child.parentNode) child.parentNode.removeChild(child);
            child.parentNode = this;
            this.childNodes.push(child);
            return child;
        }
        removeChild(child) {
            const idx = this.childNodes.indexOf(child);
            if (idx >= 0) {
                this.childNodes.splice(idx, 1);
                child.parentNode = null;
            }
            return child;
        }
        insertBefore(newChild, refChild) {
            if (newChild.parentNode) newChild.parentNode.removeChild(newChild);
            const idx = refChild ? this.childNodes.indexOf(refChild) : -1;
            if (idx >= 0) {
                this.childNodes.splice(idx, 0, newChild);
            } else {
                this.childNodes.push(newChild);
            }
            newChild.parentNode = this;
            return newChild;
        }
        replaceChild(newChild, oldChild) {
            const idx = this.childNodes.indexOf(oldChild);
            if (idx >= 0) {
                if (newChild.parentNode) newChild.parentNode.removeChild(newChild);
                this.childNodes[idx] = newChild;
                newChild.parentNode = this;
                oldChild.parentNode = null;
            }
            return oldChild;
        }
        cloneNode(deep) {
            if (this instanceof Element) {
                const clone = new Element(this.tagName);
                for (const [k, v] of Object.entries(this.__attrs)) clone.__attrs[k] = v;
                if (deep) {
                    for (const child of this.childNodes) {
                        clone.appendChild(child.cloneNode(true));
                    }
                }
                return clone;
            }
            if (this instanceof Text) return new Text(this.textContent);
            return new Node(this.nodeType, this.nodeName);
        }
        contains(other) {
            if (this === other) return true;
            for (const child of this.childNodes) {
                if (child.contains(other)) return true;
            }
            return false;
        }
        get textContent() {
            if (this instanceof Text) return this.__text;
            return this.childNodes.map(c => c.textContent).join('');
        }
        set textContent(val) {
            if (this instanceof Text) { this.__text = val; return; }
            this.childNodes = [];
            if (val) this.appendChild(new Text(val));
        }
    }

    class Text extends Node {
        constructor(text) {
            super(3, '#text');
            this.__text = text || '';
        }
        get textContent() { return this.__text; }
        set textContent(val) { this.__text = val; }
        get data() { return this.__text; }
        set data(val) { this.__text = val; }
        get nodeValue() { return this.__text; }
    }

    class Comment extends Node {
        constructor(text) { super(8, '#comment'); this.__text = text || ''; }
    }

    class Element extends Node {
        constructor(tagName) {
            super(1, tagName.toUpperCase());
            this.tagName = tagName.toUpperCase();
            this.__attrs = {};
            this.__style = {};
            this.__classList = [];
            this.id = '';
        }
        getAttribute(name) { return this.__attrs[name] !== undefined ? this.__attrs[name] : null; }
        setAttribute(name, value) {
            this.__attrs[name] = String(value);
            if (name === 'id') this.id = String(value);
            if (name === 'class') this.__classList = String(value).split(/\s+/).filter(Boolean);
        }
        removeAttribute(name) { delete this.__attrs[name]; }
        hasAttribute(name) { return name in this.__attrs; }
        get className() { return this.__classList.join(' '); }
        set className(val) { this.__classList = val.split(/\s+/).filter(Boolean); this.__attrs['class'] = val; }
        get classList() {
            const self = this;
            return {
                add(...cls) { for (const c of cls) if (!self.__classList.includes(c)) self.__classList.push(c); },
                remove(...cls) { self.__classList = self.__classList.filter(c => !cls.includes(c)); },
                toggle(c) { if (self.__classList.includes(c)) self.classList.remove(c); else self.classList.add(c); },
                contains(c) { return self.__classList.includes(c); },
                get length() { return self.__classList.length; },
                item(i) { return self.__classList[i] || null; },
            };
        }
        get style() {
            const self = this;
            return new Proxy(self.__style, {
                get(t, p) { return t[p] || ''; },
                set(t, p, v) { t[p] = v; return true; },
            });
        }
        get innerHTML() {
            return this.childNodes.map(c => {
                if (c instanceof Text) return c.__text;
                if (c instanceof Element) return c.outerHTML;
                return '';
            }).join('');
        }
        set innerHTML(html) {
            this.childNodes = [];
            // Simple: just set as text. Full HTML parsing would need a parser.
            if (html) this.appendChild(new Text(html));
        }
        get outerHTML() {
            const attrs = Object.entries(this.__attrs).map(([k,v]) => ` ${k}="${v}"`).join('');
            const inner = this.innerHTML;
            const tag = this.tagName.toLowerCase();
            if (['br','hr','img','input','meta','link'].includes(tag)) return `<${tag}${attrs}>`;
            return `<${tag}${attrs}>${inner}</${tag}>`;
        }
        get children() { return this.childNodes.filter(c => c instanceof Element); }
        get firstElementChild() { return this.children[0] || null; }
        get lastElementChild() { const ch = this.children; return ch[ch.length-1] || null; }
        get nextElementSibling() {
            if (!this.parentNode) return null;
            const siblings = this.parentNode.children;
            const idx = siblings.indexOf(this);
            return siblings[idx + 1] || null;
        }
        get previousElementSibling() {
            if (!this.parentNode) return null;
            const siblings = this.parentNode.children;
            const idx = siblings.indexOf(this);
            return siblings[idx - 1] || null;
        }

        // ─── Query selectors (simplified) ───
        querySelectorAll(sel) { return __querySelectorAll(this, sel); }
        querySelector(sel) { return this.querySelectorAll(sel)[0] || null; }
        getElementsByTagName(tag) {
            tag = tag.toUpperCase();
            if (tag === '*') return __collectAll(this);
            return __collectAll(this).filter(e => e.tagName === tag);
        }
        getElementsByClassName(cls) {
            return __collectAll(this).filter(e => e.__classList.includes(cls));
        }
        getElementById(id) {
            return __collectAll(this).find(e => e.id === id) || null;
        }

        // Layout stubs — no rendering, return zeros
        getBoundingClientRect() {
            return {top:0, left:0, right:0, bottom:0, width:0, height:0, x:0, y:0};
        }
        get offsetWidth() { return 0; }
        get offsetHeight() { return 0; }
        get scrollWidth() { return 0; }
        get scrollHeight() { return 0; }
        get clientWidth() { return 1440; }
        get clientHeight() { return 900; }

        // Focus stubs
        focus() {}
        blur() {}
        click() { this.dispatchEvent({type: 'click', target: this}); }
    }

    // HTMLFormElement
    class HTMLFormElement extends Element {
        constructor() { super('form'); }
        submit() {}
        reset() { /* noop */ }
        get elements() { return this.querySelectorAll('input, select, textarea, button'); }
    }

    // ─── Simple query selector engine ───
    function __collectAll(root) {
        const result = [];
        function walk(node) {
            if (node instanceof Element) result.push(node);
            for (const child of (node.childNodes || [])) walk(child);
        }
        for (const child of (root.childNodes || [])) walk(child);
        return result;
    }

    function __querySelectorAll(root, sel) {
        const all = __collectAll(root);
        // Simple selector matching: tag, .class, #id, [attr], tag.class, tag#id
        return all.filter(el => __matchesSelector(el, sel));
    }

    function __matchesSelector(el, sel) {
        // Handle comma-separated selectors
        if (sel.includes(',')) return sel.split(',').some(s => __matchesSelector(el, s.trim()));

        // Handle descendant selectors (space-separated)
        const parts = sel.trim().split(/\s+/);
        if (parts.length > 1) {
            // Last part must match the element
            if (!__matchesSingle(el, parts[parts.length - 1])) return false;
            // Check ancestors match remaining parts
            let current = el.parentNode;
            for (let i = parts.length - 2; i >= 0; i--) {
                while (current && !__matchesSingle(current, parts[i])) {
                    current = current.parentNode;
                }
                if (!current) return false;
                current = current.parentNode;
            }
            return true;
        }
        return __matchesSingle(el, sel);
    }

    function __matchesSingle(el, sel) {
        if (!(el instanceof Element)) return false;
        sel = sel.trim();
        // #id
        if (sel.startsWith('#')) return el.id === sel.slice(1);
        // .class
        if (sel.startsWith('.')) return el.__classList.includes(sel.slice(1));
        // [attr]
        if (sel.startsWith('[')) {
            const m = sel.match(/\[([^=\]]+)(?:=["']?([^"'\]]*)["']?)?\]/);
            if (m) return m[2] !== undefined ? el.__attrs[m[1]] === m[2] : m[1] in el.__attrs;
        }
        // tag.class or tag#id
        if (sel.includes('.')) {
            const [tag, ...classes] = sel.split('.');
            if (tag && el.tagName !== tag.toUpperCase()) return false;
            return classes.every(c => el.__classList.includes(c));
        }
        if (sel.includes('#')) {
            const [tag, id] = sel.split('#');
            if (tag && el.tagName !== tag.toUpperCase()) return false;
            return el.id === id;
        }
        // tag
        return el.tagName === sel.toUpperCase();
    }

    // ─── Document ───
    class Document extends Node {
        constructor() {
            super(9, '#document');
            this.documentElement = new Element('html');
            this.head = new Element('head');
            this.body = new Element('body');
            this.documentElement.appendChild(this.head);
            this.documentElement.appendChild(this.body);
            this.appendChild(this.documentElement);
        }
        createElement(tag) {
            if (tag.toLowerCase() === 'form') return new HTMLFormElement();
            return new Element(tag);
        }
        createTextNode(text) { return new Text(text); }
        createComment(text) { return new Comment(text); }
        createDocumentFragment() { return new Node(11, '#document-fragment'); }
        createElementNS(ns, tag) { return this.createElement(tag); }
        getElementById(id) { return this.documentElement.getElementById(id); }
        getElementsByTagName(tag) { return this.documentElement.getElementsByTagName(tag); }
        getElementsByClassName(cls) { return this.documentElement.getElementsByClassName(cls); }
        querySelector(sel) { return this.documentElement.querySelector(sel); }
        querySelectorAll(sel) { return this.documentElement.querySelectorAll(sel); }
        get title() {
            const t = this.head.querySelector('title');
            return t ? t.textContent : '';
        }
        set title(val) {
            let t = this.head.querySelector('title');
            if (!t) { t = this.createElement('title'); this.head.appendChild(t); }
            t.textContent = val;
        }
        get cookie() { return ''; }
        set cookie(val) { /* cookie jar handled in Rust */ }
    }

    // ─── Window / Navigator / Location ───

    const document = new Document();

    const location = {
        href: '',
        protocol: 'https:',
        host: '',
        hostname: '',
        port: '',
        pathname: '/',
        search: '',
        hash: '',
        origin: '',
        assign(url) { location.href = url; },
        replace(url) { location.href = url; },
        reload() {},
        toString() { return location.href; },
    };

    const navigator = {
        userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36',
        language: 'en-US',
        languages: ['en-US', 'en', 'es'],
        platform: 'MacIntel',
        cookieEnabled: true,
        onLine: true,
        vendor: 'Google Inc.',
        maxTouchPoints: 0,
        hardwareConcurrency: 8,
        deviceMemory: 8,
        connection: { effectiveType: '4g', downlink: 10, rtt: 50 },
        permissions: { query: () => Promise.resolve({state: 'granted'}) },
        clipboard: { readText: () => Promise.resolve(''), writeText: () => Promise.resolve() },
        mediaDevices: { enumerateDevices: () => Promise.resolve([]) },
        serviceWorker: { register: () => Promise.resolve({}), getRegistrations: () => Promise.resolve([]) },
        sendBeacon: () => true,
        geolocation: { getCurrentPosition: (ok) => ok({coords:{latitude:0,longitude:0}}) },
    };

    const screen = {
        width: 1440, height: 900,
        availWidth: 1440, availHeight: 875,
        colorDepth: 24, pixelDepth: 24,
    };

    const history = {
        length: 1,
        state: null,
        pushState(state, title, url) { if (url) location.href = url; history.length++; },
        replaceState(state, title, url) { if (url) location.href = url; },
        back() {}, forward() {}, go() {},
    };

    // Timers
    const __timers = { next: 1, pending: {} };

    function setTimeout(fn, ms, ...args) {
        const id = __timers.next++;
        __timers.pending[id] = { fn, ms: ms || 0, args, type: 'timeout' };
        return id;
    }
    function clearTimeout(id) { delete __timers.pending[id]; }
    function setInterval(fn, ms, ...args) {
        const id = __timers.next++;
        __timers.pending[id] = { fn, ms: ms || 0, args, type: 'interval' };
        return id;
    }
    function clearInterval(id) { delete __timers.pending[id]; }
    function requestAnimationFrame(fn) { return setTimeout(fn, 16); }
    function cancelAnimationFrame(id) { clearTimeout(id); }
    function queueMicrotask(fn) { Promise.resolve().then(fn); }

    // ─── Fetch stub (will be bridged to Rust HTTP) ───
    function fetch(url, opts) {
        // Store fetch calls for Rust to intercept
        if (!globalThis.__fetchQueue) globalThis.__fetchQueue = [];
        return new Promise((resolve, reject) => {
            globalThis.__fetchQueue.push({ url: String(url), opts: opts || {}, resolve, reject });
        });
    }

    // XMLHttpRequest minimal
    class XMLHttpRequest extends EventTarget {
        constructor() {
            super();
            this.readyState = 0;
            this.status = 0;
            this.responseText = '';
            this.response = '';
        }
        open(method, url) { this.__method = method; this.__url = url; this.readyState = 1; }
        setRequestHeader() {}
        send(body) {
            // Queue for Rust
            if (!globalThis.__xhrQueue) globalThis.__xhrQueue = [];
            globalThis.__xhrQueue.push({ method: this.__method, url: this.__url, body, xhr: this });
        }
        abort() {}
    }

    // ─── Storage ───
    class Storage {
        constructor() { this.__data = {}; }
        getItem(key) { return this.__data[key] !== undefined ? this.__data[key] : null; }
        setItem(key, value) { this.__data[key] = String(value); }
        removeItem(key) { delete this.__data[key]; }
        clear() { this.__data = {}; }
        get length() { return Object.keys(this.__data).length; }
        key(i) { return Object.keys(this.__data)[i] || null; }
    }

    const localStorage = new Storage();
    const sessionStorage = new Storage();

    // ─── Console ───
    const console = {
        log: (...args) => {},
        warn: (...args) => {},
        error: (...args) => {},
        info: (...args) => {},
        debug: (...args) => {},
        trace: (...args) => {},
        dir: (...args) => {},
        table: (...args) => {},
        group: () => {}, groupEnd: () => {},
        time: () => {}, timeEnd: () => {},
        assert: () => {},
        clear: () => {},
        count: () => {}, countReset: () => {},
    };

    // ─── Window (self reference) ───
    const window = globalThis;
    const self = globalThis;

    // Expose everything on globalThis
    Object.assign(globalThis, {
        window: globalThis,
        self: globalThis,
        document, location, navigator, screen, history,
        localStorage, sessionStorage, console,
        setTimeout, clearTimeout, setInterval, clearInterval,
        requestAnimationFrame, cancelAnimationFrame, queueMicrotask,
        fetch, XMLHttpRequest,
        Node, Text, Comment, Element, HTMLFormElement, EventTarget, Document,
        // Common globals SPAs expect
        HTMLElement: Element,
        HTMLDivElement: Element,
        HTMLSpanElement: Element,
        HTMLInputElement: Element,
        HTMLButtonElement: Element,
        HTMLAnchorElement: Element,
        HTMLImageElement: Element,
        HTMLScriptElement: Element,
        HTMLStyleElement: Element,
        HTMLLinkElement: Element,
        HTMLMetaElement: Element,
        HTMLHeadElement: Element,
        HTMLBodyElement: Element,
        HTMLFormElement: HTMLFormElement,
        HTMLSelectElement: Element,
        HTMLTextAreaElement: Element,
        HTMLCanvasElement: Element,
        HTMLVideoElement: Element,
        HTMLAudioElement: Element,
        DocumentFragment: Node,
        NodeList: Array,
        HTMLCollection: Array,
        DOMParser: class { parseFromString(html, type) { return document; } },
        MutationObserver: class { constructor(cb) {} observe() {} disconnect() {} },
        IntersectionObserver: class { constructor(cb) {} observe() {} disconnect() {} },
        ResizeObserver: class { constructor(cb) {} observe() {} disconnect() {} },
        CustomEvent: class extends EventTarget { constructor(type, opts) { super(); this.type = type; this.detail = opts?.detail; } },
        Event: class { constructor(type) { this.type = type; } },
        URL: class { constructor(url, base) { this.href = url; this.toString = () => url; } },
        URLSearchParams: class {
            constructor(init) { this.__p = new Map(); }
            get(k) { return this.__p.get(k) || null; }
            set(k,v) { this.__p.set(k,v); }
            has(k) { return this.__p.has(k); }
            toString() { return [...this.__p].map(([k,v]) => `${k}=${v}`).join('&'); }
        },
        // Performance
        performance: { now: () => Date.now(), mark: () => {}, measure: () => {}, getEntriesByType: () => [] },
        // Crypto
        crypto: {
            getRandomValues: (arr) => { for (let i = 0; i < arr.length; i++) arr[i] = Math.floor(Math.random() * 256); return arr; },
            randomUUID: () => 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, c => { const r = Math.random()*16|0; return (c === 'x' ? r : (r&0x3|0x8)).toString(16); }),
            subtle: { digest: () => Promise.resolve(new ArrayBuffer(32)) },
        },
        // CSS
        CSS: { supports: () => false, escape: (s) => s },
        getComputedStyle: () => new Proxy({}, { get: () => '' }),
        matchMedia: (q) => ({ matches: false, media: q, addEventListener: () => {}, removeEventListener: () => {} }),
        // Images/Canvas stubs
        Image: class { constructor() { this.src = ''; this.onload = null; } },
        // Encoding
        TextEncoder: typeof TextEncoder !== 'undefined' ? TextEncoder : class { encode(s) { return new Uint8Array(0); } },
        TextDecoder: typeof TextDecoder !== 'undefined' ? TextDecoder : class { decode(b) { return ''; } },
        // Blob/File stubs
        Blob: class { constructor(parts, opts) { this.size = 0; this.type = opts?.type || ''; } },
        File: class { constructor(parts, name, opts) { this.name = name; this.size = 0; } },
        FileReader: class extends EventTarget { constructor() { super(); } readAsText() {} readAsDataURL() {} },
        // Error stubs
        DOMException: class extends Error { constructor(msg, name) { super(msg); this.name = name || 'DOMException'; } },
    });

    // __ghostExportDOM: extract DOM tree for Rust to read
    globalThis.__ghostExportDOM = function() {
        function serialize(node) {
            if (node instanceof Text) return { type: 'text', text: node.__text };
            if (!(node instanceof Element)) return null;
            const obj = {
                type: 'element',
                tag: node.tagName.toLowerCase(),
                attrs: { ...node.__attrs },
                children: [],
            };
            if (node.id) obj.attrs.id = node.id;
            if (node.__classList.length) obj.attrs.class = node.__classList.join(' ');
            for (const child of node.childNodes) {
                const s = serialize(child);
                if (s) obj.children.push(s);
            }
            return obj;
        }
        return JSON.stringify(serialize(document.documentElement));
    };

    // __ghostRunTimers: execute pending timers (called from Rust)
    globalThis.__ghostRunTimers = function(rounds) {
        for (let r = 0; r < (rounds || 3); r++) {
            const ids = Object.keys(__timers.pending).map(Number);
            for (const id of ids) {
                const timer = __timers.pending[id];
                if (!timer) continue;
                if (timer.type === 'timeout') delete __timers.pending[id];
                try { timer.fn(...timer.args); } catch(e) {}
            }
        }
    };
    "#
}

/// Execute JS in the Ghost DOM context.
/// Returns the serialized DOM tree after execution.
pub fn execute_page(html: &str, scripts: &[String], url: &str) -> Result<String, String> {
    let mut ctx = Context::default();

    // 1. Install DOM polyfill
    ctx.eval(Source::from_bytes(dom_polyfill()))
        .map_err(|e| format!("DOM polyfill error: {e}"))?;

    // 2. Set location from URL
    if let Ok(parsed) = url::Url::parse(url) {
        let loc_js = format!(
            r#"
            location.href = "{}";
            location.protocol = "{}";
            location.host = "{}";
            location.hostname = "{}";
            location.port = "{}";
            location.pathname = "{}";
            location.search = "{}";
            location.hash = "{}";
            location.origin = "{}";
            "#,
            url,
            parsed.scheme().to_string() + ":",
            parsed.host_str().unwrap_or(""),
            parsed.host_str().unwrap_or(""),
            parsed.port().map(|p| p.to_string()).unwrap_or_default(),
            parsed.path(),
            parsed.query().unwrap_or(""),
            parsed.fragment().unwrap_or(""),
            parsed.origin().ascii_serialization(),
        );
        ctx.eval(Source::from_bytes(&loc_js)).ok();
    }

    // 3. Parse HTML and populate document via JS
    let populate_js = html_to_dom_js(html);
    ctx.eval(Source::from_bytes(&populate_js))
        .map_err(|e| format!("HTML populate error: {e}"))?;

    // 4. Execute each script (with error tolerance)
    for (i, script) in scripts.iter().enumerate() {
        match ctx.eval(Source::from_bytes(script)) {
            Ok(_) => {},
            Err(e) => {
                eprintln!("[GHOST-DOM] Script {i} error (non-fatal): {e}");
            }
        }
    }

    // 5. Flush timers (3 rounds — catches most init chains)
    ctx.eval(Source::from_bytes("__ghostRunTimers(3)")).ok();

    // 6. Export DOM
    let result = ctx.eval(Source::from_bytes("__ghostExportDOM()"))
        .map_err(|e| format!("DOM export error: {e}"))?;

    match result.as_string() {
        Some(s) => Ok(s.to_std_string_escaped()),
        None => Ok("{}".to_string()),
    }
}

/// Convert raw HTML into JS that populates the Ghost DOM document.
/// Extracts: elements, attributes, text nodes, structure.
fn html_to_dom_js(html: &str) -> String {
    use html5ever::parse_document;
    use html5ever::tendril::TendrilSink;
    use markup5ever_rcdom::{RcDom, Handle, NodeData};

    let dom = parse_document(RcDom::default(), Default::default())
        .from_utf8()
        .read_from(&mut html.as_bytes())
        .unwrap();

    let mut js = String::with_capacity(html.len() / 2);
    js.push_str("(function(){\n");
    js.push_str("const d = document;\n");
    js.push_str("const body = d.body;\n");
    js.push_str("const head = d.head;\n");

    // Find <html>, <head>, <body> in parsed DOM and populate
    fn find_body_head(node: &Handle) -> (Option<Handle>, Option<Handle>) {
        let mut head = None;
        let mut body = None;
        for child in node.children.borrow().iter() {
            match &child.data {
                NodeData::Element { name, .. } => {
                    let tag = name.local.as_ref();
                    if tag == "head" { head = Some(child.clone()); }
                    if tag == "body" { body = Some(child.clone()); }
                    if tag == "html" {
                        let (h, b) = find_body_head(child);
                        if h.is_some() { head = h; }
                        if b.is_some() { body = b; }
                    }
                }
                _ => {}
            }
        }
        (head, body)
    }

    let (head_node, body_node) = find_body_head(&dom.document);

    // Populate head
    if let Some(head) = head_node {
        emit_children(&head, "head", &mut js, 0);
    }

    // Populate body
    if let Some(body) = body_node {
        emit_children(&body, "body", &mut js, 0);
    }

    js.push_str("})();\n");
    js
}

fn emit_children(node: &markup5ever_rcdom::Handle, parent_var: &str, js: &mut String, depth: usize) {
    use markup5ever_rcdom::NodeData;
    if depth > 30 { return; } // prevent infinite recursion

    for (i, child) in node.children.borrow().iter().enumerate() {
        let var_name = format!("n{}_{}", depth, i);
        match &child.data {
            NodeData::Element { name, attrs, .. } => {
                let tag = name.local.as_ref();
                // Skip script tags from populating (we'll execute them separately)
                if tag == "script" { continue; }
                // Skip style tags (no rendering)
                if tag == "style" || tag == "link" { continue; }

                js.push_str(&format!("var {} = d.createElement('{}');\n", var_name, escape_js(tag)));
                for attr in attrs.borrow().iter() {
                    let attr_name = attr.name.local.as_ref();
                    let attr_val = attr.value.to_string();
                    js.push_str(&format!("{}.setAttribute('{}', '{}');\n",
                        var_name, escape_js(attr_name), escape_js(&attr_val)));
                }
                js.push_str(&format!("{}.appendChild({});\n", parent_var, var_name));

                // Recurse into children
                emit_children(child, &var_name, js, depth + 1);
            }
            NodeData::Text { contents } => {
                let text = contents.borrow().to_string();
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    js.push_str(&format!("{}.appendChild(d.createTextNode('{}'));\n",
                        parent_var, escape_js(trimmed)));
                }
            }
            _ => {}
        }
    }
}

fn escape_js(s: &str) -> String {
    s.replace('\\', "\\\\")
     .replace('\'', "\\'")
     .replace('\n', "\\n")
     .replace('\r', "\\r")
     .replace('\t', "\\t")
}

/// Extract inline and external script URLs from HTML.
pub fn extract_scripts(html: &str) -> (Vec<String>, Vec<String>) {
    use html5ever::parse_document;
    use html5ever::tendril::TendrilSink;
    use markup5ever_rcdom::{RcDom, Handle, NodeData};

    let dom = parse_document(RcDom::default(), Default::default())
        .from_utf8()
        .read_from(&mut html.as_bytes())
        .unwrap();

    let mut inline = Vec::new();
    let mut external = Vec::new();

    fn collect(node: &Handle, inline: &mut Vec<String>, external: &mut Vec<String>) {
        if let NodeData::Element { name, attrs, .. } = &node.data {
            if name.local.as_ref() == "script" {
                let attrs = attrs.borrow();
                let src = attrs.iter().find(|a| a.name.local.as_ref() == "src");
                if let Some(src_attr) = src {
                    external.push(src_attr.value.to_string());
                } else {
                    // Inline script
                    let text: String = node.children.borrow().iter()
                        .filter_map(|c| match &c.data {
                            NodeData::Text { contents } => Some(contents.borrow().to_string()),
                            _ => None,
                        })
                        .collect();
                    if !text.trim().is_empty() {
                        inline.push(text);
                    }
                }
            }
        }
        for child in node.children.borrow().iter() {
            collect(child, inline, external);
        }
    }

    collect(&dom.document, &mut inline, &mut external);
    (inline, external)
}
