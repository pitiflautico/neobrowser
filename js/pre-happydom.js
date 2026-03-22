// Minimal globals that happy-dom needs BEFORE it loads
if (typeof globalThis.console === 'undefined') {
    globalThis.console = { log(){}, warn(){}, error(){}, info(){}, debug(){}, trace(){} };
}
if (typeof globalThis.setTimeout === 'undefined') {
    globalThis.setTimeout = function(fn, ms) { fn(); return 0; };
    globalThis.clearTimeout = function() {};
    globalThis.setInterval = function(fn, ms) { return 0; };
    globalThis.clearInterval = function() {};
}
if (typeof globalThis.queueMicrotask === 'undefined') {
    globalThis.queueMicrotask = function(fn) { Promise.resolve().then(fn); };
}
if (typeof globalThis.atob === 'undefined') {
    globalThis.atob = function(s) { return ''; };
    globalThis.btoa = function(s) { return ''; };
}
if (typeof globalThis.TextEncoder === 'undefined') {
    // V8 should have these but just in case
}
if (typeof globalThis.performance === 'undefined') {
    globalThis.performance = { now() { return Date.now(); } };
}
if (typeof globalThis.crypto === 'undefined') {
    globalThis.crypto = { getRandomValues(a) { for(let i=0;i<a.length;i++) a[i]=Math.floor(Math.random()*256); return a; }, randomUUID() { return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g,c=>{var r=Math.random()*16|0;return(c=='x'?r:r&0x3|0x8).toString(16)}); } };
}
if (typeof globalThis.navigator === 'undefined') {
    globalThis.navigator = { userAgent: 'Mozilla/5.0 Chrome/136', language: 'en-US', languages: ['en-US'], platform: 'MacIntel', cookieEnabled: true, onLine: true, hardwareConcurrency: 8 };
}
if (typeof globalThis.location === 'undefined') {
    globalThis.location = { href: 'about:blank', origin: '', protocol: 'about:', host: '', hostname: '', port: '', pathname: 'blank', search: '', hash: '' };
}
if (typeof globalThis.Event === 'undefined') {
    // deno_core V8 should have Event... but let's check
}
if (typeof globalThis.process === 'undefined') {
    globalThis.process = { 
        env: {}, 
        version: 'v20.0.0', 
        versions: { node: '20.0.0' },
        platform: 'darwin',
        nextTick: function(fn) { queueMicrotask(fn); },
        stdout: { write: function(){} },
        stderr: { write: function(){} },
        cwd: function() { return '/'; },
        exit: function() {},
        on: function() { return this; },
        removeListener: function() { return this; },
        argv: [],
    };
}

// happy-dom uses setImmediate/clearImmediate (Node.js API)
if (typeof globalThis.setImmediate === 'undefined') {
    globalThis.setImmediate = function(fn) { return setTimeout(fn, 0); };
    globalThis.clearImmediate = function(id) { clearTimeout(id); };
}

// happy-dom extends URL — deno_core 0.311 may not have URL on globalThis
if (typeof globalThis.URL === 'undefined') {
    // Minimal URL polyfill — happy-dom will override with full impl
    globalThis.URL = class URL {
        constructor(url, base) {
            let full = String(url || '');
            if (base && !full.match(/^https?:\/\//)) {
                full = base.replace(/\/[^/]*$/, '/') + url;
            }
            const m = full.match(/^(https?:)\/\/([^/:]+)(:\d+)?(\/[^?#]*)?(\?[^#]*)?(#.*)?$/);
            this.href = full;
            this.protocol = m?.[1] || '';
            this.hostname = m?.[2] || '';
            this.port = m?.[3]?.slice(1) || '';
            this.host = this.hostname + (this.port ? ':' + this.port : '');
            this.pathname = m?.[4] || '/';
            this.search = m?.[5] || '';
            this.hash = m?.[6] || '';
            this.origin = this.protocol + '//' + this.host;
            this.username = '';
            this.password = '';
            this.searchParams = new URLSearchParams(this.search);
        }
        toString() { return this.href; }
        toJSON() { return this.href; }
        static canParse(url, base) { try { new URL(url, base); return true; } catch { return false; } }
        static createObjectURL() { return 'blob:null'; }
        static revokeObjectURL() {}
    };
}

// happy-dom also uses URLSearchParams
if (typeof globalThis.URLSearchParams === 'undefined') {
    globalThis.URLSearchParams = class URLSearchParams {
        constructor(init) { this._params = []; if (typeof init === 'string') { init.replace(/^\?/, '').split('&').forEach(p => { const [k,v] = p.split('='); if(k) this._params.push([decodeURIComponent(k), decodeURIComponent(v||'')]); }); } }
        get(name) { const p = this._params.find(([k]) => k === name); return p ? p[1] : null; }
        set(name, value) { this.delete(name); this._params.push([name, String(value)]); }
        append(name, value) { this._params.push([name, String(value)]); }
        delete(name) { this._params = this._params.filter(([k]) => k !== name); }
        has(name) { return this._params.some(([k]) => k === name); }
        toString() { return this._params.map(([k,v]) => encodeURIComponent(k) + '=' + encodeURIComponent(v)).join('&'); }
        forEach(cb) { this._params.forEach(([k,v]) => cb(v, k, this)); }
        entries() { return this._params[Symbol.iterator](); }
        keys() { return this._params.map(([k]) => k)[Symbol.iterator](); }
        values() { return this._params.map(([,v]) => v)[Symbol.iterator](); }
        [Symbol.iterator]() { return this._params[Symbol.iterator](); }
    };
}
