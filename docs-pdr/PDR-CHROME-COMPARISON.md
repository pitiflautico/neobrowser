# PDR — Chrome vs NeoRender: Análisis completo y plan de cierre

## Datos medidos (23 marzo 2026)

### Comparación exacta: misma página ChatGPT

| Propiedad | Chrome | NeoRender | Gap |
|---|---|---|---|
| `state = {loaderData, actionData, errors}` | ✅ | ✅ | — |
| `loaderData.root` keys | 10+ keys (dd, disablePrefetch, ...) | 10+ keys (mismos) | ✅ |
| `loaderData["routes/_conversation"]` | ✅ con datos | ✅ con datos | ✅ |
| `manifest.routes` | 100+ rutas | 100+ rutas | ✅ |
| `routeModuleKeys` | root, _conversation, _conversation.c.$id | root, _conversation, _conversation._index | **❌ DIFERENTE** |
| `history.state` | `{usr:null, key:"default", idx:1, masked:undefined}` | `{idx:0}` | **❌ FALTA usr, key** |
| `location.href` | `/c/69c0229f-...` | `/` | **❌ NO NAVEGA** |
| DOM elements | 6933 | 360 | **❌ 19x menos** |
| React fibers | 6528 | 335 | **❌ 19x menos** |
| `defaultView === window` | true | true | ✅ |
| `history instanceof History` | true | ? (removed) | ⚠️ |
| `Promise.finally` | function | function | ✅ |
| `crypto.subtle.generateKey` | function | function | ✅ |
| `PerformanceObserver` | function | function | ✅ |
| `body.offsetWidth` | 1920 | 1920 | ✅ |

### Lo que ya matchea (22/25 APIs)
- Todas las Web APIs: fetch, ReadableStream, EventSource, AbortController
- Todos los observers: MutationObserver, ResizeObserver, IntersectionObserver
- Crypto: digest, generateKey, sign, verify (HMAC-SHA-512 real)
- Layout: getBoundingClientRect, offsetWidth, matchMedia
- Storage: localStorage, sessionStorage
- Events: CompositionEvent, PopStateEvent, CustomEvent
- Timers: setTimeout, setInterval, MessageChannel, rAF, rIC

---

## Los 3 gaps que explican el 95% de la diferencia

### GAP 1: `history.state` incompleto (ROOT CAUSE)

**Chrome**: `history.state = {usr: null, key: "default", idx: 1, masked: undefined}`

**NeoRender**: `history.state = {idx: 0}` (o `null` en initial load)

**Por qué importa**: React Router's `createBrowserHistory` hace:
```javascript
let index = getIndex(); // reads history.state.idx
if (index == null) {
    index = 0;
    globalHistory.replaceState({ ...globalHistory.state, idx: index }, "");
}
```

Y `createBrowserLocation` hace:
```javascript
let maskedLocation = (globalHistory.state as HistoryState)?.masked;
let { pathname, search, hash } = maskedLocation || window.location;
// ...
(globalHistory.state && globalHistory.state.usr) || null,
(globalHistory.state && globalHistory.state.key) || "default",
```

Sin `usr` y `key`, React Router crea la location con defaults. Esto FUNCIONA para initial load. El problema es que React Router calls `replaceState({...state, idx: 0}, "")` que en Chrome setea `{usr:null, key:"default", idx:0}`, pero en nuestro engine puede fallar o no persistir correctamente.

**Fix**: Verificar que nuestro `replaceState` merge works: `{...history.state, idx:0}` debe producir `{usr:null, key:"default", idx:0}`.

### GAP 2: React Router no completa hydration del client tree

**Chrome**: 6528 fibers — React mounted TODOS los componentes
**NeoRender**: 335 fibers — React solo montó el shell SSR

**Por qué**: React Router's `HydratedRouter` hace:
1. Lee state.loaderData ✅
2. Crea routes con `createClientRoutes(manifest, state, ssr)` ✅
3. Crea router con `createRouter({hydrationData: ...})` ✅
4. `RouterProvider` renders `<Routes>` tree
5. Components mount, useEffect fires, data loads
6. Client-side hydration completes — 6528 fibers

Nuestro step 4-6 no completa. Posibles causas:
- El router se crea pero encounters un error durante render
- React error #418 (hydration mismatch) causes bailout
- Un component throws durante mount
- El event loop no drena los effects

**Fix**: Instrumentar qué error React lanza durante hydration. El #418 error ya lo vemos pero React debería recuperarse via client-side render. Si no se recupera, hay un error más profundo.

### GAP 3: URL permanece en `/` (no navega a `/c/<id>`)

**Chrome**: Está en `/c/69c0229f-...` (navegó a una conversación)
**NeoRender**: Está en `/` (homepage)

**Nota**: Esto es PARCIALMENTE esperado — Chrome ya tiene una sesión con conversación abierta, NeoRender abre fresh. La comparación de URLs no es justa.

Pero el pattern de "no navegar después de interacción" SÍ es un gap real que afecta a todas las SPAs.

---

## Root cause analysis: por qué 335 fibers y no 6528

### Hipótesis 1: React #418 hydration mismatch causa bailout total (70% probable)

React ve el SSR HTML, intenta hidratar, encuentra un mismatch (texto, atributos, estructura), lanza #418, y en producción hace recovery via client-side render. Pero si el recovery TAMBIÉN falla (por ejemplo, por un missing API durante render), React queda con el shell parcial.

**Verificación**: Interceptar `console.error` durante hydration y ver qué mismatch reporta.

### Hipótesis 2: Un effect/useEffect durante mount hace fetch que nunca resuelve (20%)

Si un component llama `fetch` en useEffect y nuestro streaming fetch no retorna, el component queda en loading state forever. Otros components que dependen de ese data no montan.

**Verificación**: Contar fetch_start vs fetch_end. Si hay más starts que ends, hay fetches colgados.

### Hipótesis 3: El turbo-stream decoded state tiene datos parciales (10%)

Nuestro manual `unflatten()` decoder puede no resolver todas las references correctamente, especialmente nested objects, arrays, y special values. Si `loaderData.root` tiene campos con `undefined` donde Chrome tiene datos reales, los components que leen esos campos crashean.

**Verificación**: Comparar `loaderData.root` campo por campo con Chrome.

---

## Plan de acción (priorizado)

### Fase 1: Diagnóstico exacto (1 sesión)

1. **Comparar loaderData campo por campo**
   - Chrome: `JSON.stringify(state.loaderData.root)` (cada campo)
   - NeoRender: igual
   - Encontrar qué campos difieren

2. **Contar fetches colgados**
   - `__neo_fetchPending()` durante y después de hydration
   - Si >0, identificar qué URLs

3. **Capturar errores React silenciosos**
   - Override `console.error` ANTES de hydration
   - Capturar el #418 error detail
   - Ver si React intenta client-side recovery

4. **Verificar history.state format**
   - Después de `HydratedRouter` init: `JSON.stringify(history.state)`
   - Comparar con Chrome

### Fase 2: Fix exacto según diagnóstico (1-2 sesiones)

Dependiendo de lo que Fase 1 revele:

**Si es loaderData parcial**: Fix el `unflatten()` decoder para manejar nested refs, arrays, Dates, Sets, Maps, RegExps (turbo-stream supports these).

**Si es fetches colgados**: Add timeout/cleanup for streaming fetches during settle. O revert a `op_fetch` (complete) para todo excepto SSE explícito (ya implementado parcialmente).

**Si es error React silencioso**: Provide the missing API/polyfill que causa el error. Likely candidatos: missing CSS APIs, ResizeObserver callbacks, IntersectionObserver timing.

**Si es history.state format**: Ensure `replaceState` produces `{usr, key, idx}` format that React Router expects. Pre-populate history.state during bootstrap.

### Fase 3: Verificación (0.5 sesiones)

- 335 → >3000 fibers
- ProseMirror editor functional
- Send button enables after type
- Conversation API fires after click
- Navigation to `/c/<id>` occurs
- Assistant message renders

---

## Datos de referencia para la comparación

### Chrome loaderData.root keys (completo)
```
dd, disablePrefetch, shouldPrefetchAccount, shouldPrefetchUser,
shouldPrefetchSystemHints, promoteCss, disableStream, ...
```

### Chrome history.state
```json
{
    "usr": null,
    "key": "default",
    "idx": 1,
    "masked": undefined
}
```

### Chrome routeModuleKeys
```
root, routes/_conversation, routes/_conversation.c.$conversationId
```

### NeoRender routeModuleKeys
```
root, routes/_conversation, routes/_conversation._index
```

La diferencia: Chrome tiene `routes/_conversation.c.$conversationId` porque está en una conversación. NeoRender tiene `routes/_conversation._index` porque está en el index (`/`).

---

## Criterio de éxito

| Métrica | Ahora | Objetivo | Chrome |
|---|---|---|---|
| DOM elements | 360 | >3000 | 6933 |
| React fibers | 335 | >3000 | 6528 |
| loaderData match | parcial | completo | referencia |
| history.state format | {idx} | {usr,key,idx} | {usr,key,idx,masked} |
| Editor renders | ✅ | ✅ | ✅ |
| Navigation works | ❌ | ✅ | ✅ |
