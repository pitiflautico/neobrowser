// ChatGPT Sentinel — Turnstile VM + PoW solver + conversation API.
// Runs inside NeoRender's V8. No iframe needed.
// Based on reverse-engineered ChatGPT client-side security flow.

(function() {
'use strict';

// ═══════════════════════════════════════════════════════════════
// 0. CONFIG — browser fingerprint config array (18 elements)
// ═══════════════════════════════════════════════════════════════

function buildConfig(userAgent) {
    var cores = [8, 16, 24, 32];
    var navigatorKeys = 'registerProtocolHandler,storage,mediaDevices,bluetooth,clipboard';
    var documentKeys = '_reactListeningo743lnnpvdg,location';
    var windowKeys = 'webpackChunk_N_E,__NEXT_DATA__';
    var cachedDpl = 'prod-f501fe933b3edf57aea882da888e1a544df99840';
    var cachedScript = 'https://cdn.oaistatic.com/_next/static/cXh69klOLzS0Gy2joLDRS/_ssgManifest.js?dpl=453ebaec0d44c2decab71692e1bfe39be35a24b3';

    var now = new Date();
    var months = ['Jan','Feb','Mar','Apr','May','Jun','Jul','Aug','Sep','Oct','Nov','Dec'];
    var daysArr = ['Sun','Mon','Tue','Wed','Thu','Fri','Sat'];
    var pad = function(n) { return String(n).padStart(2, '0'); };
    var parseTime = daysArr[now.getUTCDay()] + ' ' + months[now.getUTCMonth()] + ' ' +
        pad(now.getUTCDate()) + ' ' + now.getUTCFullYear() + ' ' +
        pad(now.getUTCHours()) + ':' + pad(now.getUTCMinutes()) + ':' + pad(now.getUTCSeconds()) +
        ' GMT-0500 (Eastern Standard Time)';

    var resolutions = [1920+1080, 2560+1440, 1920+1200, 2560+1600];
    var ua = userAgent || (typeof navigator !== 'undefined' ? navigator.userAgent :
        'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 Chrome/131.0.0.0 Safari/537.36');

    return [
        resolutions[Math.floor(Math.random() * resolutions.length)],  // 0: screen
        parseTime,                      // 1: timestamp
        4294705152,                     // 2: constant
        0,                              // 3: nonce (replaced per iteration)
        ua,                             // 4: user agent
        cachedScript,                   // 5: cached script
        cachedDpl,                      // 6: deployment
        'en-US',                        // 7: language
        'en-US,es-US,en,es',            // 8: languages
        0,                              // 9: elapsed ms (replaced per iteration)
        navigatorKeys,                  // 10
        documentKeys,                   // 11
        windowKeys,                     // 12
        performance.now() * 1000,       // 13: perf counter
        crypto.randomUUID(),            // 14: UUID
        '',                             // 15: empty
        cores[Math.floor(Math.random() * cores.length)],  // 16: cores
        Date.now() - performance.now(), // 17: time offset
    ];
}

function encodeConfig(config) {
    return btoa(JSON.stringify(config));
}

// ═══════════════════════════════════════════════════════════════
// 1. VM TOKEN — the 'p' parameter for sentinel/prepare
// ═══════════════════════════════════════════════════════════════

function generateVmToken(config) {
    var t0 = Date.now();
    config[3] = 1;
    config[9] = Math.round(Date.now() - t0);
    return 'gAAAAAC' + encodeConfig(config);
}

// ═══════════════════════════════════════════════════════════════
// 2. PROOF-OF-WORK — FNV-1a hash (NOT SHA-256/SHA3)
// ═══════════════════════════════════════════════════════════════

// FNV-1a 32-bit hash
function fnv1a(str) {
    var t = 2166136261;
    for (var i = 0; i < str.length; i++) {
        t ^= str.charCodeAt(i);
        t = Math.imul(t, 16777619) >>> 0;
    }
    t ^= (t >>> 16);
    t = Math.imul(t, 2246822507) >>> 0;
    t ^= (t >>> 13);
    t = Math.imul(t, 3266489909) >>> 0;
    t ^= (t >>> 16);
    // Return 8-char hex
    return ('00000000' + t.toString(16)).slice(-8);
}

function solvePow(seed, difficulty, config) {
    var t0 = Date.now();
    for (var i = 0; i < 500000; i++) {
        config[3] = i;
        config[9] = Math.round(Date.now() - t0);
        var encoded = encodeConfig(config);
        var hash = fnv1a(seed + encoded);
        if (hash.slice(0, difficulty.length) <= difficulty) {
            return 'gAAAAAB' + encoded + '~S';
        }
    }
    // Fallback
    config[3] = 'wQ8Lk5FbGpA2NcR9dShT6gYjU7VxZ4D';
    config[9] = Math.round(Date.now() - t0);
    return 'gAAAAAC' + encodeConfig(config);
}

// ═══════════════════════════════════════════════════════════════
// 3. TURNSTILE VM — solves Cloudflare Turnstile without an iframe
// ═══════════════════════════════════════════════════════════════

function xor(data, key) {
    if (!key || key.length === 0) return data;
    var result = [];
    for (var i = 0; i < data.length; i++) {
        result.push(String.fromCharCode(data.charCodeAt(i) ^ key.charCodeAt(i % key.length)));
    }
    return result.join('');
}

function decryptDx(dx, p) {
    try {
        var decoded = atob(dx);
        var decrypted = xor(decoded, p);
        return JSON.parse(decrypted);
    } catch (e) {
        return null;
    }
}

// OrderedMap — preserves insertion order
function OrderedMap() { this._keys = []; this._vals = {}; }
OrderedMap.prototype.add = function(key, value) {
    if (!(key in this._vals)) this._keys.push(key);
    this._vals[key] = value;
};
OrderedMap.prototype.toJSON = function() {
    var obj = {};
    for (var i = 0; i < this._keys.length; i++) obj[this._keys[i]] = this._vals[this._keys[i]];
    return obj;
};

function toStr(v) {
    if (v === undefined || v === null) return 'undefined';
    if (typeof v === 'number') return String(v);
    if (typeof v === 'string') {
        var specials = {
            'window.Math': '[object Math]',
            'window.Reflect': '[object Reflect]',
            'window.performance': '[object Performance]',
            'window.localStorage': '[object Storage]',
            'window.Object': 'function Object() { [native code] }',
            'window.Reflect.set': 'function set() { [native code] }',
            'window.performance.now': 'function () { [native code] }',
            'window.Object.create': 'function create() { [native code] }',
            'window.Object.keys': 'function keys() { [native code] }',
            'window.Math.random': 'function random() { [native code] }',
        };
        return specials[v] || v;
    }
    if (Array.isArray(v) && v.every(function(i) { return typeof i === 'string'; })) return v.join(',');
    if (v instanceof OrderedMap) return JSON.stringify(v);
    return String(v);
}

function solveTurnstile(dx, p) {
    var startTime = performance.now();
    var tokens = decryptDx(dx, p);
    if (!tokens || !Array.isArray(tokens)) return '';

    var result = '';
    var m = {};

    m[1] = function(e, t) { m[e] = xor(toStr(m[e]), toStr(m[t])); };
    m[2] = function(e, t) { m[e] = t; };
    m[3] = function(e) { result = btoa(e); };
    m[5] = function(e, t) {
        var n = m[e], tres = m[t];
        if (Array.isArray(n)) { m[e] = n.concat([tres]); }
        else if (typeof n === 'string' || typeof tres === 'string') { m[e] = toStr(n) + toStr(tres); }
        else if (typeof n === 'number' && typeof tres === 'number') { m[e] = n + tres; }
        else { m[e] = 'NaN'; }
    };
    m[6] = function(e, t, n) {
        var tv = m[t], nv = m[n];
        if (typeof tv === 'string' && typeof nv === 'string') {
            var path = tv + '.' + nv;
            if (path === 'window.document.location') m[e] = location.href || 'https://chatgpt.com/';
            else m[e] = path;
        }
    };
    m[7] = function(e) {
        var args = Array.prototype.slice.call(arguments, 1);
        var n = args.map(function(a) { return m[a]; });
        var ev = m[e];
        if (typeof ev === 'string' && ev === 'window.Reflect.set') {
            var obj = n[0], key = String(n[1]), val = n[2];
            if (obj instanceof OrderedMap) obj.add(key, val);
        } else if (typeof ev === 'function') {
            ev.apply(null, n);
        }
    };
    m[8] = function(e, t) { m[e] = m[t]; };
    m[10] = 'window';
    // 13: TRY_CALL — call with error capture
    m[13] = function(e, t) {
        var args = Array.prototype.slice.call(arguments, 2);
        try {
            var fn = m[t];
            if (typeof fn === 'function') {
                var fargs = args.map(function(a) { return m[a]; });
                m[e] = fn.apply(null, fargs);
            } else {
                // Array/property access
                m[e] = m[t];
            }
        } catch (ex) {
            m[e] = '' + ex;
        }
    };
    m[14] = function(e, t) { if (typeof m[t] === 'string') m[e] = JSON.parse(m[t]); };
    m[15] = function(e, t) { m[e] = JSON.stringify(m[t]); };
    m[17] = function(e, t) {
        var args = Array.prototype.slice.call(arguments, 2);
        var i = args.map(function(a) { return m[a]; });
        var tv = m[t];
        var res = null;
        if (typeof tv === 'string') {
            if (tv === 'window.performance.now') {
                res = performance.now() - startTime + Math.random();
            } else if (tv === 'window.Object.create') {
                res = new OrderedMap();
            } else if (tv === 'window.Object.keys') {
                if (typeof i[0] === 'string' && i[0] === 'window.localStorage') {
                    res = [
                        'STATSIG_LOCAL_STORAGE_INTERNAL_STORE_V4',
                        'STATSIG_LOCAL_STORAGE_STABLE_ID',
                        'client-correlated-secret',
                        'oai/apps/capExpiresAt',
                        'oai-did',
                        'STATSIG_LOCAL_STORAGE_LOGGING_REQUEST',
                        'UiState.isNavigationCollapsed.1',
                    ];
                } else { res = []; }
            } else if (tv === 'window.Math.random') {
                res = Math.random();
            }
        }
        m[e] = res;
    };
    m[18] = function(e) { m[e] = atob(toStr(m[e])); };
    m[19] = function(e) { m[e] = btoa(toStr(m[e])); };
    m[20] = function(e, t, n) {
        var args = Array.prototype.slice.call(arguments, 3);
        var o = args.map(function(a) { return m[a]; });
        if (m[e] === m[t]) {
            var nv = m[n];
            if (typeof nv === 'function') nv.apply(null, o);
        }
    };
    m[21] = function() {};
    // 22: TEMP_STACK_CALL — like 17 but with temp stack
    m[22] = function(e, t) {
        var args = Array.prototype.slice.call(arguments, 2);
        var i = args.map(function(a) { return m[a]; });
        var tv = m[t];
        var res = null;
        if (typeof tv === 'function') {
            try { res = tv.apply(null, i); } catch (ex) {}
        } else if (typeof tv === 'string') {
            // Same string-based dispatch as m[17]
            if (tv === 'window.performance.now') res = performance.now() - startTime + Math.random();
            else if (tv === 'window.Object.create') res = new OrderedMap();
            else if (tv === 'window.Object.keys') {
                if (typeof i[0] === 'string' && i[0] === 'window.localStorage') {
                    res = ['STATSIG_LOCAL_STORAGE_INTERNAL_STORE_V4','STATSIG_LOCAL_STORAGE_STABLE_ID',
                        'client-correlated-secret','oai/apps/capExpiresAt','oai-did',
                        'STATSIG_LOCAL_STORAGE_LOGGING_REQUEST','UiState.isNavigationCollapsed.1'];
                } else { res = []; }
            } else if (tv === 'window.Math.random') res = Math.random();
        }
        m[e] = res;
    };
    m[23] = function(e, t) {
        var args = Array.prototype.slice.call(arguments, 2);
        if (m[e] !== null && m[e] !== undefined) {
            var tv = m[t];
            if (typeof tv === 'function') tv.apply(null, args);
        }
    };
    m[24] = function(e, t, n) {
        var tv = m[t], nv = m[n];
        if (typeof tv === 'string' && typeof nv === 'string') m[e] = tv + '.' + nv;
    };
    // 34: MOVE — move value (copy + delete source)
    m[34] = function(e, t) { m[e] = m[t]; delete m[t]; };

    m[9] = tokens;
    m[16] = p;

    // Execute token list, then check if m[9] was replaced (inner bytecode)
    function runTokens(toks) {
        for (var ti = 0; ti < toks.length; ti++) {
            try {
                var token = toks[ti];
                var opcode = token[0];
                var targs = token.slice(1);
                var fn = m[opcode];
                if (typeof fn === 'function') fn.apply(null, targs);
            } catch (e) { /* skip like real impl */ }
        }
    }

    // Run outer layer
    var prevM9 = m[9];
    runTokens(tokens);

    // If m[9] was replaced with a new token list (inner bytecode), execute it
    if (m[9] !== prevM9 && Array.isArray(m[9])) {
        var innerTokens = m[9];
        runTokens(innerTokens);
        // Check for yet another layer
        if (m[9] !== innerTokens && Array.isArray(m[9])) {
            runTokens(m[9]);
        }
    }

    return result;
}

// ═══════════════════════════════════════════════════════════════
// 4. HIGH-LEVEL: __chatgpt_sentinel() — full sentinel flow
// ═══════════════════════════════════════════════════════════════

globalThis.__chatgpt_sentinel = async function() {
    // 1. Get access token
    var authResp = await fetch('/api/auth/session');
    var authData = await authResp.json();
    var accessToken = authData.accessToken;
    if (!accessToken) return JSON.stringify({ error: 'no_access_token', keys: Object.keys(authData) });

    var deviceId = crypto.randomUUID();

    // 2. Build config + VM token (the 'p' parameter)
    var config = buildConfig();
    var vmToken = generateVmToken(config);

    // 3. Prepare sentinel — send VM token as 'p'
    var prepResp = await fetch('/backend-api/sentinel/chat-requirements/prepare', {
        method: 'POST',
        headers: {
            'Authorization': 'Bearer ' + accessToken,
            'Content-Type': 'application/json',
            'OAI-Device-Id': deviceId,
        },
        body: JSON.stringify({ p: vmToken }),
    });
    var sentinel = await prepResp.json();

    // 4. Solve Turnstile (XOR decrypt with vmToken, then execute VM)
    var turnstileToken = '';
    if (sentinel.turnstile && sentinel.turnstile.required && sentinel.turnstile.dx) {
        try {
            turnstileToken = solveTurnstile(sentinel.turnstile.dx, vmToken);
        } catch (e) { /* non-fatal */ }
    }

    // 5. Solve PoW (SHA3-512 via Rust native op)
    var powToken = '';
    if (sentinel.proofofwork && sentinel.proofofwork.required) {
        var powConfig = buildConfig();
        try {
            var powResultJson = ops.op_pow_solve(
                sentinel.proofofwork.seed,
                sentinel.proofofwork.difficulty,
                JSON.stringify(powConfig)
            );
            var powResult = JSON.parse(powResultJson);
            powToken = powResult.token || '';
        } catch (e) {
            // Fallback to FNV-1a
            powToken = solvePow(sentinel.proofofwork.seed, sentinel.proofofwork.difficulty, powConfig);
        }
    }

    return JSON.stringify({
        ok: true,
        accessToken: accessToken,
        deviceId: deviceId,
        prepareToken: sentinel.prepare_token || '',
        turnstileToken: turnstileToken,
        powToken: powToken,
        persona: sentinel.persona,
    });
};

// ═══════════════════════════════════════════════════════════════
// 5. __chatgpt_send() — send a message, get response
// ═══════════════════════════════════════════════════════════════

globalThis.__chatgpt_send = async function(message, model, conversationId, parentMessageId) {
    model = model || 'auto';
    parentMessageId = parentMessageId || crypto.randomUUID();

    var sentinelRaw = await globalThis.__chatgpt_sentinel();
    var sentinel = JSON.parse(sentinelRaw);
    if (!sentinel.ok) return sentinelRaw;

    var body = {
        action: 'next',
        messages: [{
            id: crypto.randomUUID(),
            author: { role: 'user' },
            content: { content_type: 'text', parts: [message] },
            metadata: {},
        }],
        model: model,
        parent_message_id: parentMessageId,
        timezone_offset_min: new Date().getTimezoneOffset(),
        timezone: 'America/Los_Angeles',
        history_and_training_disabled: false,
        conversation_mode: { kind: 'primary_assistant' },
        force_paragen: false,
        force_paragen_model_slug: '',
        force_rate_limit: false,
        force_use_sse: true,
        suggestions: [],
        supported_encodings: [],
        system_hints: [],
        paragen_cot_summary_display_override: 'allow',
        paragen_stream_type_override: null,
        conversation_origin: null,
        client_contextual_info: {
            is_dark_mode: false,
            time_since_loaded: Math.floor(Math.random() * 450) + 50,
            page_height: Math.floor(Math.random() * 500) + 500,
            page_width: Math.floor(Math.random() * 1000) + 1000,
            pixel_ratio: 1.5,
            screen_height: Math.floor(Math.random() * 400) + 800,
            screen_width: Math.floor(Math.random() * 1000) + 1200,
        },
        reset_rate_limits: false,
        variant_purpose: 'comparison_implicit',
        websocket_request_id: crypto.randomUUID(),
    };
    if (conversationId) body.conversation_id = conversationId;

    var headers = {
        'Authorization': 'Bearer ' + sentinel.accessToken,
        'Content-Type': 'application/json',
        'Accept': 'text/event-stream',
        'OAI-Device-Id': sentinel.deviceId,
        'OAI-Language': 'en-US',
        'Openai-Sentinel-Chat-Requirements-Token': sentinel.prepareToken,
    };
    if (sentinel.powToken) headers['Openai-Sentinel-Proof-Token'] = sentinel.powToken;
    if (sentinel.turnstileToken) headers['Openai-Sentinel-Turnstile-Token'] = sentinel.turnstileToken;

    var resp = await fetch('/backend-api/conversation', {
        method: 'POST',
        headers: headers,
        body: JSON.stringify(body),
    });

    if (resp.status !== 200) {
        var errText = await resp.text();
        return JSON.stringify({
            error: true,
            status: resp.status,
            detail: errText.slice(0, 500),
            sentinel_debug: {
                hasPrepareToken: !!sentinel.prepareToken,
                hasPowToken: !!sentinel.powToken,
                hasTurnstileToken: !!sentinel.turnstileToken,
                turnstileLen: (sentinel.turnstileToken || '').length,
                powLen: (sentinel.powToken || '').length,
            }
        });
    }

    // Parse SSE response
    var text = await resp.text();
    var lines = text.split('\n');
    var lastData = null;
    var fullText = '';
    var convId = conversationId || '';
    var msgId = '';

    for (var li = 0; li < lines.length; li++) {
        var line = lines[li];
        if (line.indexOf('data: ') !== 0) continue;
        var data = line.slice(6).trim();
        if (data === '[DONE]') break;
        try {
            var parsed = JSON.parse(data);
            if (parsed.message && parsed.message.content && parsed.message.content.parts) {
                fullText = parsed.message.content.parts.join('');
                msgId = parsed.message.id || msgId;
            }
            if (parsed.conversation_id) convId = parsed.conversation_id;
            lastData = parsed;
        } catch (ex) {}
    }

    return JSON.stringify({
        ok: true,
        text: fullText,
        conversation_id: convId,
        message_id: msgId,
        model: (lastData && lastData.message && lastData.message.metadata) ? lastData.message.metadata.model_slug : model,
    });
};

})();
