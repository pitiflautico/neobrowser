# PDR: Anatomía de un navegador — Lo que somos vs lo que debemos ser

## Fecha: 24 March 2026

## Qué hace un browser real cuando el usuario escribe una URL y pulsa Enter

### FASE 1: RESOLUCIÓN Y CONEXIÓN

```
URL → DNS → TCP → TLS → HTTP
```

| Paso | Browser real | NeoRender | Falta |
|---|---|---|---|
| URL parsing | ✅ url::Url | ✅ | — |
| DNS resolve | OS resolver | ✅ wreq delega al OS | — |
| TCP connect | Pool de conexiones | ✅ wreq pool | — |
| TLS handshake | BoringSSL nativo | ⚠️ wreq BoringSSL pero fingerprint no pasa Cloudflare | TLS fingerprint real |
| HTTP/2 negotiation | ALPN + SETTINGS frames | ✅ wreq HTTP/2 | — |
| Cookie injection | Cookie jar → header | ✅ SqliteCookieStore → header | — |
| Redirect following | 302/301/307/308 auto | ✅ Policy::limited(10) | — |
| Cache check | disk/memory cache | ✅ DiskCache con ETag/Last-Modified | — |

**Veredicto: 7/8. TLS fingerprint es el gap.**

---

### FASE 2: RESPONSE Y PARSING

```
HTTP Response → Body streaming → HTML tokenizer → DOM tree
```

| Paso | Browser real | NeoRender | Falta |
|---|---|---|---|
| Response headers parse | Streaming headers | ✅ | — |
| Set-Cookie store | Almacena cookies | ✅ SqliteCookieStore | — |
| Content-Encoding decompress | gzip/br/zstd auto | ⚠️ **Bug: wreq devuelve 0 bytes con algunos servers brotli** | **BLOCKER** |
| Body streaming | Incremental read | ⚠️ Lee todo el body de golpe | Streaming body |
| HTML tokenizer | Incremental (SAX-like) | ❌ happy-dom parsea todo de golpe con `document.write()` | Incremental parsing |
| Speculative parsing | Escanea scripts/css ANTES de ejecutar | ❌ No hay speculative parsing | No crítico |
| DOM construction | Incremental durante parsing | ✅ happy-dom construye DOM completo | — |
| `<base>` URL resolution | Aplica a todos los URLs relativos | ✅ page_origin en module loader | — |

**Veredicto: 5/8. Brotli bug es grave. Streaming e incremental parsing son nice-to-have.**

---

### FASE 3: RESOURCE LOADING

Aquí es donde un browser real brilla y nosotros fallamos.

```
HTML parsed → descubrir recursos → fetch en paralelo → procesar
```

#### Lo que un browser carga en paralelo:

| Recurso | Browser real | NeoRender | Falta |
|---|---|---|---|
| `<link rel="stylesheet">` | Fetch + parse CSS → CSSOM | ❌ Ignorado | No tenemos CSSOM |
| `<link rel="preload">` | Fetch anticipado | ⚠️ Fetch pero no procesamos | — |
| `<link rel="modulepreload">` | Fetch + parse + instantiate module | ⚠️ Fetch pero no resolvemos deps transitivas | **Prefetch recursivo** |
| `<script>` blocking | Fetch → ejecutar → bloquea parser | ✅ | — |
| `<script defer>` | Fetch en paralelo → ejecutar después de parse | ✅ Group 2 | — |
| `<script async>` | Fetch → ejecutar cuando listo | ✅ Group 3 | — |
| `<script type="module">` | Fetch + resolve graph + execute in dependency order | ⚠️ **Ejecutamos en document order, no en dependency order** | **Module graph evaluation** |
| `<img>` | Fetch → decode → layout slot | ❌ No fetched | No necesario |
| `@font-face` | Fetch → register → text re-layout | ❌ No fetched | No necesario |
| favicon | Fetch → tab icon | ❌ No fetched | No necesario |

**Veredicto: 3/10. Pero 5 de los que faltan son imágenes/CSS/fonts que no necesitamos. Los 2 gaps reales: module graph order y modulepreload recursivo.**

---

### FASE 4: CSS (No aplica a nosotros — somos headless)

Un browser real:
1. Parsea CSS → CSSOM
2. Cascade: author + user-agent + inherited styles
3. Computed values: `getComputedStyle()` devuelve valores reales
4. Layout: box model, positioning, flex, grid
5. Paint: pixels
6. Compositing: layers, z-index

Nosotros: **Nada de esto.** Y eso tiene consecuencias:

| API que depende de CSS/Layout | Browser real | NeoRender | Impacto |
|---|---|---|---|
| `getComputedStyle()` | Valores reales | Proxy que devuelve '' | Frameworks que checkan visibility/display |
| `getBoundingClientRect()` | Posición y tamaño reales | `{x:0,y:0,width:0,height:0}` | Drag&drop, scroll-into-view, lazy loading |
| `offsetWidth/offsetHeight` | Dimensiones reales | 0 | Checks de visibilidad (`if (el.offsetWidth > 0)`) |
| `IntersectionObserver` | Fires cuando elemento entra en viewport | Fire sintético post-settle | Lazy loading, infinite scroll |
| `ResizeObserver` | Fires cuando elemento cambia tamaño | Fire sintético post-settle | Responsive components |
| `window.innerWidth/innerHeight` | Dimensiones reales del viewport | Hardcoded 1024×768 | Responsive design breakpoints |
| `matchMedia()` | Evalúa media queries reales | `{matches: false}` siempre | Mobile-first CSS → JS |

**Veredicto: No tenemos CSS engine y no vamos a tenerlo. Pero necesitamos mejores stubs que no devuelvan 0 — muchos frameworks hacen `if (el.offsetWidth === 0) return // hidden`.**

---

### FASE 5: SCRIPT EXECUTION — EL CORAZÓN

Aquí es donde pasamos el 90% del tiempo. Un browser ejecuta JS en un contexto muy específico:

#### 5.1 El Environment

| Qué | Browser real | NeoRender | Falta |
|---|---|---|---|
| V8 isolate | Uno por tab/renderer process | ✅ Uno por sesión (recreado cross-origin) | — |
| Global scope | `window === globalThis === self` | ✅ | — |
| `document` | Documento HTML live | ✅ happy-dom document | — |
| `navigator.userAgent` | UA real | ✅ Chrome 145 | — |
| `location` | URL real con pushState | ✅ Proxy con popstate | — |
| `history` | Real con navegación | ✅ pushState/replaceState/back/forward | — |
| `performance.now()` | HR timer | ✅ | — |
| `crypto.subtle` | WebCrypto real | ⚠️ Stubs (sign/verify son fake) | **No real crypto** |
| `localStorage/sessionStorage` | Persistente | ⚠️ In-memory, no persiste | |
| `fetch()` | Real HTTP con cookies | ✅ Via op_fetch con cookie injection | — |

#### 5.2 El Event Loop

**ESTO ES LO MÁS IMPORTANTE Y DONDE MÁS FALLAMOS.**

Un browser real tiene este ciclo:

```
while (true) {
  1. Pick oldest macrotask from queue (setTimeout, IO, click, etc.)
  2. Execute it
  3. Drain ALL microtasks (Promise.then, queueMicrotask, MutationObserver)
  4. If it's time to render:
     a. Run requestAnimationFrame callbacks
     b. Run IntersectionObserver callbacks
     c. Run ResizeObserver callbacks
     d. Update rendering (layout + paint)
  5. If idle: run requestIdleCallback
}
```

| Step | Browser | NeoRender | Falta |
|---|---|---|---|
| Macrotask queue | Real queue con prioridades | ✅ deno_core event loop | — |
| Microtask drain | Después de CADA macrotask | ✅ V8 kExplicit + manual checkpoint | — |
| rAF | Antes de render, ~16ms | ⚠️ setTimeout(16ms) — timing incorrecto | No se ejecuta en el render step |
| IO callbacks (MutationObserver) | Después de microtasks | ⚠️ MutationObserver shim — no fires en el momento correcto | **MO timing** |
| rIC | Cuando idle | ✅ setTimeout(1ms) stub | — |
| MessageChannel | Macrotask delivery | ⚠️ Implementado pero no verificado | |
| `postMessage` | Macrotask | ⚠️ Existe pero no cross-context | |

**El problema real del event loop**: No es que falten APIs. Es que **el timing no es correcto**. En un browser real:

1. Script ejecuta → microtasks drenan → MO fires → rAF fires → render
2. setTimeout(0) → siguiente iteration del loop

En nosotros:
1. Script ejecuta → microtasks drenan → ... ¿y luego?
2. `run_until_settled()` pumpa el loop con polling pero **no simula el render step**
3. rAF nunca corre en el momento correcto
4. MO fires cuando happy-dom quiere, no en el momento spec

#### 5.3 Module Loading

| Paso | Browser | NeoRender | Falta |
|---|---|---|---|
| Parse HTML → discover `<script type="module">` | ✅ | ✅ | — |
| Fetch module + static imports recursivamente | ✅ Paralelo | ⚠️ Solo depth 3 secuencial | **Parallelismo** |
| Build module graph | ✅ Completo antes de evaluar | ❌ **No construimos el graph** | **BLOCKER** |
| Instantiate modules (link imports/exports) | ✅ Depth-first | ❌ deno_core lo hace pero nosotros no controlamos el orden | |
| Evaluate in post-order DFS | ✅ Garantizado por spec | ⚠️ deno_core lo hace si le das el graph completo | Depende de graph |
| Error en un module → otros siguen | ✅ Aislado | ✅ Ya implementado | — |

**El módulo `performance_analytics.js` no es un bug de brotli solamente — es que no construimos el module graph antes de evaluar. En un browser real, TODOS los imports se resuelven ANTES de que se ejecute NADA.**

---

### FASE 6: EVENTS — LA INTERACCIÓN

```
User action → OS event → browser event → JS handler → DOM mutation → re-render
```

| Paso | Browser | NeoRender | Falta |
|---|---|---|---|
| Mouse click | Coordinates → hit test → target element | ❌ **No hay hit testing** | Solo resolvemos por selector |
| Click event chain | mousedown → mouseup → click | ✅ LiveDom fireClick | — |
| Focus management | Click → focus target → blur previous → update activeElement | ✅ LiveDom fireFocusChange | — |
| Keyboard input | keydown → keypress → beforeinput → textInput → input → keyup | ✅ LiveDom fireTypeText per-char | — |
| Default actions | Click link → navigate. Click submit → submit form. | ✅ LiveDom detecta y ejecuta | — |
| Event delegation (React) | Events bubble to root → React synthetic dispatch | ⚠️ **Events bubble via happy-dom pero React puede no escuchar** | **React event delegation** |
| Trusted vs untrusted | Browser events son trusted | ❌ Todos nuestros events son untrusted | Algunos handlers checkan `event.isTrusted` |

**El problema de React**: React 18+ usa event delegation. Escucha en `document.body` (o el root). Cuando nosotros dispatachamos un click en un botón, el evento burbujea. Pero React necesita que el evento llegue a SU root listener. Si happy-dom no burbujea correctamente, React no lo ve.

---

### FASE 7: RENDERING PIPELINE (POST-JS)

En un browser real, después de que JS modifica el DOM:

```
DOM change → Style recalc → Layout → Paint → Composite → Display
```

Nosotros no tenemos NADA de esto. Y normalmente no importa. PERO:

1. **React batch updates**: React batchea setState y hace un solo render al final del event handler. Sin render cycle, ¿cuándo ocurre el commit?
   - Respuesta: React usa microtasks (Promise.resolve) para flush. Funciona.

2. **CSS transitions/animations**: No aplica.

3. **Lazy loading via IntersectionObserver**: Sin render, IO nunca fire naturalmente.
   - Fix actual: fire sintético post-settle. Funcional.

---

### FASE 8: NAVIGATION (SPA)

```
Link click → preventDefault → pushState → router matches → fetch data → render components
```

| Paso | Browser | NeoRender | Falta |
|---|---|---|---|
| Click `<a>` | preventDefault + pushState | ✅ LiveDom detecta link click | — |
| pushState | URL change + popstate | ✅ | — |
| Router resolve | Framework-specific | ⚠️ Funciona si React/Vue router está montado | — |
| Data fetch | fetch() + Suspense | ⚠️ fetch funciona pero settle puede cortar antes de que data llegue | **Settle aware de fetches** |
| Component render | React reconciliation | ⚠️ Funciona si todo lo anterior funciona | — |

---

## RESUMEN: LOS 10 GAPS REALES (no síntomas)

| # | Gap | Por qué rompe | Dificultad |
|---|---|---|---|
| **1** | **Brotli decompression roto en wreq** | Módulos JS devuelven 0 bytes → app no monta | ALTA (bug de wreq) |
| **2** | **No construimos module graph antes de evaluar** | Módulos se evalúan sin tener sus dependencias resueltas | ALTA (cambio arquitectural) |
| **3** | **TLS fingerprint no pasa Cloudflare** | ChatGPT y otros sites con Cloudflare → 403 | ALTA (problema de librería) |
| **4** | **MutationObserver timing incorrecto** | Frameworks que dependen de MO en el momento correcto del loop | MEDIA |
| **5** | **Layout APIs devuelven 0** | `offsetWidth === 0` → frameworks creen que elemento está hidden | BAJA (mejores stubs) |
| **6** | **React event delegation puede fallar** | Click events pueden no llegar al root listener de React | MEDIA |
| **7** | **crypto.subtle es fake** | JWT verification, auth tokens — resultados incorrectos | MEDIA |
| **8** | **Event loop no simula render step** | rAF, IO, RO no fires en el momento correcto | MEDIA |
| **9** | **No hay streaming body** | SSE, streaming fetch — todo buffered | MEDIA |
| **10** | **happy-dom #window incompatibilities** | Constructors fallan fuera de Window context | BAJA (ya parcheado mayormente) |

---

## QUÉ HARÍA UN BROWSER REAL QUE NO HACEMOS

### El ciclo completo de un "type text in input and click submit":

**Browser real**:
1. Focus input → `focusin`, `focus` events fire → `document.activeElement` = input
2. Key press 'h' → `keydown` → `beforeinput` → modify `input.value` → `input` event → `keyup`
3. React: root listener catches `input` event → synthetic onChange → setState → re-render
4. Repeat for each character
5. Focus out (if user tabs or clicks elsewhere) → `change` event on input
6. Click "Submit" button → `mousedown` → `mouseup` → `click` → event bubbles to React root
7. React: root listener catches `click` → synthetic onClick → handler calls fetch/submit
8. fetch() starts → Promise pending → event loop continues
9. Response arrives → Promise resolves → microtasks → setState → re-render with data

**NeoRender (ideal, con LiveDom)**:
1-4. ✅ LiveDom `fireTypeText` hace exactamente esto, incluyendo _valueTracker
5. ✅ LiveDom fires `change` on blur
6-7. ⚠️ LiveDom `fireClick` dispatcha mousedown/mouseup/click que burbujean. **PERO**: ¿happy-dom burbujea correctamente hasta `document.body` donde React escucha? Si no, React no procesa el click.
8-9. ✅ fetch() funciona, promises resuelven, settle loop espera.

**El gap #6 (React event delegation) es probablemente por qué el botón Send de ChatGPT no dispara fetch cuando lo clickeamos.**

---

## PLAN DE ACCIÓN (PRIORIZADO POR IMPACTO)

### Inmediato: Verificar event bubbling
Antes de parchear nada más, verificar: ¿los events burbujean hasta el root de React en happy-dom?

```javascript
document.body.addEventListener('click', function(e) {
    console.log('BODY SAW CLICK on', e.target.tagName, e.target.getAttribute('data-testid'));
}, true); // capture phase

// Then click a button
```

Si el click NO llega a body → happy-dom tiene un bug de bubbling → ESE es el root cause.

### Si bubbling funciona: el problema es otro
Si los events SÍ burbujean, el problema es que React puede no estar escuchando en el root correcto, o el event no tiene las properties que React espera (`nativeEvent`, etc.).

### Prioridad de fixes:
1. **Event bubbling verification** (30 min)
2. **Brotli workaround** (1 sesión — retry without compression o usar curl-impersonate)
3. **Module graph pre-resolution** (2 sesiones — resolver TODOS los imports antes de evaluar)
4. **Layout API stubs con valores no-zero** (0.5 sesión)
5. **Render step simulation** (1 sesión — fire rAF/MO/IO/RO en el momento correcto)
