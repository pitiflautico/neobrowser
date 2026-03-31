# PDR — NeoRender V2: Gaps restantes para navegador funcional

## Estado actual (23 marzo 2026)

### Lo que funciona
- 10/10 sites renderizan (HN 332, Mercadona 118, GitHub 511, Wikipedia 1334 WOM nodes)
- React 18 hydration (329/354 fibers en ChatGPT)
- Happy-dom DOM: className, classList, querySelector, MutationObserver, todos OK
- Async fetch con cookie auto-injection desde SQLite
- SSE detection + parsing en HTTP client (read hasta [DONE])
- ReadableStream incremental delivery (pull-based con yields)
- ProseMirror EditorView capture + text input via transactions
- find_element multi-strategy + fill_form React-compatible
- Quiescence compuesta (bootstrap + interaction modes)
- Module lifecycle tracking + source transforms (Promise.finally, getAll safety)
- 415+ tests, 0 clippy warnings

### Lo que NO funciona
- SPA navigation post-interacción (React Router no llama navigate)
- ChatGPT PONG (sentinel/Turnstile challenge requiere crypto real)
- Rich text editing nativo (Selection/Range parcial)

---

## Gaps ordenados por impacto

### G1 — crypto.subtle real (BLOQUEA ChatGPT PONG)

**Problema**: `crypto.subtle.generateKey`, `sign`, `verify`, `importKey` son stubs que retornan datos falsos. ChatGPT's frontend necesita crypto real para resolver Turnstile VM challenge internamente. Sin esto, el sentinel flow queda pending y la conversation POST nunca se dispara.

**Lo que tenemos**: SHA-256 real (inline Rust op `op_pow_solve` + JS `_sha256`), `getRandomValues` real, `digest` real.

**Lo que falta**:
- `generateKey` — generar par ECDSA/RSA real
- `importKey` — importar key desde raw/jwk/spki
- `sign` — firmar con HMAC/ECDSA
- `verify` — verificar firma
- `exportKey` — exportar key a formato

**Fix propuesto**:
- Opción A: usar `ring` crate en Rust para operaciones crypto, exponer via ops
- Opción B: usar deno_core's built-in `deno_crypto` extension (si disponible en 0.311)
- Opción C: implementar solo HMAC-SHA256 (lo más probable que necesite Turnstile)

**Ficheros**:
- `crates/neo-runtime/src/ops.rs` — nuevos ops `op_crypto_generate_key`, `op_crypto_sign`, etc.
- `js/bootstrap.js` — conectar crypto.subtle methods a los ops

**Complejidad**: Media-alta
**Dependencias**: Ninguna
**Test**: ChatGPT sentinel/chat-requirements completa → conversation POST se dispara
**Estimado**: 1 sesión

---

### G2 — Streaming fetch body delivery (BLOQUEA SPA navigation)

**Problema**: Aunque el ReadableStream usa `pull()` con yields, el body ENTERO se descarga antes de que JS lo vea (op_fetch lee todo el body). El consumer (React/turbo-stream) necesita chunks MIENTRAS llegan de la red, no después.

**Lo que tenemos**: SSE detection en client.rs (read hasta [DONE]), pull-based ReadableStream en JS, `sse_events` array.

**Lo que falta**:
- Fetch que retorna HEADERS inmediatamente y BODY como stream
- Chunks entregados al JS ReadableStream mientras la red los envía
- El event loop debe correr entre chunks (microtask yields)

**Fix propuesto**:
- Cambiar `op_fetch` de "leer todo → retornar" a "retornar headers → stream body via nuevo op"
- Nuevo op `op_fetch_stream_read(stream_id)` que lee el siguiente chunk
- JS fetch wrapper usa estos ops para alimentar el ReadableStream en tiempo real

**Ficheros**:
- `crates/neo-runtime/src/ops.rs` — `op_fetch_start` (retorna headers + stream_id), `op_fetch_read_chunk` (retorna siguiente chunk)
- `crates/neo-http/src/client.rs` — retornar Response con body stream en vez de String
- `js/bootstrap.js` — fetch wrapper usa streaming ops

**Complejidad**: Alta
**Dependencias**: G1 (para ChatGPT, la conversation POST necesita crypto primero)
**Test**: `fetch(url).then(r => r.body.getReader().read())` retorna primer chunk antes de que el body complete
**Estimado**: 2 sesiones

---

### G3 — history.state sync + popstate contract (MEJORA SPA routing)

**Problema**: `history.pushState` actualiza location pero React Router puede necesitar que `window.history.state` se sincronice correctamente. Además, React Router's `createBrowserHistory` puede verificar el tipo real de `window.history`.

**Lo que tenemos**: pushState/replaceState shim, location sync, popstate dispatch en back/forward, PopStateEvent constructor.

**Lo que falta**:
- `window.history.state` debe reflejar el state pasado a pushState
- `window.history` debe pasar `instanceof` checks o al menos tener prototype correcto
- Verificar que React Router's `createBrowserHistory` funciona con nuestro shim

**Fix propuesto**:
- Añadir getter `state` a nuestro history object que retorna el state actual
- Verificar si happy-dom's Window tiene `history` propio que interfiere
- Test con React Router directamente

**Ficheros**: `js/browser_shim.js`
**Complejidad**: Baja
**Test**: `history.pushState({foo:1}, '', '/test'); history.state.foo === 1`
**Estimado**: 0.5 sesiones

---

### G4 — PerformanceObserver stub

**Problema**: Muchas apps modernas usan `PerformanceObserver` para telemetría. Sin él, código que hace `new PerformanceObserver(callback)` crashea.

**Lo que tenemos**: `performance.now()` funciona. No hay PerformanceObserver.

**Fix propuesto**:
```javascript
globalThis.PerformanceObserver = class PerformanceObserver {
    constructor(cb) { this._cb = cb; }
    observe() {}
    disconnect() {}
    takeRecords() { return []; }
    static supportedEntryTypes = [];
};
```

**Ficheros**: `js/bootstrap.js`
**Complejidad**: Trivial
**Estimado**: 5 minutos

---

### G5 — Selection/Range funcional (MEJORA rich text)

**Problema**: ProseMirror y otros editores ricos dependen de Selection y Range para tracking de caret, selección de texto, y operaciones de editing. Nuestras APIs existen pero no representan estado real.

**Lo que tenemos**: `Selection`, `Range`, `getSelection()` — constructors existen (de happy-dom). `execCommand('insertText')` parcialmente funciona.

**Lo que falta**:
- `getSelection()` debe reflejar el caret position real en contenteditable
- `Range` debe poder setearse en nodos específicos
- `execCommand('insertText')` debe insertar texto y actualizar Selection
- ProseMirror's `readDOMChange` necesita que Selection refleje los cambios

**Fix propuesto**:
- Para ProseMirror: ya tenemos el bridge via `view.dispatch(tr)` — no necesitamos Selection real
- Para otros editores: implementar Selection/Range tracking básico con un "virtual caret"
- El bridge por editor es más fiable que Selection/Range genérico

**Ficheros**: `js/browser_shim.js`, `crates/neo-engine/src/live_dom.rs`
**Complejidad**: Alta (Selection/Range genérico), Baja (bridge por editor)
**Estimado**: 1-2 sesiones para bridge, 4+ para Selection/Range real

---

### G6 — Layout stubs con valores plausibles

**Problema**: `getBoundingClientRect()` retorna `{top:0, left:0, width:0, height:0}`. Código que decide visibilidad, posición o layout basándose en estas APIs falla silenciosamente.

**Lo que tenemos**: APIs existen (happy-dom), retornan 0s o defaults.

**Lo que falta**:
- `offsetWidth/Height` con valores non-zero plausibles basados en contenido
- `getBoundingClientRect` con dimensiones estimadas
- `scrollHeight/scrollTop` básicos
- `matchMedia` que responda a breakpoints razonables

**Fix propuesto**:
- Heuristic sizing: block elements get viewport width, inline elements get text-based width
- `offsetWidth = 1920` para body, proporcionalmente para children
- `matchMedia('(min-width: 1024px)')` → true para desktop

**Ficheros**: `js/browser_shim.js`
**Complejidad**: Media
**Estimado**: 1 sesión

---

### G7 — Worker stub

**Problema**: Apps que usan Web Workers crashean en `new Worker(url)`.

**Fix propuesto**:
```javascript
globalThis.Worker = class Worker extends EventTarget {
    constructor() { super(); }
    postMessage() {}
    terminate() {}
};
```

**Complejidad**: Trivial (stub). Real Worker = muy complejo (nuevo V8 isolate).
**Estimado**: 5 minutos para stub

---

### G8 — FormData mejorado

**Problema**: Nuestro FormData stub es básico. Falta `entries()` iterator completo, `has()`, y construcción desde form element.

**Fix propuesto**: happy-dom ya tiene FormData — verificar y usar directamente.

**Complejidad**: Baja
**Estimado**: 0.5 sesiones

---

## Resumen por prioridad

### P0 — Sin esto no navega SPAs
| # | Gap | Coste | Impacto |
|---|---|---|---|
| G1 | crypto.subtle real | 1 sesión | Desbloquea ChatGPT + cualquier site con anti-bot |
| G2 | Streaming fetch body | 2 sesiones | Desbloquea SPA navigation post-interacción |
| G3 | history.state sync | 0.5 sesiones | Mejora React Router compat |

### P1 — Mejora significativa
| # | Gap | Coste | Impacto |
|---|---|---|---|
| G4 | PerformanceObserver | 5 min | Evita crashes telemetría |
| G5 | Selection/Range | 1-2 sesiones | Rich text editing |
| G6 | Layout stubs | 1 sesión | Visibilidad/responsive logic |

### P2 — Completitud
| # | Gap | Coste | Impacto |
|---|---|---|---|
| G7 | Worker stub | 5 min | Evita crashes en apps con workers |
| G8 | FormData mejorado | 0.5 sesiones | Mejor form handling |

### Total estimado: 6-8 sesiones para P0+P1

---

## Criterios de éxito

### Mercadona tienda
- Type CP → React onChange fires → navigation to categories view
- Categories visible after submit

### ChatGPT PONG
- Type "Di Pong" → send → conversation POST completes → assistant message renders → read text

### 10 sites
- All 10 render + interact (find_element, fill_form) sin crashes

### Formularios genéricos
- DuckDuckGo search: type query → submit → results page renders
- GitHub: search input → submit → results
- HN: login form detection + fill
