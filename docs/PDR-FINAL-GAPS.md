# PDR — NeoRender V2: Lo que falta (final, sin excusas)

## Filosofía

No necesitamos Chrome real para casi nada. Lo que falta es ingeniería.
Anti-bot/fingerprint es la única frontera real que requiere Chrome.
Todo lo demás se implementa.

---

## Lo que YA funciona (no tocar)

- 10/10 sites renderizan (332→1334 WOM nodes)
- React 18 hydration (329/354 fibers)
- Happy-dom DOM completo (className, classList, querySelector, MutationObserver)
- Event loop: Promise, queueMicrotask, setTimeout, setInterval, MessageChannel, rAF, rIC
- Fetch async + cookies auto-injection desde SQLite + SSE detection
- History/location/pushState/replaceState/back/forward/popstate/hashchange
- document.defaultView === window
- find_element multi-strategy + fill_form React-compatible
- ProseMirror/Tiptap/Lexical/Slate/CodeMirror/Quill detection + input bridge
- 415+ tests, 0 warnings

---

## G1 — crypto.subtle real

**Status**: stubs (generateKey retorna {}, sign retorna ArrayBuffer vacío)
**Bloquea**: ChatGPT Turnstile, cualquier site con challenge criptográfico, WebAuthn flows
**Impacto**: P0 absoluta

### Qué implementar

| Método | Necesario para | Complejidad |
|---|---|---|
| `digest(algo, data)` | PoW, hashing | ✅ YA FUNCIONA (SHA-256 real) |
| `generateKey(algo, extractable, usages)` | Turnstile, WebCrypto flows | Media |
| `importKey(format, key, algo, extractable, usages)` | JWT verify, HMAC | Media |
| `exportKey(format, key)` | Token generation | Baja |
| `sign(algo, key, data)` | HMAC-SHA256, ECDSA | Media |
| `verify(algo, key, signature, data)` | Token verify | Media |
| `encrypt/decrypt` | Raro en frontend | Baja |
| `deriveBits/deriveKey` | PBKDF2, HKDF | Baja |

### Approach

Usar `ring` crate (ya en el ecosistema Rust):
```rust
// Cargo.toml
ring = "0.17"

// ops.rs
#[op2(async)]
async fn op_crypto_sign(#[string] algo: String, #[buffer] key: &[u8], #[buffer] data: &[u8]) -> Result<Vec<u8>, AnyError> {
    match algo.as_str() {
        "HMAC-SHA256" => {
            let k = hmac::Key::new(hmac::HMAC_SHA256, key);
            Ok(hmac::sign(&k, data).as_ref().to_vec())
        }
        _ => Err(...)
    }
}
```

En JS, `crypto.subtle.sign` llama `ops.op_crypto_sign(algo, key, data)` (async op → Promise).

### Ficheros
- `crates/neo-runtime/Cargo.toml` — añadir `ring`
- `crates/neo-runtime/src/ops.rs` — ops crypto
- `crates/neo-runtime/src/v8.rs` — registrar ops
- `js/bootstrap.js` — conectar crypto.subtle a ops

### Estimado: 1 sesión
### Test: ChatGPT sentinel completa → conversation POST se dispara

---

## G2 — Streaming fetch body (incremental real)

**Status**: op_fetch lee body entero antes de retornar. ReadableStream entrega chunks con yields pero DESPUÉS de que todo el body llegó.
**Bloquea**: navigation post-streaming en SPAs (turbo-stream, React Router data), ChatGPT conversation rendering
**Impacto**: P0

### Qué implementar

Separar fetch en 2 fases:
1. `op_fetch_start(url, method, body, headers)` → retorna `{stream_id, status, headers}` inmediatamente
2. `op_fetch_read_chunk(stream_id)` → retorna siguiente chunk o `{done: true}`

El HTTP response se mantiene vivo en Rust (HashMap<u32, ActiveStream>). JS llama `read_chunk` en un loop async.

### Approach

```rust
struct ActiveStream {
    response: rquest::Response,
    id: u32,
}

static STREAMS: Lazy<Mutex<HashMap<u32, ActiveStream>>> = ...;

#[op2(async)]
async fn op_fetch_start(...) -> String {
    let resp = client.get(url).send().await?;
    let id = next_stream_id();
    let status = resp.status();
    let headers = resp.headers().clone();
    STREAMS.lock().insert(id, ActiveStream { response: resp, id });
    json!({ "stream_id": id, "status": status, "headers": headers })
}

#[op2(async)]
async fn op_fetch_read_chunk(#[smi] stream_id: u32) -> Option<Vec<u8>> {
    let streams = STREAMS.lock();
    let stream = streams.get_mut(&stream_id)?;
    stream.response.chunk().await  // returns next chunk or None
}
```

En JS:
```javascript
globalThis.fetch = async function(input, init) {
    const result = JSON.parse(await ops.op_fetch_start(url, method, body, headers));
    return new NeoResponse(result.stream_id, result.status, result.headers);
};

// NeoResponse.body getter:
get body() {
    const streamId = this._streamId;
    return new ReadableStream({
        async pull(controller) {
            const chunk = await ops.op_fetch_read_chunk(streamId);
            if (chunk === null) { controller.close(); return; }
            controller.enqueue(new Uint8Array(chunk));
        }
    });
}
```

### Ficheros
- `crates/neo-runtime/src/ops.rs` — `op_fetch_start`, `op_fetch_read_chunk`, `op_fetch_close`
- `js/bootstrap.js` — NeoResponse con streaming body

### Complejidad: Alta
### Estimado: 2 sesiones
### Test: `fetch(url).then(r => r.body.getReader().read())` retorna primer chunk antes de body completo

---

## G3 — Pipeline post-interacción robusto

**Status**: React re-renders post-click pero navigation a nueva ruta no siempre ocurre
**Bloquea**: SPA transitions complejas (ChatGPT → /c/<id>, Mercadona → categorías)
**Impacto**: P0/P1

### Qué implementar

No es una API nueva — es coherencia entre:
- fetch completion → store update → effect fires → navigate() → URL change → view mount

### Items concretos

1. **history.state getter**: verificar que `window.history.state` retorna lo que se pasó a pushState
2. **React Router createBrowserHistory**: verificar que funciona con nuestro history shim (test directo)
3. **Side-effect timing**: después de un fetch resolve, los microtasks deben drenar ANTES del siguiente macrotask check
4. **Navigation detection**: log cuando navigate/pushState se llama para debugging

### Ficheros
- `js/browser_shim.js` — history.state getter, navigate logging
- `crates/neo-runtime/src/v8_runtime_impl.rs` — microtask drain guarantee

### Complejidad: Media
### Estimado: 1 sesión
### Test: Mercadona tienda: type CP → Continuar → categories visible

---

## G4 — PerformanceObserver + stubs menores

**Status**: no existe
**Bloquea**: crashes de telemetría en muchas apps
**Impacto**: P2

```javascript
globalThis.PerformanceObserver = class PerformanceObserver {
    constructor(cb) {}
    observe() {}
    disconnect() {}
    takeRecords() { return []; }
    static supportedEntryTypes = [];
};

globalThis.Worker = class Worker extends EventTarget {
    constructor() { super(); }
    postMessage() {}
    terminate() {}
};
```

### Estimado: 10 minutos
### Test: `new PerformanceObserver(() => {})` no crashea

---

## G5 — Layout con valores plausibles

**Status**: getBoundingClientRect retorna 0s, offsetWidth retorna 0 o undefined
**Bloquea**: responsive logic, virtualized lists, lazy loading, menús dropdown
**Impacto**: P1

### Qué implementar

Heuristic sizing sin layout engine real:

```javascript
// En browser_shim.js o bootstrap.js
const _viewportW = 1920, _viewportH = 1080;

// Block elements: full viewport width, height based on content
// Inline elements: width based on text length * avg char width
Element.prototype.getBoundingClientRect = function() {
    const tag = this.tagName?.toLowerCase();
    const isBlock = ['div','p','section','article','main','header','footer','nav','form','ul','ol','li','h1','h2','h3','h4','h5','h6','table','tr'].includes(tag);
    const textLen = this.textContent?.length || 0;
    const w = isBlock ? _viewportW : Math.min(textLen * 8, _viewportW);
    const h = isBlock ? Math.max(20, Math.min(textLen * 0.5, 500)) : 20;
    return { top: 0, left: 0, right: w, bottom: h, width: w, height: h, x: 0, y: 0 };
};

// offsetWidth/Height
Object.defineProperty(HTMLElement.prototype, 'offsetWidth', { get() { return this.getBoundingClientRect().width; } });
Object.defineProperty(HTMLElement.prototype, 'offsetHeight', { get() { return this.getBoundingClientRect().height; } });
Object.defineProperty(HTMLElement.prototype, 'clientWidth', { get() { return this.getBoundingClientRect().width; } });
Object.defineProperty(HTMLElement.prototype, 'clientHeight', { get() { return this.getBoundingClientRect().height; } });
```

### Ficheros: `js/browser_shim.js`
### Complejidad: Media
### Estimado: 1 sesión
### Test: `document.body.offsetWidth > 0`

---

## G6 — Selection/Range mejorados

**Status**: APIs existen (happy-dom), sin estado real de caret
**Bloquea**: ProseMirror/Lexical editing nativo (pero tenemos bridge)
**Impacto**: P1 (P2 si bridge cubre los casos)

### Approach

NO intentar implementar Selection/Range completo. En su lugar:

1. **Virtual caret**: tracking de posición como (node, offset)
2. **getSelection()** retorna el virtual caret
3. **execCommand('insertText')** usa el virtual caret para insertar
4. **Bridge por editor sigue siendo la primera opción** (ProseMirror via view.dispatch, etc.)

### Complejidad: Alta para implementación completa, media para virtual caret
### Estimado: 2 sesiones para virtual caret, indefinido para Selection/Range real

---

## Resumen ejecutivo

| # | Gap | Coste | Impacto | Prioridad |
|---|---|---|---|---|
| G1 | crypto.subtle real | 1 sesión | Desbloquea anti-bot flows | **P0** |
| G2 | Streaming fetch body | 2 sesiones | Desbloquea SPA navigation | **P0** |
| G3 | Pipeline post-interacción | 1 sesión | Robustez routing | **P0/P1** |
| G4 | PerformanceObserver + stubs | 10 min | Evita crashes | **P2** |
| G5 | Layout plausible | 1 sesión | Responsive logic | **P1** |
| G6 | Selection/Range virtual | 2 sesiones | Rich text | **P1** |

### Total P0: 4 sesiones
### Total P0+P1: 8 sesiones
### Total todo: 8.5 sesiones

---

## Lo que NO implementamos (y por qué)

| Item | Razón |
|---|---|
| Canvas/WebGL/WebRTC | No navegamos, no renderizamos gráficos |
| Audio/Video real | No consumimos multimedia |
| Layout engine real | Coste/beneficio desproporcionado. Stubs plausibles cubren 90% |
| IME composition real | Requiere input method del OS. Bridge por editor cubre |
| Fingerprint perfecto | Anti-bot evoluciona. Chrome proxy cuando sea necesario |
| Service Worker real | Stub suficiente. Apps sin SW funcionan |

---

## Orden de ejecución

```
Sesión 1: G1 (crypto.subtle) + G4 (stubs triviales)
Sesión 2: G2 parte 1 (op_fetch_start + op_fetch_read_chunk en Rust)
Sesión 3: G2 parte 2 (JS ReadableStream integration + tests)
Sesión 4: G3 (pipeline post-interacción) + G5 (layout stubs)
Sesión 5: G6 (Selection/Range virtual) + test battery completa
```

### Gate después de sesión 1:
ChatGPT sentinel completa → conversation POST se dispara

### Gate después de sesión 3:
`fetch(sse_url).then(r => r.body.getReader())` entrega chunks incrementalmente

### Gate después de sesión 4:
Mercadona tienda: CP → submit → categories visible

### Gate final:
10/10 sites interactivos + ChatGPT PONG + DuckDuckGo search
