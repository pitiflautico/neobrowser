// Node.js polyfills required by linkedom.
// Must run BEFORE linkedom.js is loaded.

// atob/btoa — base64 encoding
if (typeof atob === 'undefined') {
    const _c = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
    const _lookup = new Uint8Array(256);
    for (let i = 0; i < _c.length; i++) _lookup[_c.charCodeAt(i)] = i;

    globalThis.atob = function(b64) {
        b64 = String(b64).replace(/[\s=]+/g, '');
        const len = b64.length;
        const bytes = new Uint8Array(Math.floor(len * 3 / 4));
        let p = 0;
        for (let i = 0; i < len; i += 4) {
            const a = _lookup[b64.charCodeAt(i)];
            const b = _lookup[b64.charCodeAt(i+1)];
            const c = _lookup[b64.charCodeAt(i+2)];
            const d = _lookup[b64.charCodeAt(i+3)];
            bytes[p++] = (a << 2) | (b >> 4);
            if (i+2 < len) bytes[p++] = ((b & 15) << 4) | (c >> 2);
            if (i+3 < len) bytes[p++] = ((c & 3) << 6) | d;
        }
        let str = '';
        for (let i = 0; i < p; i++) str += String.fromCharCode(bytes[i]);
        return str;
    };

    globalThis.btoa = function(str) {
        str = String(str);
        let out = '';
        for (let i = 0; i < str.length; i += 3) {
            const a = str.charCodeAt(i);
            const b = str.charCodeAt(i+1);
            const c = str.charCodeAt(i+2);
            out += _c[a >> 2];
            out += _c[((a & 3) << 4) | (b >> 4)];
            out += i+1 < str.length ? _c[((b & 15) << 2) | (c >> 6)] : '=';
            out += i+2 < str.length ? _c[c & 63] : '=';
        }
        return out;
    };
}

// Buffer (Node.js compat for linkedom)
if (typeof Buffer === 'undefined') {
    globalThis.Buffer = {
        from: (input, encoding) => {
            if (encoding === 'base64') {
                const decoded = atob(input);
                return { toString: () => decoded, length: decoded.length };
            }
            if (typeof input === 'string') {
                const enc = new TextEncoder();
                const buf = enc.encode(input);
                buf.toString = (e) => e === 'base64' ? btoa(input) : input;
                return buf;
            }
            return input;
        },
        isBuffer: () => false,
        alloc: (size) => new Uint8Array(size),
    };
}

// process (Node.js compat)
if (typeof process === 'undefined') {
    globalThis.process = { env: {}, version: 'v20.0.0', platform: 'linux' };
}

// TextEncoder/TextDecoder — may not be exposed globally in deno_core
if (typeof TextDecoder === 'undefined') {
    globalThis.TextDecoder = class TextDecoder {
        constructor(label) { this.encoding = label || 'utf-8'; }
        decode(input) {
            if (!input || input.length === 0) return '';
            const bytes = input instanceof Uint8Array ? input : new Uint8Array(input);
            let str = '';
            for (let i = 0; i < bytes.length; i++) str += String.fromCharCode(bytes[i]);
            return str;
        }
    };
}
if (typeof TextEncoder === 'undefined') {
    globalThis.TextEncoder = class TextEncoder {
        constructor() { this.encoding = 'utf-8'; }
        encode(str) {
            const bytes = [];
            for (let i = 0; i < str.length; i++) bytes.push(str.charCodeAt(i) & 0xff);
            return new Uint8Array(bytes);
        }
    };
}
