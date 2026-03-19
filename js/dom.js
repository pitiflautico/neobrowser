// NeoRender DOM — AI's browser render engine.
// Minimal DOM so that Vue/React/Svelte can boot and build their component tree.
// No rendering, no layout, no paint. Just the DOM that JS frameworks manipulate.
// The AI reads the final DOM as an action map (WOM).

// ─── Event (defined early, Element.click() needs it) ───
class Event {
    constructor(type, opts) {
        this.type = type;
        this.bubbles = opts?.bubbles || false;
        this.cancelable = opts?.cancelable || false;
        this.target = null;
        this.currentTarget = null;
        this.defaultPrevented = false;
    }
    preventDefault() { this.defaultPrevented = true; }
    stopPropagation() {}
    stopImmediatePropagation() {}
}
class CustomEvent extends Event {
    constructor(type, opts) { super(type, opts); this.detail = opts?.detail; }
}

// ─── Node storage ───
const __nodes = new Map();
let __nextId = 1;

class EventTarget {
    constructor() { this.__listeners = {}; }
    addEventListener(type, fn, opts) {
        if (!this.__listeners[type]) this.__listeners[type] = [];
        this.__listeners[type].push(fn);
    }
    removeEventListener(type, fn) {
        if (!this.__listeners[type]) return;
        this.__listeners[type] = this.__listeners[type].filter(f => f !== fn);
    }
    dispatchEvent(evt) {
        const listeners = this.__listeners[evt.type || evt] || [];
        for (const fn of listeners) { try { fn(evt); } catch(e) {} }
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
        this.ownerDocument = null; // set after document is created
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
        if (idx >= 0) { this.childNodes.splice(idx, 1); child.parentNode = null; }
        return child;
    }
    insertBefore(newChild, refChild) {
        if (newChild.parentNode) newChild.parentNode.removeChild(newChild);
        const idx = refChild ? this.childNodes.indexOf(refChild) : -1;
        if (idx >= 0) this.childNodes.splice(idx, 0, newChild);
        else this.childNodes.push(newChild);
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
            if (deep) for (const child of this.childNodes) clone.appendChild(child.cloneNode(true));
            return clone;
        }
        if (this instanceof Text) return new Text(this.textContent);
        return new Node(this.nodeType, this.nodeName);
    }
    contains(other) {
        if (this === other) return true;
        for (const child of this.childNodes) if (child.contains(other)) return true;
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
    constructor(text) { super(3, '#text'); this.__text = text || ''; }
    get textContent() { return this.__text; }
    set textContent(val) { this.__text = val; }
    get data() { return this.__text; }
    set data(val) { this.__text = val; }
    get nodeValue() { return this.__text; }
    get length() { return this.__text.length; }
    substringData(offset, count) { return this.__text.substring(offset, offset + count); }
}

class Comment extends Node {
    constructor(text) { super(8, '#comment'); this.__text = text || ''; }
    get data() { return this.__text; }
    get nodeValue() { return this.__text; }
}

class DocumentFragment extends Node {
    constructor() { super(11, '#document-fragment'); }
    get children() { return this.childNodes.filter(c => c instanceof Element); }
    getElementById(id) { return __collectAll(this).find(e => e.id === id) || null; }
    querySelector(sel) { return __querySelectorAll(this, sel)[0] || null; }
    querySelectorAll(sel) { return __querySelectorAll(this, sel); }
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
    get attributes() {
        return Object.entries(this.__attrs).map(([name, value]) => ({ name, value, localName: name }));
    }
    get className() { return this.__classList.join(' '); }
    set className(val) { this.__classList = val.split(/\s+/).filter(Boolean); this.__attrs['class'] = val; }
    get classList() {
        const self = this;
        return {
            add(...cls) { for (const c of cls) if (!self.__classList.includes(c)) self.__classList.push(c); },
            remove(...cls) { self.__classList = self.__classList.filter(c => !cls.includes(c)); },
            toggle(c) { this.contains(c) ? this.remove(c) : this.add(c); },
            contains(c) { return self.__classList.includes(c); },
            get length() { return self.__classList.length; },
            item(i) { return self.__classList[i] || null; },
            forEach(fn) { self.__classList.forEach(fn); },
        };
    }
    get style() {
        const self = this;
        return new Proxy(self.__style, {
            get(t, p) {
                if (p === 'setProperty') return (k, v) => { t[k] = v; };
                if (p === 'getPropertyValue') return (k) => t[k] || '';
                if (p === 'removeProperty') return (k) => { delete t[k]; };
                if (p === 'cssText') return Object.entries(t).map(([k,v]) => `${k}:${v}`).join(';');
                return t[p] || '';
            },
            set(t, p, v) { t[p] = v; return true; },
        });
    }
    get innerHTML() {
        return this.childNodes.map(c => {
            if (c instanceof Text) return c.__text;
            if (c instanceof Comment) return `<!--${c.__text}-->`;
            if (c instanceof Element) return c.outerHTML;
            return '';
        }).join('');
    }
    set innerHTML(html) {
        this.childNodes = [];
        if (html) this.appendChild(new Text(html));
    }
    get outerHTML() {
        const attrs = Object.entries(this.__attrs).map(([k,v]) => ` ${k}="${v}"`).join('');
        const inner = this.innerHTML;
        const tag = this.tagName.toLowerCase();
        if (['br','hr','img','input','meta','link','area','base','col','embed','source','track','wbr'].includes(tag))
            return `<${tag}${attrs}>`;
        return `<${tag}${attrs}>${inner}</${tag}>`;
    }
    get children() { return this.childNodes.filter(c => c instanceof Element); }
    get childElementCount() { return this.children.length; }
    get firstElementChild() { return this.children[0] || null; }
    get lastElementChild() { const ch = this.children; return ch[ch.length-1] || null; }
    get nextElementSibling() {
        if (!this.parentNode) return null;
        const siblings = this.parentNode.children;
        return siblings[siblings.indexOf(this) + 1] || null;
    }
    get previousElementSibling() {
        if (!this.parentNode) return null;
        const siblings = this.parentNode.children;
        return siblings[siblings.indexOf(this) - 1] || null;
    }
    querySelectorAll(sel) { return __querySelectorAll(this, sel); }
    querySelector(sel) { return this.querySelectorAll(sel)[0] || null; }
    getElementsByTagName(tag) {
        tag = tag.toUpperCase();
        if (tag === '*') return __collectAll(this);
        return __collectAll(this).filter(e => e.tagName === tag);
    }
    getElementsByClassName(cls) { return __collectAll(this).filter(e => e.__classList.includes(cls)); }
    getElementById(id) { return __collectAll(this).find(e => e.id === id) || null; }
    closest(sel) {
        let el = this;
        while (el) { if (__matchesSelector(el, sel)) return el; el = el.parentNode; }
        return null;
    }
    matches(sel) { return __matchesSelector(this, sel); }
    getBoundingClientRect() { return {top:0,left:0,right:0,bottom:0,width:0,height:0,x:0,y:0}; }
    getClientRects() { return [this.getBoundingClientRect()]; }
    get offsetWidth() { return 0; }
    get offsetHeight() { return 0; }
    get offsetTop() { return 0; }
    get offsetLeft() { return 0; }
    get scrollWidth() { return 0; }
    get scrollHeight() { return 0; }
    get clientWidth() { return 1440; }
    get clientHeight() { return 900; }
    focus() {} blur() {}
    click() { this.dispatchEvent(new Event('click')); }
    remove() { if (this.parentNode) this.parentNode.removeChild(this); }
    after(...nodes) { /* stub */ }
    before(...nodes) { /* stub */ }
    append(...nodes) { for (const n of nodes) this.appendChild(typeof n === 'string' ? new Text(n) : n); }
    prepend(...nodes) {
        const first = this.firstChild;
        for (const n of nodes) this.insertBefore(typeof n === 'string' ? new Text(n) : n, first);
    }
    replaceWith(...nodes) { /* stub */ }
}

// ─── HTMLTemplateElement (Vue 3 needs this) ───
class HTMLTemplateElement extends Element {
    constructor() { super('template'); this.content = new DocumentFragment(); }
}

// ─── Query selector engine ───
function __collectAll(root) {
    const result = [];
    function walk(node) {
        if (node instanceof Element) result.push(node);
        for (const child of (node.childNodes || [])) walk(child);
    }
    for (const child of (root.childNodes || [])) walk(child);
    return result;
}
function __querySelectorAll(root, sel) { return __collectAll(root).filter(el => __matchesSelector(el, sel)); }
function __matchesSelector(el, sel) {
    if (!(el instanceof Element)) return false;
    if (sel.includes(',')) return sel.split(',').some(s => __matchesSelector(el, s.trim()));
    const parts = sel.trim().split(/\s+/);
    if (parts.length > 1) {
        if (!__matchesSingle(el, parts[parts.length - 1])) return false;
        let current = el.parentNode;
        for (let i = parts.length - 2; i >= 0; i--) {
            while (current && !__matchesSingle(current, parts[i])) current = current.parentNode;
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
    if (sel === '*') return true;
    if (sel.startsWith('#')) return el.id === sel.slice(1);
    if (sel.startsWith('.')) return el.__classList.includes(sel.slice(1));
    if (sel.startsWith('[')) {
        const m = sel.match(/\[([^=\]]+)(?:=["']?([^"'\]]*)["']?)?\]/);
        if (m) return m[2] !== undefined ? el.__attrs[m[1]] === m[2] : m[1] in el.__attrs;
    }
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
        if (tag.toLowerCase() === 'template') return new HTMLTemplateElement();
        return new Element(tag);
    }
    createTextNode(text) { return new Text(text); }
    createComment(text) { return new Comment(text); }
    createDocumentFragment() { return new DocumentFragment(); }
    createElementNS(ns, tag) { return this.createElement(tag); }
    createRange() {
        return {
            setStart() {}, setEnd() {},
            selectNode() {}, selectNodeContents() {},
            collapse() {}, cloneRange() { return this; },
            createContextualFragment(html) {
                const frag = new DocumentFragment();
                frag.appendChild(new Text(html));
                return frag;
            },
            getBoundingClientRect() { return {top:0,left:0,right:0,bottom:0,width:0,height:0}; },
        };
    }
    createTreeWalker(root, whatToShow, filter) {
        const nodes = [];
        function collect(n) {
            if (whatToShow === 1 && n instanceof Element) nodes.push(n);
            else if (whatToShow === 4 && n instanceof Text) nodes.push(n);
            else if (!whatToShow || whatToShow === 0xFFFFFFFF) nodes.push(n);
            for (const c of (n.childNodes || [])) collect(c);
        }
        collect(root);
        let idx = -1;
        return {
            currentNode: root,
            nextNode() { idx++; return idx < nodes.length ? (this.currentNode = nodes[idx]) : null; },
            previousNode() { idx--; return idx >= 0 ? (this.currentNode = nodes[idx]) : null; },
        };
    }
    getElementById(id) { return this.documentElement.getElementById(id); }
    getElementsByTagName(tag) { return this.documentElement.getElementsByTagName(tag); }
    getElementsByClassName(cls) { return this.documentElement.getElementsByClassName(cls); }
    querySelector(sel) { return this.documentElement.querySelector(sel); }
    querySelectorAll(sel) { return this.documentElement.querySelectorAll(sel); }
    get title() { const t = this.head.querySelector('title'); return t ? t.textContent : ''; }
    set title(val) {
        let t = this.head.querySelector('title');
        if (!t) { t = this.createElement('title'); this.head.appendChild(t); }
        t.textContent = val;
    }
    get cookie() { return this.__cookies || ''; }
    set cookie(val) {
        const existing = this.__cookies || '';
        this.__cookies = existing ? existing + '; ' + val : val;
    }
    // readyState: SPAs check this to decide when to mount
    readyState = 'loading';
    // createRange: used by Vue/React for DOM manipulation
    createRange() { return { setStart(){}, setEnd(){}, commonAncestorContainer: this.body, createContextualFragment(html) { const d = document.createElement('div'); d.innerHTML = html; return d; } }; }
    // createTreeWalker: used by Vue 3 for template compilation
    createTreeWalker(root, what, filter) { return { currentNode: root, nextNode() { return null; } }; }
    // createNodeIterator
    createNodeIterator(root) { return { nextNode() { return null; } }; }
}

const document = new Document();

// ─── Location ───
const location = {
    href: '', protocol: 'https:', host: '', hostname: '',
    port: '', pathname: '/', search: '', hash: '', origin: '',
    assign(url) { location.href = url; },
    replace(url) { location.href = url; },
    reload() {},
    toString() { return location.href; },
};

// ─── Navigator ───
const navigator = {
    userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36',
    language: 'en-US', languages: ['en-US', 'en', 'es'],
    platform: 'MacIntel', cookieEnabled: true, onLine: true,
    vendor: 'Google Inc.', maxTouchPoints: 0,
    hardwareConcurrency: 8, deviceMemory: 8,
    connection: { effectiveType: '4g', downlink: 10, rtt: 50 },
    permissions: { query: () => Promise.resolve({state: 'granted'}) },
    clipboard: { readText: () => Promise.resolve(''), writeText: () => Promise.resolve() },
    mediaDevices: { enumerateDevices: () => Promise.resolve([]) },
    serviceWorker: { register: () => Promise.resolve({}), getRegistrations: () => Promise.resolve([]) },
    sendBeacon: () => true,
};

const screen = { width: 1440, height: 900, availWidth: 1440, availHeight: 875, colorDepth: 24, pixelDepth: 24 };
const history = {
    length: 1, state: null,
    pushState(s, t, u) { if (u) location.href = u; history.length++; },
    replaceState(s, t, u) { if (u) location.href = u; },
    back() {}, forward() {}, go() {},
};

const localStorage = new (class Storage {
    constructor() { this.__data = {}; }
    getItem(k) { return this.__data[k] !== undefined ? this.__data[k] : null; }
    setItem(k, v) { this.__data[k] = String(v); }
    removeItem(k) { delete this.__data[k]; }
    clear() { this.__data = {}; }
    get length() { return Object.keys(this.__data).length; }
    key(i) { return Object.keys(this.__data)[i] || null; }
})();
const sessionStorage = new localStorage.constructor();

const console = {
    log: (...a) => {}, warn: (...a) => {}, error: (...a) => {},
    info: (...a) => {}, debug: (...a) => {}, trace: (...a) => {},
    dir: (...a) => {}, table: (...a) => {},
    group: () => {}, groupEnd: () => {},
    time: () => {}, timeEnd: () => {},
    assert: () => {}, clear: () => {}, count: () => {}, countReset: () => {},
};

// ─── Expose on globalThis ───
Object.assign(globalThis, {
    window: globalThis, self: globalThis,
    document, location, navigator, screen, history,
    localStorage, sessionStorage, console,
    Node, Text, Comment, Element, HTMLTemplateElement, EventTarget, Document,
    DocumentFragment,
    HTMLElement: Element, HTMLDivElement: Element, HTMLSpanElement: Element,
    HTMLInputElement: Element, HTMLButtonElement: Element, HTMLAnchorElement: Element,
    HTMLImageElement: Element, HTMLScriptElement: Element, HTMLStyleElement: Element,
    HTMLLinkElement: Element, HTMLMetaElement: Element, HTMLHeadElement: Element,
    HTMLBodyElement: Element, HTMLFormElement: Element, HTMLSelectElement: Element,
    HTMLTextAreaElement: Element, HTMLCanvasElement: Element,
    HTMLVideoElement: Element, HTMLAudioElement: Element,
    HTMLTableElement: Element, HTMLTableRowElement: Element,
    SVGElement: Element,
    NodeList: Array, HTMLCollection: Array,
    DOMParser: class { parseFromString(html) { return document; } },
    MutationObserver: class { constructor(cb) { this.__cb = cb; } observe() {} disconnect() {} },
    IntersectionObserver: class { constructor(cb) {} observe() {} disconnect() {} },
    ResizeObserver: class { constructor(cb) {} observe() {} disconnect() {} },
    CustomEvent,
    Event,
    URL: class { constructor(url, base) { this.href = base ? new String(base + url) : url; this.toString = () => this.href; this.pathname = '/'; this.search = ''; this.hash = ''; this.origin = ''; this.hostname = ''; } },
    URLSearchParams: class {
        constructor(init) { this.__p = new Map(); if (typeof init === 'string') init.replace(/^\?/,'').split('&').forEach(p => { const [k,v] = p.split('='); if (k) this.__p.set(k,v||''); }); }
        get(k) { return this.__p.get(k) || null; } set(k,v) { this.__p.set(k,v); }
        has(k) { return this.__p.has(k); } delete(k) { this.__p.delete(k); }
        toString() { return [...this.__p].map(([k,v]) => `${k}=${v}`).join('&'); }
        forEach(fn) { this.__p.forEach((v,k) => fn(v,k)); }
        entries() { return this.__p.entries(); }
    },
    performance: { now: () => Date.now(), mark: () => {}, measure: () => {}, getEntriesByType: () => [], getEntriesByName: () => [] },
    crypto: {
        getRandomValues: (arr) => { for (let i = 0; i < arr.length; i++) arr[i] = Math.floor(Math.random() * 256); return arr; },
        randomUUID: () => 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, c => { const r = Math.random()*16|0; return (c === 'x' ? r : (r&0x3|0x8)).toString(16); }),
        subtle: { digest: () => Promise.resolve(new ArrayBuffer(32)), importKey: () => Promise.resolve({}), sign: () => Promise.resolve(new ArrayBuffer(32)) },
    },
    CSS: { supports: () => false, escape: (s) => s },
    getComputedStyle: (el) => new Proxy({}, { get: (t,p) => { if (p === 'getPropertyValue') return () => ''; return ''; } }),
    matchMedia: (q) => ({ matches: false, media: q, addEventListener: () => {}, removeEventListener: () => {}, addListener: () => {}, removeListener: () => {} }),
    Image: class { constructor() { this.src = ''; this.onload = null; this.onerror = null; } },
    Blob: class { constructor(p, o) { this.size = 0; this.type = o?.type || ''; } },
    File: class { constructor(p, n, o) { this.name = n; this.size = 0; } },
    FileReader: class extends EventTarget { constructor() { super(); } readAsText() {} readAsDataURL() {} },
    DOMException: class extends Error { constructor(msg, name) { super(msg); this.name = name || 'DOMException'; } },
    AbortController: class { constructor() { this.signal = { aborted: false, addEventListener: () => {}, removeEventListener: () => {} }; } abort() { this.signal.aborted = true; } },
    Headers: class extends Map { constructor(init) { super(); if (init) Object.entries(init).forEach(([k,v]) => this.set(k.toLowerCase(), v)); } },
    FormData: class { constructor() { this.__d = []; } append(k,v) { this.__d.push([k,v]); } get(k) { const e = this.__d.find(([n]) => n===k); return e?e[1]:null; } entries() { return this.__d[Symbol.iterator](); } },
    requestAnimationFrame: (fn) => setTimeout(fn, 16),
    cancelAnimationFrame: (id) => clearTimeout(id),
    queueMicrotask: (fn) => Promise.resolve().then(fn),
    // Encoding
    TextEncoder: globalThis.TextEncoder || class { encode(s) { return new Uint8Array(0); } },
    TextDecoder: globalThis.TextDecoder || class { decode(b) { return ''; } },
});

// ─── Make window an EventTarget (SPAs add load/popstate/resize listeners) ───
const __windowET = new EventTarget();
globalThis.addEventListener = __windowET.addEventListener.bind(__windowET);
globalThis.removeEventListener = __windowET.removeEventListener.bind(__windowET);
globalThis.dispatchEvent = __windowET.dispatchEvent.bind(__windowET);

// ─── IE compat stubs (some libs check for createEventObject) ───
Document.prototype.createEventObject = function() { return new Event(''); };

// ─── Export DOM as HTML for Rust to re-parse ───
globalThis.__neorender_export = function() {
    return document.documentElement.outerHTML;
};
