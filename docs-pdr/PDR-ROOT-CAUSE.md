# PDR — Root Cause: comparación Chrome vs NeoRender

## Datos medidos (Chrome real vs NeoRender, misma página ChatGPT)

| Propiedad | Chrome | NeoRender | Causa |
|---|---|---|---|
| `__reactRouterContext.state` | **{loaderData, actionData, errors}** | **null/undefined** | turbo-stream no decodifica |
| `history.constructor.name` | **History** | **Object** | plain object shim |
| `location.constructor.name` | **Location** | **Object** | plain object shim |
| DOM elements | 6933 | ~350 | state vacío = app no renderiza completa |
| Fibers | 6528 | ~329 | mismo |
| `Promise.finally` | function | function | ✅ |
| `defaultView === window` | true | true | ✅ |
| `popstate in window` | true | true | ✅ |

## Root cause #1: `__reactRouterContext.state` vacío

**Esto explica TODO**: sin state, React Router no tiene hydration data, no crea routes correctamente, no puede navegar.

En Chrome:
1. Server envía HTML con `<script>` que crea `__reactRouterContext` con `streamController`
2. Más `<script>` tags enqueue turbo-stream data al controller
3. `HydratedRouter` llama `decodeViaTurboStream(context.stream, window)`
4. El decode popula `context.state = {loaderData, actionData, errors}`
5. Router usa `state.loaderData` para hidratar las rutas

En NeoRender:
1. Scripts crean `__reactRouterContext` con streamController ✅
2. Scripts enqueue data al controller ✅
3. `HydratedRouter` intenta `decodeViaTurboStream(context.stream, window)`
4. **FALLA** — nuestro ReadableStream o el turbo-stream decoder no funciona correctamente
5. `context.state` queda null → Router no puede hidratar → no puede navegar

### Fix

Dos opciones:

**Opción A: Fix turbo-stream decode**
- El turbo-stream decoder (bundled en 16KB) necesita ReadableStream funcional
- Nuestro ReadableStream pull-based puede no ser compatible con el decoder
- Necesitamos verificar qué error produce el decode y fijarlo

**Opción B: Bypass turbo-stream — construir state manualmente**
- Los datos están en los `<script>` que hacen `streamController.enqueue(...)`
- Interceptar esos enqueue calls, acumular los raw strings
- Parsear turbo-stream format manualmente y setear `context.state`
- Ya tenemos el interceptor en browser_shim.js pero no funciona

**Opción C: Usar deno extensions**
- Activar `deno_web` extension que tiene ReadableStream REAL
- El ReadableStream real de deno es compatible con turbo-stream decoder
- También nos da History, Location, crypto.subtle REALES

## Root cause #2: History/Location no son clases reales

`history` es un plain object con methods. `location` es un plain object.
React Router puede hacer `history instanceof History` → false.

### Fix

**Opción A: Crear clases History/Location**
```javascript
class NeoHistory {
    constructor() { this._entries = []; this._index = 0; }
    get length() { return this._entries.length; }
    get state() { return this._entries[this._index]?.state || null; }
    pushState(state, title, url) { /* ... */ }
    replaceState(state, title, url) { /* ... */ }
    back() { /* ... dispatch popstate */ }
    forward() { /* ... dispatch popstate */ }
    go(n) { /* ... */ }
}
globalThis.history = new NeoHistory();
```

**Opción B: Usar deno_web extension**
- Deno's `deno_web` tiene History y Location implementados
- Activar la extension nos da las clases reales

## Recomendación: usar deno extensions

En vez de reimplementar todo, activar las extensions de deno que ya existen:

| Extension | Qué nos da | Esfuerzo |
|---|---|---|
| `deno_web` | History, Location, EventTarget, MessageChannel, TextEncoder, crypto | Bajo |
| `deno_url` | URL, URLSearchParams (reales, no polyfill) | Bajo |
| `deno_fetch` | fetch, Request, Response, Headers, ReadableStream | Medio |
| `deno_crypto` | crypto.subtle COMPLETO (no stubs) | Bajo |
| `deno_net` | TCP/TLS para fetch real | Alto |

### Por qué no lo hicimos antes

deno_core 0.311 es una versión antigua. Las extensions pueden tener incompatibilidades.
Pero el beneficio es enorme — History/Location/ReadableStream/crypto REALES.

### Plan de migración

1. **Investigar** qué extensions son compatibles con deno_core 0.311
2. **Activar** `deno_web` + `deno_url` + `deno_crypto` (las más fáciles)
3. **Verificar** que History, Location, ReadableStream funcionan
4. **Opcional**: activar `deno_fetch` si nuestro wrapper actual no basta
5. **Eliminar** polyfills que ya no necesitamos (URL, crypto, History, Location)

### Alternativa: cherry-pick solo lo que falta

Si activar extensions completas es complejo:
1. **Fix turbo-stream decode** — el ROOT CAUSE #1
2. **History/Location classes** — copiar de deno_web source (~200 líneas)
3. **ReadableStream de deno** — copiar implementación (~500 líneas)

## Acción inmediata (antes de migrar extensions)

### Fix turbo-stream state (ROOT CAUSE #1)

El state se puebla via turbo-stream decode del ReadableStream.
Nuestro interceptor en browser_shim.js captura los enqueue calls.
Pero el decode nunca se ejecuta porque el stream ya se consumió.

**Fix concreto**:
1. Interceptar los enqueue calls (ya lo hacemos)
2. Acumular el raw turbo-stream data
3. Después del `streamController.close()`, llamar `turboStream.decode(raw)`
4. Setear `context.state = decoded.value`
5. Verificar que `context.state.loaderData` existe

**Esto debería ser suficiente para desbloquear React Router navigation.**

## Gates

| Gate | Criterio |
|---|---|
| Gate 1 | `__reactRouterContext.state` tiene `loaderData` |
| Gate 2 | `history instanceof History` si React Router lo chequea |
| Gate 3 | Click send → pushState se llama → pathname cambia |
