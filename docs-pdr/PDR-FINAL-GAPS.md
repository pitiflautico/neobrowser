# PDR — NeoRender V2: Gaps restantes (v2, corregido)

## Orden de ejecución

### Sesión 1-2: G2 — Streaming fetch real
### Sesión 3-4: G1 — crypto.subtle con CryptoKey real
### Sesión 5: G3 — Instrumentación causal + microtask drain
### Sesión 5: G5 — Layout plausible + G4 stubs triviales
### Sesión 6+: G6 — Selection/Range (solo si bridge no cubre)

---

## G2 — Streaming fetch body (PRIMERO)

**Status**: op_fetch lee body entero antes de retornar
**Bloquea**: SPA navigation, turbo-stream, ChatGPT conversation rendering
**Impacto**: Estructural — sin esto, pipelines modernos se rompen

### Diseño completo

#### Rust ops

```
op_fetch_start(url, method, body, headers) → { stream_id, status, headers }
op_fetch_read_chunk(stream_id) → { done: bool, data: Option<Vec<u8>>, error: Option<String> }
op_fetch_close(stream_id) → void
```

#### Invariantes

- Un `stream_id` solo puede tener un reader (no shared)
- `op_fetch_close` libera el response en Rust (cleanup)
- Abort: si `AbortSignal` fires, el stream se cierra con error
- EOF: `read_chunk` retorna `{done: true}` y auto-close
- Error: `read_chunk` retorna `{error: "..."}` y auto-close
- Timeout: configurable por stream (default 60s total, 15s per-chunk)
- Límite: max 20 streams abiertos simultáneamente
- Non-streaming fallback: si Content-Length < 64KB, lee entero (fast path)
- bodyUsed: tracked en JS (NeoResponse)
- reader.cancel(): calls op_fetch_close

#### Rust state

```rust
struct ActiveStream {
    response: rquest::Response,
    created_at: Instant,
    total_bytes: usize,
}

// In OpState, not global static
struct StreamStore {
    streams: HashMap<u32, ActiveStream>,
    next_id: u32,
}
```

#### JS integration

```javascript
globalThis.fetch = async function(input, init) {
    // ... URL/headers/cookies prep (existing code) ...
    const result = JSON.parse(await ops.op_fetch_start(url, method, body, headers));
    return new NeoResponse(result.stream_id, result.status, result.headers, url);
};

class NeoResponse {
    constructor(streamId, status, headers, url) {
        this._streamId = streamId;
        this.status = status;
        this.headers = new Headers(headers);
        this._url = url;
        this._bodyUsed = false;
        this._reader = null;
    }

    get body() {
        if (!this._stream) {
            const sid = this._streamId;
            this._stream = new ReadableStream({
                async pull(controller) {
                    const chunk = JSON.parse(await ops.op_fetch_read_chunk(sid));
                    if (chunk.done) { controller.close(); return; }
                    if (chunk.error) { controller.error(new Error(chunk.error)); return; }
                    controller.enqueue(new Uint8Array(chunk.data));
                }
            });
        }
        return this._stream;
    }

    async text() {
        this._bodyUsed = true;
        const reader = this.body.getReader();
        const chunks = [];
        while (true) {
            const { done, value } = await reader.read();
            if (done) break;
            chunks.push(value);
        }
        return new TextDecoder().decode(concat(chunks));
    }

    async json() {
        const t = await this.text();
        return t ? JSON.parse(t) : null;
    }

    clone() { /* needs stream tee or re-fetch */ }
}
```

### Ficheros
- `crates/neo-runtime/src/ops.rs` — `op_fetch_start`, `op_fetch_read_chunk`, `op_fetch_close`, `StreamStore`
- `crates/neo-http/src/client.rs` — `request_streaming()` que retorna headers + body stream
- `js/bootstrap.js` — NeoResponse rewrite completo

### Estimado: 2 sesiones
### Gate: primer chunk visible antes de fin de body en 3 runs consecutivos

---

## G1 — crypto.subtle con CryptoKey real

**Status**: digest real (SHA-256), todo lo demás stubs
**Bloquea**: ChatGPT Turnstile, WebCrypto flows, JWT verify
**Impacto**: P0

### Subdivisión

#### G1a — Auditoría de algoritmos usados por ChatGPT

Instrumentar: qué llama `crypto.subtle.*` durante ChatGPT load + send:
- Algoritmo (HMAC-SHA256, ECDSA-P256, AES-GCM, etc.)
- Formato de key (raw, jwk, spki, pkcs8)
- Usages (sign, verify, encrypt, decrypt)

```javascript
// Trap temporal
const _orig = crypto.subtle;
crypto.subtle = new Proxy(_orig, {
    get(target, prop) {
        return function(...args) {
            console.error('[CRYPTO] ' + prop + ' algo=' + JSON.stringify(args[0]) + ' format=' + args[1]?.substring?.(0,20));
            return target[prop]?.apply(target, args);
        };
    }
});
```

#### G1b — CryptoKey model

```javascript
class NeoCryptoKey {
    constructor(type, extractable, algorithm, usages, _raw) {
        this.type = type;           // 'secret' | 'public' | 'private'
        this.extractable = extractable;
        this.algorithm = algorithm;  // { name: 'HMAC', hash: 'SHA-256' }
        this.usages = usages;        // ['sign', 'verify']
        this._raw = _raw;           // Uint8Array internal key material
    }
}
```

#### G1c — Operaciones via `ring` crate

| Op | Algoritmo mínimo | ring API |
|---|---|---|
| generateKey | HMAC-SHA256, ECDSA-P256 | `hmac::Key::generate`, `agreement::EphemeralPrivateKey` |
| importKey | raw, jwk | Key parsing |
| exportKey | raw, jwk | Key serialization |
| sign | HMAC-SHA256 | `hmac::sign` |
| verify | HMAC-SHA256 | `hmac::verify` |
| digest | SHA-256, SHA-384, SHA-512 | Ya funciona |

#### G1d — Tests de compat JS

No solo tests Rust. Tests que ejecutan en el engine:
```javascript
const key = await crypto.subtle.generateKey({name: 'HMAC', hash: 'SHA-256'}, true, ['sign','verify']);
const sig = await crypto.subtle.sign('HMAC', key, new TextEncoder().encode('test'));
const ok = await crypto.subtle.verify('HMAC', key, sig, new TextEncoder().encode('test'));
assert(ok === true);
```

### Ficheros
- `crates/neo-runtime/Cargo.toml` — `ring = "0.17"`
- `crates/neo-runtime/src/ops.rs` — crypto ops
- `js/bootstrap.js` — crypto.subtle wired to ops con CryptoKey

### Estimado: 2 sesiones
### Gate: ChatGPT sentinel/chat-requirements completa consistentemente (3 runs)

---

## G3 — Instrumentación causal + microtask drain

**Status**: sabemos que navigate() no se llama pero no POR QUÉ
**Bloquea**: debugging de SPA transitions
**Impacto**: P0/P1

### G3a — Instrumentación causal post-interacción

Logging automático de cada paso del pipeline:

```javascript
globalThis.__neo_interaction_trace = [];
function traceStep(step, data) {
    __neo_interaction_trace.push({ step, ts: Date.now(), data });
}
```

Instrumentar:
- Event dispatched (click, submit, input) → `traceStep('event', {type, target})`
- Fetch started → `traceStep('fetch_start', {url, method})`
- Fetch resolved → `traceStep('fetch_end', {url, status})`
- history.pushState called → `traceStep('pushState', {url})`
- DOM mutation → `traceStep('mutation', {count})`
- Timer fired → `traceStep('timer', {source})`

Query: `__neo_interaction_trace` after interaction to see what happened.

### G3b — Microtask drain guarantee

After fetch resolve, ensure microtask queue drains BEFORE next macrotask:
- Verify `queueMicrotask` fires before `setTimeout(0)`
- Verify Promise.then() chains complete in same microtask checkpoint
- Test: `fetch().then(setState).then(navigate)` completes before next pump

### G3c — Route transition assertions

Tests que verifican:
```
click(button) → wait → location.pathname changed
```
En 3 corridas consecutivas.

### Estimado: 1 sesión
### Gate: trace shows complete pipeline OR shows WHERE it breaks

---

## G5 — Layout con valores plausibles

**Status**: getBoundingClientRect retorna 0s
**Bloquea**: responsive logic, virtualized lists, visibility checks

### Approach: heuristic sizing

```javascript
const VP_W = 1920, VP_H = 1080;
const BLOCK_TAGS = new Set(['div','p','section','article','main','header','footer','nav','form','ul','ol','li','h1','h2','h3','h4','h5','h6','table','tr','body']);

Element.prototype.getBoundingClientRect = function() {
    const isBlock = BLOCK_TAGS.has(this.tagName?.toLowerCase());
    const textLen = this.textContent?.length || 0;
    const w = isBlock ? VP_W : Math.min(textLen * 8, VP_W);
    const h = isBlock ? Math.max(20, Math.min(textLen * 0.3, 500)) : 20;
    return { top: 0, left: 0, right: w, bottom: h, width: w, height: h, x: 0, y: 0 };
};
```

Plus: offsetWidth/Height, clientWidth/Height, scrollHeight via getters.

### Estimado: 1 sesión (junto con G4 stubs)

---

## G4 — Stubs triviales (con G5)

```javascript
globalThis.PerformanceObserver = class { constructor(){} observe(){} disconnect(){} takeRecords(){return []} static supportedEntryTypes=[] };
globalThis.Worker = class extends EventTarget { constructor(){super()} postMessage(){} terminate(){} };
```

### Estimado: 10 minutos

---

## G6 — Selection/Range virtual (P2)

Solo si el bridge por editor no cubre.
Actualmente tenemos bridge para ProseMirror, Lexical, Slate, CodeMirror, Quill.

Si se necesita: virtual caret (node, offset) con getSelection/setSelection tracking.

### Estimado: 2 sesiones
### Prioridad: P2

---

## Gates corregidos

| Gate | Criterio | Cuándo |
|---|---|---|
| Gate 1 | `fetch(sse_url).then(r => r.body.getReader().read())` retorna primer chunk antes de EOF | Fin sesión 2 |
| Gate 2 | ChatGPT `sentinel/chat-requirements` completa en 3 runs consecutivos | Fin sesión 4 |
| Gate 3 | Click/submit produce navigate O mount nueva vista en 3 corridas | Fin sesión 5 |
| Gate final | ChatGPT send → POST dispatched, Mercadona CP → categories, DDG search → results. 3 repeticiones | Fin sesión 5-6 |

---

## Total: 5-6 sesiones para P0+P1
