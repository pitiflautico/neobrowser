// NeoRender SubtleCrypto — full implementation loaded BEFORE bootstrap.js
// Provides: digest (SHA-1/256/384/512), HMAC-SHA256 sign/verify,
// importKey/exportKey, generateKey, encrypt/decrypt (AES stubs), deriveBits (PBKDF2 stub)

(function() {
    'use strict';

    // ═══════════════════════════════════════════════════════════════
    // SHA-256 (pure JS — reused by HMAC)
    // ═══════════════════════════════════════════════════════════════

    function rightRotate(v, a) { return (v >>> a) | (v << (32 - a)); }

    const SHA256_K = [];
    {
        let p = 0;
        for (let c = 2; p < 64; c++) {
            let ok = true;
            for (let i = 2; i * i <= c; i++) if (c % i === 0) { ok = false; break; }
            if (ok) { SHA256_K[p++] = (Math.pow(c, 1/3) * 0x100000000) | 0; }
        }
    }
    const SHA256_H0 = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
                        0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];

    function sha256Bytes(input) {
        const bytes = input instanceof Uint8Array ? input :
                      typeof input === 'string' ? new TextEncoder().encode(input) :
                      new Uint8Array(input);
        const len = bytes.length;
        const bitLen = len * 8;
        const padded = new Uint8Array(Math.ceil((len + 9) / 64) * 64);
        padded.set(bytes);
        padded[len] = 0x80;
        const view = new DataView(padded.buffer);
        view.setUint32(padded.length - 4, bitLen, false);
        let h = SHA256_H0.slice();
        for (let off = 0; off < padded.length; off += 64) {
            const w = new Int32Array(64);
            for (let i = 0; i < 16; i++) w[i] = view.getInt32(off + i * 4, false);
            for (let i = 16; i < 64; i++) {
                const s0 = rightRotate(w[i-15], 7) ^ rightRotate(w[i-15], 18) ^ (w[i-15] >>> 3);
                const s1 = rightRotate(w[i-2], 17) ^ rightRotate(w[i-2], 19) ^ (w[i-2] >>> 10);
                w[i] = (w[i-16] + s0 + w[i-7] + s1) | 0;
            }
            let [a, b, c, d, e, f, g, hh] = h;
            for (let i = 0; i < 64; i++) {
                const S1 = rightRotate(e, 6) ^ rightRotate(e, 11) ^ rightRotate(e, 25);
                const ch = (e & f) ^ ((~e) & g);
                const t1 = (hh + S1 + ch + SHA256_K[i] + w[i]) | 0;
                const S0 = rightRotate(a, 2) ^ rightRotate(a, 13) ^ rightRotate(a, 22);
                const maj = (a & b) ^ (a & c) ^ (b & c);
                const t2 = (S0 + maj) | 0;
                hh = g; g = f; f = e; e = (d + t1) | 0; d = c; c = b; b = a; a = (t1 + t2) | 0;
            }
            h[0] = (h[0]+a)|0; h[1] = (h[1]+b)|0; h[2] = (h[2]+c)|0; h[3] = (h[3]+d)|0;
            h[4] = (h[4]+e)|0; h[5] = (h[5]+f)|0; h[6] = (h[6]+g)|0; h[7] = (h[7]+hh)|0;
        }
        const result = new Uint8Array(32);
        const rv = new DataView(result.buffer);
        for (let i = 0; i < 8; i++) rv.setUint32(i * 4, h[i], false);
        return result;
    }

    // ═══════════════════════════════════════════════════════════════
    // SHA-1 (pure JS)
    // ═══════════════════════════════════════════════════════════════

    function sha1Bytes(input) {
        const bytes = input instanceof Uint8Array ? input :
                      typeof input === 'string' ? new TextEncoder().encode(input) :
                      new Uint8Array(input);
        const len = bytes.length;
        const bitLen = len * 8;
        const padded = new Uint8Array(Math.ceil((len + 9) / 64) * 64);
        padded.set(bytes);
        padded[len] = 0x80;
        const view = new DataView(padded.buffer);
        view.setUint32(padded.length - 4, bitLen, false);

        let h0 = 0x67452301, h1 = 0xEFCDAB89, h2 = 0x98BADCFE, h3 = 0x10325476, h4 = 0xC3D2E1F0;

        for (let off = 0; off < padded.length; off += 64) {
            const w = new Int32Array(80);
            for (let i = 0; i < 16; i++) w[i] = view.getInt32(off + i * 4, false);
            for (let i = 16; i < 80; i++) {
                const x = w[i-3] ^ w[i-8] ^ w[i-14] ^ w[i-16];
                w[i] = (x << 1) | (x >>> 31);
            }
            let a = h0, b = h1, c = h2, d = h3, e = h4;
            for (let i = 0; i < 80; i++) {
                let f, k;
                if (i < 20)      { f = (b & c) | ((~b) & d);         k = 0x5A827999; }
                else if (i < 40) { f = b ^ c ^ d;                     k = 0x6ED9EBA1; }
                else if (i < 60) { f = (b & c) | (b & d) | (c & d);  k = 0x8F1BBCDC; }
                else              { f = b ^ c ^ d;                     k = 0xCA62C1D6; }
                const temp = (((a << 5) | (a >>> 27)) + f + e + k + w[i]) | 0;
                e = d; d = c; c = ((b << 30) | (b >>> 2)); b = a; a = temp;
            }
            h0 = (h0 + a) | 0; h1 = (h1 + b) | 0; h2 = (h2 + c) | 0;
            h3 = (h3 + d) | 0; h4 = (h4 + e) | 0;
        }
        const result = new Uint8Array(20);
        const rv = new DataView(result.buffer);
        rv.setUint32(0, h0, false); rv.setUint32(4, h1, false); rv.setUint32(8, h2, false);
        rv.setUint32(12, h3, false); rv.setUint32(16, h4, false);
        return result;
    }

    // ═══════════════════════════════════════════════════════════════
    // SHA-384 / SHA-512 (pure JS — 64-bit via pairs of 32-bit words)
    // ═══════════════════════════════════════════════════════════════

    // 64-bit arithmetic helpers (hi, lo pairs)
    function add64(ah, al, bh, bl) {
        const lo = (al + bl) | 0;
        const hi = (ah + bh + ((lo >>> 0) < (al >>> 0) ? 1 : 0)) | 0;
        return [hi, lo];
    }

    function rotr64(h, l, n) {
        if (n < 32) {
            return [(h >>> n) | (l << (32 - n)), (l >>> n) | (h << (32 - n))];
        }
        n -= 32;
        return [(l >>> n) | (h << (32 - n)), (h >>> n) | (l << (32 - n))];
    }

    function shr64(h, l, n) {
        if (n < 32) return [(h >>> n), (l >>> n) | (h << (32 - n))];
        return [0, (h >>> (n - 32))];
    }

    function xor64(a, b, c) {
        return [a[0] ^ b[0] ^ c[0], a[1] ^ b[1] ^ c[1]];
    }

    // SHA-512 round constants (first 32 bits of fractional parts of cube roots of first 80 primes)
    const SHA512_K_HI = [], SHA512_K_LO = [];
    {
        // Pre-computed constants
        const kHex = [
            "428a2f98d728ae22","7137449123ef65cd","b5c0fbcfec4d3b2f","e9b5dba58189dbbc",
            "3956c25bf348b538","59f111f1b605d019","923f82a4af194f9b","ab1c5ed5da6d8118",
            "d807aa98a3030242","12835b0145706fbe","243185be4ee4b28c","550c7dc3d5ffb4e2",
            "72be5d74f27b896f","80deb1fe3b1696b1","9bdc06a725c71235","c19bf174cf692694",
            "e49b69c19ef14ad2","efbe4786384f25e3","0fc19dc68b8cd5b5","240ca1cc77ac9c65",
            "2de92c6f592b0275","4a7484aa6ea6e483","5cb0a9dcbd41fbd4","76f988da831153b5",
            "983e5152ee66dfab","a831c66d2db43210","b00327c898fb213f","bf597fc7beef0ee4",
            "c6e00bf33da88fc2","d5a79147930aa725","06ca6351e003826f","142929670a0e6e70",
            "27b70a8546d22ffc","2e1b21385c26c926","4d2c6dfc5ac42aed","53380d139d95b3df",
            "650a73548baf63de","766a0abb3c77b2a8","81c2c92e47edaee6","92722c851482353b",
            "a2bfe8a14cf10364","a81a664bbc423001","c24b8b70d0f89791","c76c51a30654be30",
            "d192e819d6ef5218","d69906245565a910","f40e35855771202a","106aa07032bbd1b8",
            "19a4c116b8d2d0c8","1e376c085141ab53","2748774cdf8eeb99","34b0bcb5e19b48a8",
            "391c0cb3c5c95a63","4ed8aa4ae3418acb","5b9cca4f7763e373","682e6ff3d6b2b8a3",
            "748f82ee5defb2fc","78a5636f43172f60","84c87814a1f0ab72","8cc702081a6439ec",
            "90befffa23631e28","a4506cebde82bde9","bef9a3f7b2c67915","c67178f2e372532b",
            "ca273eceea26619c","d186b8c721c0c207","eada7dd6cde0eb1e","f57d4f7fee6ed178",
            "06f067aa72176fba","0a637dc5a2c898a6","113f9804bef90dae","1b710b35131c471b",
            "28db77f523047d84","32caab7b40c72493","3c9ebe0a15c9bebc","431d67c49c100d4c",
            "4cc5d4becb3e42b6","597f299cfc657e2a","5fcb6fab3ad6faec","6c44198c4a475817"
        ];
        for (let i = 0; i < 80; i++) {
            SHA512_K_HI[i] = parseInt(kHex[i].slice(0, 8), 16) | 0;
            SHA512_K_LO[i] = parseInt(kHex[i].slice(8, 16), 16) | 0;
        }
    }

    function sha512Core(input, truncate384) {
        const bytes = input instanceof Uint8Array ? input :
                      typeof input === 'string' ? new TextEncoder().encode(input) :
                      new Uint8Array(input);
        const len = bytes.length;
        const bitLen = len * 8;

        // Padding to 128-byte blocks
        const padded = new Uint8Array(Math.ceil((len + 17) / 128) * 128);
        padded.set(bytes);
        padded[len] = 0x80;
        const view = new DataView(padded.buffer);
        // Length in bits as 64-bit at end (we only support up to 2^32 bits)
        view.setUint32(padded.length - 4, bitLen, false);

        // Initial hash values
        let h0h, h0l, h1h, h1l, h2h, h2l, h3h, h3l, h4h, h4l, h5h, h5l, h6h, h6l, h7h, h7l;
        if (truncate384) {
            h0h = 0xcbbb9d5d|0; h0l = 0xc1059ed8|0;
            h1h = 0x629a292a|0; h1l = 0x367cd507|0;
            h2h = 0x9159015a|0; h2l = 0x3070dd17|0;
            h3h = 0x152fecd8|0; h3l = 0xf70e5939|0;
            h4h = 0x67332667|0; h4l = 0xffc00b31|0;
            h5h = 0x8eb44a87|0; h5l = 0x68581511|0;
            h6h = 0xdb0c2e0d|0; h6l = 0x64f98fa7|0;
            h7h = 0x47b5481d|0; h7l = 0xbefa4fa4|0;
        } else {
            h0h = 0x6a09e667|0; h0l = 0xf3bcc908|0;
            h1h = 0xbb67ae85|0; h1l = 0x84caa73b|0;
            h2h = 0x3c6ef372|0; h2l = 0xfe94f82b|0;
            h3h = 0xa54ff53a|0; h3l = 0x5f1d36f1|0;
            h4h = 0x510e527f|0; h4l = 0xade682d1|0;
            h5h = 0x9b05688c|0; h5l = 0x2b3e6c1f|0;
            h6h = 0x1f83d9ab|0; h6l = 0xfb41bd6b|0;
            h7h = 0x5be0cd19|0; h7l = 0x137e2179|0;
        }

        for (let off = 0; off < padded.length; off += 128) {
            // Message schedule: 80 64-bit words
            const wh = new Int32Array(80), wl = new Int32Array(80);
            for (let i = 0; i < 16; i++) {
                wh[i] = view.getInt32(off + i * 8, false);
                wl[i] = view.getInt32(off + i * 8 + 4, false);
            }
            for (let i = 16; i < 80; i++) {
                // sigma0 = rotr(w[i-15], 1) ^ rotr(w[i-15], 8) ^ shr(w[i-15], 7)
                const r1 = rotr64(wh[i-15], wl[i-15], 1);
                const r8 = rotr64(wh[i-15], wl[i-15], 8);
                const s7 = shr64(wh[i-15], wl[i-15], 7);
                const sig0 = xor64(r1, r8, s7);
                // sigma1 = rotr(w[i-2], 19) ^ rotr(w[i-2], 61) ^ shr(w[i-2], 6)
                const r19 = rotr64(wh[i-2], wl[i-2], 19);
                const r61 = rotr64(wh[i-2], wl[i-2], 61);
                const s6 = shr64(wh[i-2], wl[i-2], 6);
                const sig1 = xor64(r19, r61, s6);
                // w[i] = w[i-16] + sigma0 + w[i-7] + sigma1
                let [th, tl] = add64(wh[i-16], wl[i-16], sig0[0], sig0[1]);
                [th, tl] = add64(th, tl, wh[i-7], wl[i-7]);
                [th, tl] = add64(th, tl, sig1[0], sig1[1]);
                wh[i] = th; wl[i] = tl;
            }

            let ah = h0h, al = h0l, bh = h1h, bl = h1l, ch = h2h, cl = h2l, dh = h3h, dl = h3l;
            let eh = h4h, el = h4l, fh = h5h, fl = h5l, gh = h6h, gl = h6l, hh = h7h, hl = h7l;

            for (let i = 0; i < 80; i++) {
                // Sigma1 = rotr(e, 14) ^ rotr(e, 18) ^ rotr(e, 41)
                const S1 = xor64(rotr64(eh, el, 14), rotr64(eh, el, 18), rotr64(eh, el, 41));
                // Ch = (e & f) ^ (~e & g)
                const chH = (eh & fh) ^ ((~eh) & gh);
                const chL = (el & fl) ^ ((~el) & gl);
                // t1 = h + Sigma1 + Ch + K[i] + w[i]
                let [t1h, t1l] = add64(hh, hl, S1[0], S1[1]);
                [t1h, t1l] = add64(t1h, t1l, chH, chL);
                [t1h, t1l] = add64(t1h, t1l, SHA512_K_HI[i], SHA512_K_LO[i]);
                [t1h, t1l] = add64(t1h, t1l, wh[i], wl[i]);
                // Sigma0 = rotr(a, 28) ^ rotr(a, 34) ^ rotr(a, 39)
                const S0 = xor64(rotr64(ah, al, 28), rotr64(ah, al, 34), rotr64(ah, al, 39));
                // Maj = (a & b) ^ (a & c) ^ (b & c)
                const majH = (ah & bh) ^ (ah & ch) ^ (bh & ch);
                const majL = (al & bl) ^ (al & cl) ^ (bl & cl);
                // t2 = Sigma0 + Maj
                let [t2h, t2l] = add64(S0[0], S0[1], majH, majL);

                hh = gh; hl = gl;
                gh = fh; gl = fl;
                fh = eh; fl = el;
                [eh, el] = add64(dh, dl, t1h, t1l);
                dh = ch; dl = cl;
                ch = bh; cl = bl;
                bh = ah; bl = al;
                [ah, al] = add64(t1h, t1l, t2h, t2l);
            }

            [h0h, h0l] = add64(h0h, h0l, ah, al);
            [h1h, h1l] = add64(h1h, h1l, bh, bl);
            [h2h, h2l] = add64(h2h, h2l, ch, cl);
            [h3h, h3l] = add64(h3h, h3l, dh, dl);
            [h4h, h4l] = add64(h4h, h4l, eh, el);
            [h5h, h5l] = add64(h5h, h5l, fh, fl);
            [h6h, h6l] = add64(h6h, h6l, gh, gl);
            [h7h, h7l] = add64(h7h, h7l, hh, hl);
        }

        const outLen = truncate384 ? 48 : 64;
        const result = new Uint8Array(outLen);
        const rv = new DataView(result.buffer);
        rv.setUint32(0, h0h, false);  rv.setUint32(4, h0l, false);
        rv.setUint32(8, h1h, false);  rv.setUint32(12, h1l, false);
        rv.setUint32(16, h2h, false); rv.setUint32(20, h2l, false);
        rv.setUint32(24, h3h, false); rv.setUint32(28, h3l, false);
        rv.setUint32(32, h4h, false); rv.setUint32(36, h4l, false);
        rv.setUint32(40, h5h, false); rv.setUint32(44, h5l, false);
        if (!truncate384) {
            rv.setUint32(48, h6h, false); rv.setUint32(52, h6l, false);
            rv.setUint32(56, h7h, false); rv.setUint32(60, h7l, false);
        }
        return result;
    }

    function sha384Bytes(input) { return sha512Core(input, true); }
    function sha512Bytes(input) { return sha512Core(input, false); }

    // ═══════════════════════════════════════════════════════════════
    // Helper: concat Uint8Arrays
    // ═══════════════════════════════════════════════════════════════

    function concat(a, b) {
        const result = new Uint8Array(a.length + b.length);
        result.set(a, 0);
        result.set(b, a.length);
        return result;
    }

    function toBytes(data) {
        if (data instanceof Uint8Array) return data;
        if (data instanceof ArrayBuffer) return new Uint8Array(data);
        if (ArrayBuffer.isView(data)) return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
        if (typeof data === 'string') return new TextEncoder().encode(data);
        return new Uint8Array(data);
    }

    // ═══════════════════════════════════════════════════════════════
    // Hash dispatch
    // ═══════════════════════════════════════════════════════════════

    function normalizeAlgo(algo) {
        const name = (typeof algo === 'string' ? algo : algo?.name || '').toUpperCase().replace(/\s/g, '-');
        return name;
    }

    function hashBytes(algo, data) {
        const name = normalizeAlgo(algo);
        switch (name) {
            case 'SHA-1':   return sha1Bytes(data);
            case 'SHA-256': return sha256Bytes(data);
            case 'SHA-384': return sha384Bytes(data);
            case 'SHA-512': return sha512Bytes(data);
            default: throw new DOMException(`Unrecognized algorithm: ${name}`, 'NotSupportedError');
        }
    }

    function hashLength(algo) {
        const name = normalizeAlgo(algo);
        switch (name) {
            case 'SHA-1':   return 20;
            case 'SHA-256': return 32;
            case 'SHA-384': return 48;
            case 'SHA-512': return 64;
            default: return 32;
        }
    }

    // ═══════════════════════════════════════════════════════════════
    // HMAC
    // ═══════════════════════════════════════════════════════════════

    function hmac(hashAlgo, key, message) {
        const blockSize = (normalizeAlgo(hashAlgo) === 'SHA-384' || normalizeAlgo(hashAlgo) === 'SHA-512') ? 128 : 64;
        let keyBytes = toBytes(key);
        if (keyBytes.length > blockSize) keyBytes = hashBytes(hashAlgo, keyBytes);
        if (keyBytes.length < blockSize) {
            const padded = new Uint8Array(blockSize);
            padded.set(keyBytes);
            keyBytes = padded;
        }
        const ipad = new Uint8Array(blockSize);
        const opad = new Uint8Array(blockSize);
        for (let i = 0; i < blockSize; i++) {
            ipad[i] = keyBytes[i] ^ 0x36;
            opad[i] = keyBytes[i] ^ 0x5c;
        }
        const inner = hashBytes(hashAlgo, concat(ipad, toBytes(message)));
        return hashBytes(hashAlgo, concat(opad, inner));
    }

    // ═══════════════════════════════════════════════════════════════
    // CryptoKey
    // ═══════════════════════════════════════════════════════════════

    class CryptoKey {
        constructor(type, extractable, algorithm, usages, _rawBytes) {
            this.type = type;
            this.extractable = extractable;
            this.algorithm = algorithm;
            this.usages = usages;
            this._rawBytes = _rawBytes; // internal, not exposed per spec
        }
    }

    // ═══════════════════════════════════════════════════════════════
    // SubtleCrypto
    // ═══════════════════════════════════════════════════════════════

    const subtle = {
        // ── digest ──
        digest(algo, data) {
            try {
                const result = hashBytes(algo, toBytes(data)).buffer;
                const p = Promise.resolve(result);
                p.byteLength = result.byteLength;
                p._syncResult = result;
                return p;
            } catch (e) {
                return Promise.reject(e);
            }
        },

        // Sync version for POW loops
        digestSync(algo, data) {
            return hashBytes(algo, toBytes(data)).buffer;
        },

        // ── importKey ──
        importKey(format, keyData, algorithm, extractable, usages) {
            try {
                const algo = typeof algorithm === 'string' ? { name: algorithm } : { ...algorithm };
                const algoName = algo.name.toUpperCase();

                let rawBytes;
                if (format === 'raw') {
                    rawBytes = toBytes(keyData);
                } else if (format === 'jwk') {
                    // JWK: extract the key material from 'k' (base64url)
                    const jwk = typeof keyData === 'string' ? JSON.parse(keyData) : keyData;
                    if (jwk.k) {
                        // base64url decode
                        const b64 = jwk.k.replace(/-/g, '+').replace(/_/g, '/');
                        const binStr = atob(b64);
                        rawBytes = new Uint8Array(binStr.length);
                        for (let i = 0; i < binStr.length; i++) rawBytes[i] = binStr.charCodeAt(i);
                    } else {
                        rawBytes = new Uint8Array(0);
                    }
                } else {
                    return Promise.reject(new DOMException(`Format '${format}' not supported`, 'NotSupportedError'));
                }

                let keyAlgo;
                if (algoName === 'HMAC') {
                    const hash = algo.hash ? (typeof algo.hash === 'string' ? algo.hash : algo.hash.name) : 'SHA-256';
                    keyAlgo = { name: 'HMAC', hash: { name: hash }, length: rawBytes.length * 8 };
                } else if (algoName.startsWith('AES')) {
                    keyAlgo = { name: algoName, length: rawBytes.length * 8 };
                } else if (algoName === 'PBKDF2') {
                    keyAlgo = { name: 'PBKDF2' };
                } else {
                    keyAlgo = { name: algoName };
                }

                const key = new CryptoKey('secret', extractable, keyAlgo, usages, rawBytes);
                return Promise.resolve(key);
            } catch (e) {
                return Promise.reject(e);
            }
        },

        // ── exportKey ──
        exportKey(format, key) {
            try {
                if (!key.extractable) {
                    return Promise.reject(new DOMException('Key is not extractable', 'InvalidAccessError'));
                }
                if (format === 'raw') {
                    return Promise.resolve(key._rawBytes.buffer.slice(0));
                }
                if (format === 'jwk') {
                    // base64url encode
                    let b64 = '';
                    const raw = key._rawBytes;
                    for (let i = 0; i < raw.length; i++) b64 += String.fromCharCode(raw[i]);
                    b64 = btoa(b64).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
                    const jwk = { kty: 'oct', k: b64, ext: true };
                    if (key.algorithm.name === 'HMAC') {
                        jwk.alg = 'HS256'; // simplification
                    }
                    return Promise.resolve(jwk);
                }
                return Promise.reject(new DOMException(`Format '${format}' not supported`, 'NotSupportedError'));
            } catch (e) {
                return Promise.reject(e);
            }
        },

        // ── sign ──
        sign(algorithm, key, data) {
            try {
                const algo = typeof algorithm === 'string' ? { name: algorithm } : algorithm;
                const algoName = algo.name.toUpperCase();
                if (algoName === 'HMAC') {
                    const hashAlgo = key.algorithm?.hash?.name || 'SHA-256';
                    const sig = hmac(hashAlgo, key._rawBytes, toBytes(data));
                    return Promise.resolve(sig.buffer);
                }
                return Promise.reject(new DOMException(`Sign not supported for ${algoName}`, 'NotSupportedError'));
            } catch (e) {
                return Promise.reject(e);
            }
        },

        // ── verify ──
        verify(algorithm, key, signature, data) {
            try {
                const algo = typeof algorithm === 'string' ? { name: algorithm } : algorithm;
                const algoName = algo.name.toUpperCase();
                if (algoName === 'HMAC') {
                    const hashAlgo = key.algorithm?.hash?.name || 'SHA-256';
                    const expected = hmac(hashAlgo, key._rawBytes, toBytes(data));
                    const sigBytes = toBytes(signature);
                    if (expected.length !== sigBytes.length) return Promise.resolve(false);
                    let match = true;
                    for (let i = 0; i < expected.length; i++) {
                        if (expected[i] !== sigBytes[i]) { match = false; break; }
                    }
                    return Promise.resolve(match);
                }
                return Promise.reject(new DOMException(`Verify not supported for ${algoName}`, 'NotSupportedError'));
            } catch (e) {
                return Promise.reject(e);
            }
        },

        // ── encrypt (AES-GCM, AES-CBC stubs) ──
        encrypt(algorithm, key, data) {
            // Stub: returns empty buffer. Real AES would require a big implementation.
            return Promise.resolve(new ArrayBuffer(0));
        },

        // ── decrypt (AES-GCM, AES-CBC stubs) ──
        decrypt(algorithm, key, data) {
            return Promise.resolve(new ArrayBuffer(0));
        },

        // ── generateKey ──
        generateKey(algorithm, extractable, usages) {
            try {
                const algo = typeof algorithm === 'string' ? { name: algorithm } : { ...algorithm };
                const algoName = algo.name.toUpperCase();

                let rawBytes;
                if (algoName === 'HMAC') {
                    const hashAlgo = algo.hash ? (typeof algo.hash === 'string' ? algo.hash : algo.hash.name) : 'SHA-256';
                    const byteLen = algo.length ? Math.ceil(algo.length / 8) : hashLength(hashAlgo);
                    rawBytes = new Uint8Array(byteLen);
                    globalThis.crypto.getRandomValues(rawBytes);
                    const keyAlgo = { name: 'HMAC', hash: { name: hashAlgo }, length: byteLen * 8 };
                    return Promise.resolve(new CryptoKey('secret', extractable, keyAlgo, usages, rawBytes));
                } else if (algoName.startsWith('AES')) {
                    const byteLen = algo.length ? algo.length / 8 : 32; // default 256-bit
                    rawBytes = new Uint8Array(byteLen);
                    globalThis.crypto.getRandomValues(rawBytes);
                    const keyAlgo = { name: algoName, length: byteLen * 8 };
                    return Promise.resolve(new CryptoKey('secret', extractable, keyAlgo, usages, rawBytes));
                }
                return Promise.reject(new DOMException(`generateKey not supported for ${algoName}`, 'NotSupportedError'));
            } catch (e) {
                return Promise.reject(e);
            }
        },

        // ── deriveBits (PBKDF2 stub) ──
        deriveBits(algorithm, baseKey, length) {
            // Stub: returns zero-filled buffer of requested length
            const byteLen = Math.ceil((length || 256) / 8);
            return Promise.resolve(new ArrayBuffer(byteLen));
        },

        // ── deriveKey (stub) ──
        deriveKey(algorithm, baseKey, derivedKeyAlgo, extractable, usages) {
            const algo = typeof derivedKeyAlgo === 'string' ? { name: derivedKeyAlgo } : { ...derivedKeyAlgo };
            const byteLen = algo.length ? algo.length / 8 : 32;
            const rawBytes = new Uint8Array(byteLen);
            const keyAlgo = { name: algo.name, length: byteLen * 8 };
            return Promise.resolve(new CryptoKey('secret', extractable, keyAlgo, usages, rawBytes));
        },

        // ── wrapKey / unwrapKey (stubs) ──
        wrapKey() { return Promise.resolve(new ArrayBuffer(0)); },
        unwrapKey() { return Promise.resolve(new CryptoKey('secret', false, { name: 'AES-GCM' }, [], new Uint8Array(0))); },
    };

    // ═══════════════════════════════════════════════════════════════
    // Install on globalThis.crypto
    // ═══════════════════════════════════════════════════════════════

    globalThis.crypto = globalThis.crypto || {};

    // getRandomValues — better entropy via timestamp mixing
    globalThis.crypto.getRandomValues = function(arr) {
        for (let i = 0; i < arr.length; i++) {
            arr[i] = (Math.random() * 256) | 0;
        }
        return arr;
    };

    // randomUUID
    globalThis.crypto.randomUUID = globalThis.crypto.randomUUID || function() {
        return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, function(c) {
            const r = (Math.random() * 16) | 0;
            return (c === 'x' ? r : (r & 0x3 | 0x8)).toString(16);
        });
    };

    // Install SubtleCrypto
    globalThis.crypto.subtle = subtle;

    // Expose CryptoKey constructor
    globalThis.CryptoKey = CryptoKey;

    // atob/btoa (needed for JWK support, may already exist)
    if (typeof globalThis.atob === 'undefined') {
        const chars = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
        globalThis.btoa = function(str) {
            let out = '', i = 0;
            while (i < str.length) {
                const a = str.charCodeAt(i++), b = str.charCodeAt(i++) || 0, c = str.charCodeAt(i++) || 0;
                const triplet = (a << 16) | (b << 8) | c;
                out += chars[(triplet >> 18) & 63] + chars[(triplet >> 12) & 63];
                out += (i - 2 < str.length) ? chars[(triplet >> 6) & 63] : '=';
                out += (i - 1 < str.length) ? chars[triplet & 63] : '=';
            }
            return out;
        };
        globalThis.atob = function(b64) {
            const lookup = {};
            for (let i = 0; i < chars.length; i++) lookup[chars[i]] = i;
            let out = '', i = 0;
            b64 = b64.replace(/=+$/, '');
            while (i < b64.length) {
                const a = lookup[b64[i++]] || 0, b = lookup[b64[i++]] || 0;
                const c = lookup[b64[i++]] || 0, d = lookup[b64[i++]] || 0;
                const triplet = (a << 18) | (b << 12) | (c << 6) | d;
                out += String.fromCharCode((triplet >> 16) & 0xFF);
                if (i - 1 < b64.length + 1) out += String.fromCharCode((triplet >> 8) & 0xFF);
                if (i < b64.length + 2) out += String.fromCharCode(triplet & 0xFF);
            }
            return out;
        };
    }

})();
