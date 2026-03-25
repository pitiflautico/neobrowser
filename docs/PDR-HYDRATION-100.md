# PDR: Hydration 100% — Completar TODAS las piezas SPA

## Fecha: 24 March 2026

## Estado actual

NeoRender V2 tiene las piezas fundamentales pero happy-dom rompe por incompatibilidades internas (`#window`, `queueMicrotask`) y varias áreas están incompletas. El engine tiene DOS capas: **LiveDom (JS, buena)** y **neo-interact (Rust, limitada)**. LiveDom ya implementa React compat, events, focus — pero las excepciones de happy-dom impiden que los scripts de la página se ejecuten.

---

## MAPA COMPLETO: 26 ÁREAS × ESTADO × FIX

### 1. RED / FETCH — 9/10 ✅

| Item | Estado | Fix |
|---|---|---|
| HTTP client realista | ✅ | — |
| Cookies persistentes | ✅ | — |
| Redirects | ✅ | — |
| Cache básica | ✅ | — |
| Compression gzip/br/zstd | ✅ | — |
| CORS coherente | ⚠️ | Añadir preflight check en op_fetch cuando method != GET/POST simple |
| Headers browser-like | ✅ | — |
| TLS fingerprint | ✅ | Chrome 145 via wreq-util |
| Script/css/json/image/font fetch | ✅ | — |
| Preload/modulepreload/prefetch | ✅ | — |

**Esfuerzo fix: 0** (funcional)

---

### 2. HTML / PARSEADO — 5/5 ✅

Todo funciona. happy-dom + html5ever.

**Esfuerzo fix: 0**

---

### 3. DOM CORE — 12/12 ✅

Todo funciona via happy-dom. querySelector, classList, innerHTML, MutationObserver, etc.

**Esfuerzo fix: 0**

---

### 4. BOM / NAVEGACIÓN — 10/13

| Item | Estado | Fix |
|---|---|---|
| location | ✅ | — |
| history pushState/replaceState/back/forward | ✅ | — |
| popstate/hashchange | ✅ | — |
| navigator | ✅ | — |
| screen | STUB | Añadir width/height/availWidth/availHeight reales (1920x1080 default) |
| URL/URLSearchParams | ✅ | — |
| localStorage/sessionStorage | ⚠️ | Funciona in-memory, no persiste. Conectar a ops `op_storage_get/set` que YA existen |
| document.cookie | ✅ | — |
| performance.now | ✅ | — |
| crypto.getRandomValues | ✅ | — |
| crypto.subtle | ⚠️ | Solo digest(). Falta: sign/verify/encrypt/decrypt para JWT, HMAC. Implementar con `ring` crate en op |
| btoa/atob | ✅ | — |
| structuredClone | ✅ | — |

**Esfuerzo fix: 1 sesión** (crypto.subtle + storage persistence)

---

### 5. EVENT LOOP — 8/11

| Item | Estado | Fix |
|---|---|---|
| Microtasks correctas | ✅ | — |
| Promise jobs | ✅ | — |
| queueMicrotask | ✅ | — |
| Macrotasks | ✅ | — |
| setTimeout/setInterval/clear* | ✅ | — |
| MessageChannel | ⚠️ | bootstrap.js línea ~1574 — VERIFICAR si funciona. Si no: polyfill con Promise.resolve().then() para port.postMessage |
| requestAnimationFrame | ⚠️ | Alias setTimeout(16ms). Funcional pero timing no exacto. Aceptable. |
| requestIdleCallback | ✅ | Polyfill setTimeout(1ms) + timeRemaining() |
| Orden micro→render→macro | ✅ | — |
| Watchdogs y budgets | ✅ | — |
| Settle prematuro | ⚠️ | 3 consecutive quiet rounds. Mejorable: detectar pending route changes |

**Esfuerzo fix: 0.5 sesión** (MessageChannel verificar, settle mejorar)

---

### 6. SCRIPT EXECUTION — 8/8 ✅

Todo funciona: inline, external, defer, async, orden correcto, console.*, error handling, unhandledrejection.

**Esfuerzo fix: 0**

---

### 7. ESM / MÓDULOS — 10/12

| Item | Estado | Fix |
|---|---|---|
| import/export reales | ✅ | — |
| import dinámico | ✅ | — |
| Resolución relative/absolute | ✅ | — |
| Manifest chunk loading | ✅ | — |
| Module graph traversal | ✅ | Prefetch depth 3, 200 modules |
| Top-level await | ✅ | Async IIFE wrap |
| Module cache | ✅ | ScriptStore + V8 cache |
| Circular deps | ✅ | deno_core handles |
| Prefetch | ✅ | — |
| Bytecode cache | ✅ | V8 code cache |
| Side effects preservation | ✅ | — |
| No cortar event loop antes de imports encadenados | ⚠️ | Extended settle 3000ms pero Vite apps con 99 modules pueden necesitar más. Hacer el settle timeout proporcional al número de modules discovered |

**Esfuerzo fix: 0.5 sesión** (settle dinámico)

---

### 8. HIDRATACIÓN FRAMEWORKS — 3/7

| Item | Estado | Fix |
|---|---|---|
| React 18/19 hydrateRoot/createRoot | ⚠️ | Funciona si scripts ejecutan. El blocker es happy-dom, no React bridge |
| React synthetic events | ✅ | LiveDom dispatcha events correctamente |
| React _valueTracker | ✅ | LiveDom setElValue + reactNotifyChange ya implementado |
| Vue mounting/reactivity | ❌ | No hay bridge Vue. Fix: Vue usa `__VUE__` global — detectar y no interferir. Normalmente funciona si DOM+events son correctos |
| Svelte hydrate/mount | ❌ | Igual que Vue — debería funcionar si el DOM está correcto |
| SSR shell + client takeover | ✅ | DOM parsed → scripts ejecutan → framework monta |
| Detección "app realmente montada" | ❌ | Hoy: solo DOM node count delta. Fix: check `document.querySelector('[data-reactroot]')` o `#__next` o `#app` tiene contenido. Verificar que interactive elements tienen event listeners |

**Esfuerzo fix: 1 sesión** (mount detection, Vue/Svelte testing)

---

### 9. EVENT SYSTEM — 6/8

| Item | Estado | Fix |
|---|---|---|
| addEventListener/removeEventListener | ✅ | — |
| capture/bubble | ⚠️ | happy-dom lo soporta pero no verificado en todos los paths |
| stopPropagation/stopImmediatePropagation/preventDefault | ✅ | — |
| dispatchEvent | ✅ | — |
| Mouse/Keyboard/Input/Focus/Submit/PointerEvent | ✅ | Todos exportados + polyfills |
| composedPath | ⚠️ | Definido pero no testeado |
| target/currentTarget/relatedTarget | ✅ | — |
| Default actions básicas | ✅ | LiveDom fireClick maneja checkbox toggle, radio select, form submit, link nav |

**Esfuerzo fix: 0.5 sesión** (tests de capture/bubble + composedPath)

---

### 10. INPUTS / FORMULARIOS — 7/14

| Item | Estado | Fix |
|---|---|---|
| input.value/checked/selected/files | ✅ | LiveDom handles via setElValue |
| textarea/select/option | ✅ | LiveDom fireTypeText maneja select |
| defaultValue/defaultChecked | ⚠️ | happy-dom tiene pero interact no trackea |
| selectionStart/selectionEnd | ✅ | LiveDom con type guards (SEL_TYPES) |
| focus/blur | ✅ | LiveDom fireFocusChange dispatcha focusin/focusout/focus/blur |
| input/change/keydown/keyup/keypress/beforeinput/composition | ✅ | LiveDom fireTypeText dispatcha todos per-char |
| submit/reset | ⚠️ | LiveDom fireSubmit existe. Falta: reset event |
| FormData | ✅ | Polyfill en bootstrap |
| form.elements | ✅ | — |
| requestSubmit | ❌ | Fix: `HTMLFormElement.prototype.requestSubmit = function(btn) { this.submit(); }` en bootstrap |
| Constraint validation | ❌ | Fix: checkValidity/reportValidity ya están en browser_shim.js. Falta: wired to actual validation |
| name/value serialization | ✅ | collect_form_data en forms.rs |
| click button type=submit → form submit | ✅ | LiveDom fireClick detecta submit button → executeFormSubmit |
| Enter in input → submit | ✅ | LiveDom firePressKey('Enter') → finds closest form → submit |

**Esfuerzo fix: 0.5 sesión** (requestSubmit polyfill, constraint validation wire-up)

---

### 11. FOCUS / SELECCIÓN — 3/6

| Item | Estado | Fix |
|---|---|---|
| document.activeElement | ✅ | LiveDom fireFocusChange actualiza activeElement |
| focus management | ✅ | LiveDom focus dispatcha eventos |
| tabIndex | ⚠️ | Parseado, no usado para tab order. Low priority |
| Selection API | STUB | Range/Selection básicos. Fix: mínimo createRange + getRangeAt |
| scrollIntoView | ❌ | Fix: `Element.prototype.scrollIntoView = function() {}` no-op stub |
| Focused element correct before type | ✅ | LiveDom fireTypeText llama focus si activeElement !== el |

**Esfuerzo fix: 0.5 sesión** (scrollIntoView stub, Selection mínima)

---

### 12. LAYOUT / VISIBILITY — 1/6

| Item | Estado | Fix |
|---|---|---|
| getBoundingClientRect | STUB | Retorna fixed rect. Mejorable: estimar por posición en DOM |
| offsetWidth/offsetHeight | STUB | Fix: return non-zero defaults (width=auto→parentWidth, height=lineHeight) |
| getComputedStyle | ⚠️ | Proxy con Proxy handler. Funcional para reads básicos |
| hidden/disabled/aria-hidden | ⚠️ | Parseados como attributes. LiveDom isVisible() check offsetParent |
| IntersectionObserver | STUB | No fires. Fix: fire all observers once after settle con `isIntersecting: true` |
| ResizeObserver | ⚠️ | Tracking + synthetic fire. Funcional |

**Esfuerzo fix: 0.5 sesión** (IO fire, offset defaults)

---

### 13. WEB APIS MODERNAS — 9/13

| Item | Estado | Fix |
|---|---|---|
| fetch/Request/Response/Headers | ✅ | — |
| AbortController/AbortSignal | ✅ | — |
| ReadableStream | ✅ | NeoReadableStream polyfill forzado |
| WritableStream/TransformStream | STUB | Existen pero pipe no funcional. Low priority |
| TextEncoder/TextDecoder | ✅ | — |
| Blob/File | ✅ | — |
| URL.createObjectURL | STUB | Retorna input. Fix: blob: URL scheme |
| CustomEvent/EventTarget | ✅ | — |
| DOMParser/XMLSerializer | ✅ | — |
| BroadcastChannel | STUB | No-op. Aceptable para single-context |
| Worker/SharedWorker | STUB | No-op. Aceptable |
| WebSocket | ❌ | Fix: polyfill con fetch long-polling fallback. O: real WebSocket via op |
| EventSource | ✅ | NeoEventSource polyfill con SSE parsing |
| postMessage | ⚠️ | window.postMessage existe pero no cross-context |

**Esfuerzo fix: 1 sesión** (WebSocket op, WritableStream)

---

### 14. CSS / RECURSOS — 0/5 (ACEPTABLE)

No tenemos CSS engine y no lo necesitamos. Link rel=stylesheet se parsea, no se evalúa. CSS no bloquea hydration en nuestro modelo.

**Esfuerzo fix: 0**

---

### 15. OBSERVERS — 2/4

| Item | Estado | Fix |
|---|---|---|
| MutationObserver fires | ✅ | Global MO tracking para quiescence |
| ResizeObserver fires | ⚠️ | Synthetic fire after settle |
| IntersectionObserver fires | ❌ | Fix: fire once after settle con `isIntersecting: true, intersectionRatio: 1` |
| PerformanceObserver | STUB | No-op. Aceptable |

**Esfuerzo fix: 0.5 sesión** (IO fire)

---

### 16. EDITORES RICOS — 5/11

| Item | Estado | Fix |
|---|---|---|
| contenteditable | ⚠️ | LiveDom detecta y usa textContent. Funcional básico |
| execCommand | ❌ | Fix: `document.execCommand = function() { return false; }` stub |
| Selection/Range | STUB | Mínimo. Fix: createRange que retorna rango funcional |
| clipboard events | ⚠️ | ClipboardEvent definido |
| ProseMirror bridge | ✅ | LiveDom typeInEditor detecta ProseMirror view + dispatch tr |
| Lexical bridge | ✅ | LiveDom typeInEditor $getRoot/$createTextNode |
| Slate bridge | ⚠️ | textContent + compositionend |
| CodeMirror bridge | ✅ | LiveDom cmView.dispatch |
| Quill bridge | ✅ | LiveDom editor.setText |
| Composition events | ⚠️ | CompositionEvent definido, dispatched en contenteditable |
| paste | ❌ | Fix: sintetizar paste event con clipboardData |

**Esfuerzo fix: 1 sesión** (paste, execCommand, Selection)

---

### 17. ROUTING SPA — 4/6

| Item | Estado | Fix |
|---|---|---|
| Click `<a>` interception | ✅ | LiveDom fireClick detecta `<a>` → op_navigation_request |
| preventDefault real | ✅ | — |
| pushState/replaceState coherent | ✅ | Dispatch popstate en ambos |
| history/location listeners | ✅ | — |
| Nav post-submit/post-click async | ⚠️ | pump_after_interaction existe pero 10s timeout fijo |
| No settle before router resolves | ❌ | Fix: detectar pushState/replaceState DURANTE settle → reset quiet counter |

**Esfuerzo fix: 0.5 sesión** (route-aware settle)

---

### 18. ASYNC DURING HYDRATION — 5/7

| Item | Estado | Fix |
|---|---|---|
| Pending fetch tracking | ✅ | __fetchLog + quiescence |
| Pending module loads tracking | ✅ | ModuleTracker atomics |
| Pending timers tracking | ✅ | __timerMap.size |
| Pending promises | ⚠️ | Via modules/callbacks, no full graph |
| Network-idle heuristic | ✅ | pending_fetches == 0 in quiescence |
| App-idle heuristic | ✅ | 3 consecutive quiet rounds |
| Settle strategy | ⚠️ | Funcional pero puede mejorar: settle timeout proporcional a modules discovered |

**Esfuerzo fix: 0.5 sesión** (dynamic settle)

---

### 19. APP READY DETECTION — 4/8

| Item | Estado | Fix |
|---|---|---|
| DOM changed post-SSR | ✅ | Node count delta |
| Listeners effective | ❌ | Fix: eval `document.querySelectorAll('[onclick],[data-onclick]').length` o check __reactProps |
| Buttons enabled | ⚠️ | disabled parsed pero no checked |
| Forms respond | ⚠️ | Forms collected, response not verified |
| Router navigates | ❌ | Fix: check pushState was called during settle |
| Inputs update state | ✅ | Via _valueTracker in LiveDom |
| Post-hydration fetches complete | ✅ | Quiescence pending_fetches |
| No pending errors | ⚠️ | Errors collected pero no validated |

**Esfuerzo fix: 0.5 sesión** (listener check, router detect)

---

### 20. INSTRUMENTACIÓN — 8/10

| Item | Estado | Fix |
|---|---|---|
| Script load logs | ✅ | — |
| Module load/fail logs | ✅ | — |
| JS exceptions | ✅ | — |
| Unhandled rejections | ✅ | — |
| Pending fetch count | ✅ | — |
| Pending timer count | ✅ | — |
| Event listeners count | ❌ | Fix: wrap addEventListener para contar. Low priority |
| DOM changes | ✅ | __domMutations counter |
| Settle reason codes | ⚠️ | Básicos. Mejorar con reason enum |
| Per-action trace | ✅ | — |

**Esfuerzo fix: 0.5 sesión** (listener count, reason codes)

---

### 21. POLYFILLS / STUBS — 2/6

| Item | Estado | Fix |
|---|---|---|
| ViewTransition | STUB | No-op. Aceptable |
| Object.groupBy | ❌ | Fix: 3-line polyfill |
| pipeThrough | ⚠️ | Patch existe |
| matchMedia | ⚠️ | Hardcoded false. Mejorable: accept width/height config |
| visualViewport | ❌ | Fix: stub con width/height fijos |
| TrustedTypes | ❌ | Fix: `globalThis.trustedTypes = { createPolicy: () => ({ createHTML: s=>s, createScript: s=>s }) }` |

**Esfuerzo fix: 0.5 sesión**

---

### 22. SECURITY / CSP — 1/5

| Item | Estado | Fix |
|---|---|---|
| CSP script-src | ❌ | No prioritario — somos headless, no enforcement necesario |
| Nonces | ❌ | Idem |
| Integrity | ❌ | Idem |
| crossOrigin | ⚠️ | Parseado |
| origin/base URL correct | ✅ | — |

**Esfuerzo fix: 0** (no prioritario para AI browser)

---

### 23. RENDER / SETTLE — 8/10

| Item | Estado | Fix |
|---|---|---|
| Bootstrap | ✅ | — |
| Execute HTML scripts | ✅ | — |
| Load module graph | ✅ | — |
| Run event loop | ✅ | — |
| Drain microtasks | ✅ | — |
| Pump platform tasks | ✅ | — |
| Wait network idle | ✅ | Quiescence pending_fetches |
| Wait DOM stability | ✅ | 3 quiet rounds |
| Validate hydration markers | ❌ | Fix: check root container has content, React __reactFiber exists |
| Declare "ready" | ✅ | PageState::Complete |

**Esfuerzo fix: 0.5 sesión** (hydration validation)

---

### 24. ACCIONES SINTÉTICAS — 6/7

| Item | Estado | Fix |
|---|---|---|
| Click realista | ✅ | LiveDom fireClick con mousedown/mouseup/click + default actions |
| Type realista | ✅ | LiveDom fireTypeText per-char con keydown/keypress/beforeinput/input/keyup |
| Paste | ❌ | Fix: sintetizar paste event con ClipboardEvent + data |
| keypress Enter/Tab/Escape | ✅ | LiveDom firePressKey |
| change/input dispatch coherent | ✅ | LiveDom dispatcha en type y on blur |
| Submit via requestSubmit or click | ✅ | LiveDom fireClick on submit button → executeFormSubmit |
| Not just mutate value | ✅ | LiveDom usa setElValue + event chain |

**Esfuerzo fix: 0.5 sesión** (paste)

---

### 25. FORM COMPAT — 5/7

| Item | Estado | Fix |
|---|---|---|
| React controlled inputs | ✅ | LiveDom _valueTracker + reactNotifyChange + Strategy 3 (fiber) |
| _valueTracker | ✅ | setElValue sets tracker to old value before update |
| onInput/onChange mapping | ✅ | InputEvent dispatched per-char + Strategy 2 direct __reactProps call |
| Checked radios/checkboxes | ✅ | LiveDom fireClick toggles + group deselect |
| Multi-select | ✅ | LiveDom fireTypeText handles select-multiple |
| File input | ❌ | Fix: file input stub que acepta paths |
| disabled/readonly | ⚠️ | Parseados pero no enforced en interact |

**Esfuerzo fix: 0.5 sesión** (file input, disabled enforcement)

---

### 26. COMMON BREAKERS — RESOLUTION

| Breaker | Estado | Resuelto por |
|---|---|---|
| Microtasks no drenadas | ✅ | V8 kAuto + queueMicrotask budget |
| Dynamic import incompleto | ✅ | ModuleTracker + extended settle |
| MessageChannel ausente | ⚠️ | Verificar implementación existente |
| rAF ausente | ✅ | setTimeout(16ms) polyfill |
| ResizeObserver/IntersectionObserver ausentes | ⚠️ | RO: synthetic fire. IO: FALTA fire |
| Focus incorrecto | ✅ | LiveDom fireFocusChange |
| Input events mal disparados | ✅ | LiveDom fireTypeText per-char |
| Value sin tracker | ✅ | LiveDom setElValue + _valueTracker |
| Settle prematuro | ⚠️ | 3 quiet rounds pero route-aware falta |
| Excepción silenciosa durante mount | ⚠️ | Errors collected, can improve |
| Pending fetch no observado | ✅ | Quiescence tracking |
| Routing post-click no esperado | ⚠️ | pump_after_interaction 10s pero no route-aware |

---

## BLOCKER #0: HAPPY-DOM INTERNALS

**Antes de todo lo demás**, hay que estabilizar happy-dom. Los errores actuales:

1. `this.#window.queueMicrotask is not a function` — **PARCHEADO** con optional chaining
2. `MutationObserver constructed outside Window context` — **PARCHEADO** con fake window fallback
3. Posibles más errores similares en otros constructores de happy-dom

**Fix definitivo**: Auditar TODOS los `this[window]` y `this.#window` checks en happy-dom.bundle.js y parchar los que fallan fuera de Window context. Alternativa: migrar a happy-dom-without-node (fork que no requiere Window context).

**Esfuerzo: 1 sesión**

---

## PLAN DE IMPLEMENTACIÓN POR SESIONES

### Sesión 1: Happy-dom stability
- Auditar y parchar TODOS los `this[window]` / `this.#window` fails
- O migrar a happy-dom-without-node
- Test: sesame, factorial, ChatGPT cargan sin errors de bootstrap

### Sesión 2: IntersectionObserver + settle inteligente
- IO fire once after settle
- Settle timeout proporcional a modules (más modules → más tiempo)
- Route-aware settle (detectar pushState → reset quiet counter)
- Test: SPAs con lazy loading ven contenido

### Sesión 3: requestSubmit + paste + polyfills menores
- requestSubmit polyfill
- paste event synthesis
- Object.groupBy, visualViewport, TrustedTypes, execCommand stubs
- scrollIntoView no-op
- Test: forms con requestSubmit funcionan

### Sesión 4: crypto.subtle + WebSocket
- crypto.subtle real via `ring` crate (sign/verify/importKey/exportKey)
- WebSocket op (real TCP via tokio-tungstenite)
- Test: ChatGPT sentinel, JWT verification

### Sesión 5: Hydration validation + mount detection
- Check root containers post-settle
- React __reactFiber detection
- Vue __VUE__ detection
- Listener effectiveness check
- Test: detección correcta de "app montada" vs "shell vacío"

### Sesión 6: Testing + hardening
- Test suite contra 10 sites reales
- Regression tests para cada fix
- Performance benchmarks
- Document remaining gaps

---

## RESUMEN CUANTITATIVO

| Categoría | Items | YES | PARTIAL | STUB | NO |
|---|---|---|---|---|---|
| 1. Red/Fetch | 10 | 9 | 1 | 0 | 0 |
| 2. HTML | 5 | 5 | 0 | 0 | 0 |
| 3. DOM Core | 12 | 12 | 0 | 0 | 0 |
| 4. BOM/Nav | 13 | 10 | 2 | 1 | 0 |
| 5. Event Loop | 11 | 8 | 2 | 1 | 0 |
| 6. Scripts | 8 | 8 | 0 | 0 | 0 |
| 7. ESM | 12 | 10 | 1 | 0 | 0 |
| 8. Frameworks | 7 | 3 | 2 | 0 | 2 |
| 9. Events | 8 | 6 | 2 | 0 | 0 |
| 10. Inputs | 14 | 10 | 1 | 0 | 3 |
| 11. Focus | 6 | 3 | 1 | 1 | 1 |
| 12. Layout | 6 | 1 | 2 | 2 | 1 |
| 13. Web APIs | 14 | 9 | 2 | 2 | 1 |
| 14. CSS | 5 | 0 | 4 | 0 | 0 |
| 15. Observers | 4 | 2 | 1 | 1 | 0 |
| 16. Editores | 11 | 5 | 3 | 1 | 2 |
| 17. Routing | 6 | 4 | 1 | 0 | 1 |
| 18. Async | 7 | 5 | 2 | 0 | 0 |
| 19. Ready | 8 | 4 | 3 | 0 | 1 |
| 20. Instrumentation | 10 | 8 | 1 | 0 | 1 |
| 21. Polyfills | 6 | 2 | 2 | 1 | 1 |
| 22. Security | 5 | 1 | 1 | 0 | 3 |
| 23. Settle | 10 | 8 | 0 | 0 | 1 |
| 24. Actions | 7 | 6 | 0 | 0 | 1 |
| 25. Forms | 7 | 5 | 1 | 0 | 1 |
| 26. Breakers | 12 | 7 | 4 | 0 | 0 |
| **TOTAL** | **224** | **151 (67%)** | **39 (17%)** | **10 (4%)** | **20 (9%)** |

### Target: 200/224 (89%) — suficiente para que SPAs reales funcionen

Los 20 NO son:
- 3 CSP (no prioritario)
- 2 framework detection (Vue, "app montada")
- 3 inputs (requestSubmit, constraint validation, file input)
- 1 focus (scrollIntoView)
- 1 layout (IO fire)
- 1 web API (WebSocket)
- 2 editores (execCommand, paste)
- 1 routing (route-aware settle)
- 1 polyfill (TrustedTypes)
- 1 ready (listener check)
- 1 settle (hydration markers)
- 1 instrumentation (listener count)
- 1 breakers resolution
- 1 polyfill (visualViewport)

**6 sesiones para completar. Prioridad: sesión 1 (happy-dom stability) desbloquea todo lo demás.**
