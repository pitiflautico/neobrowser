# PDR: SPA Hydration V2 — Lo que falta para que SPAs modernas funcionen

## Fecha: 24 March 2026

## Problema

NeoRender V2 carga páginas SSR bien (sesamehr, Wikipedia, HN). Pero SPAs modernas (Factorial, ChatGPT, apps Vite/React) fallan silenciosamente — el HTML llega, los scripts se ejecutan, pero **la app nunca monta**. El `#root` o `#factorialRoot` queda vacío.

### Sites probados y resultado:

| Site | Framework | Resultado | Causa raíz |
|---|---|---|---|
| sesamehr.com | SSR + jQuery | OK | No necesita hidratación |
| app.sesametime.com/login | Vue SPA | OK — form visible | Vue monta |
| chatgpt.com | React 18 + Next.js | Parcial (329/354 fibers) | API calls vacíos (auth), SSE missing |
| app.factorialhr.com | React + Vite + Lit WC | FALLA — root vacío | Module chain rota, vendor.js panic |
| Mercadona | React 18 SSR | OK | Hydration funciona |

---

## Análisis: Qué hace un browser real que nosotros NO hacemos

### 1. `<link rel="modulepreload">` — IGNORADO

**Browser real**: Cuando encuentra `<link rel="modulepreload" href="vendor.js">`, el browser:
1. Fetch el módulo en paralelo (no bloquea parsing)
2. Parse el módulo (compilar bytecode)
3. Resuelve sus dependencias estáticas recursivamente
4. Cuando un `<script type="module">` lo importa → ya está listo

**NeoRender**:
- Fetchea el contenido como texto plano en la fase de script_fetch
- Lo guarda en el ScriptStore
- Cuando `load_module()` lo pide, el NeoModuleLoader lo encuentra en el store
- PERO: **no resuelve dependencias transitivas del modulepreload**

**Consecuencia**: Cuando `app.js` hace `import { Router } from './vendor.js'`, vendor.js está en el store. Pero cuando vendor.js internamente hace `import { h } from './framework.js'`, ese módulo NO está pre-fetched → fetch on-demand → posible timeout o error de budget.

**Fix necesario**: En phase 3 (prefetch), cuando fetcheamos un modulepreload:
1. Parse el JS para encontrar `import` statements estáticos
2. Fetch recursivamente hasta depth 2-3
3. Guardar todo en el ScriptStore antes de que la ejecución empiece

### 2. Module evaluation order — INCORRECTO

**Browser real**: Los módulos ES se evalúan en **post-order DFS** del dependency graph. Si A importa B y C, y B importa D:
```
Evaluate: D → B → C → A
```
Esto garantiza que cuando A se ejecuta, B y C ya están evaluados.

**NeoRender**: Evalúa módulos en **document order** (como aparecen en el HTML):
```
script_exec.rs: for script in deferred_scripts { execute(script) }
```
Si `vendor.js` aparece antes que `vite.js` en el HTML, vendor se evalúa primero. Pero si vite.js depende de vendor.js, deno_core maneja eso. **El problema real es cuando un módulo ya fue evaluado como inline script y luego se intenta evaluar como ES module** → panic "Module already evaluated".

**Fix necesario**:
- Trackear qué URLs ya se ejecutaron (como inline scripts)
- Cuando `load_module(url)` se llama para esa URL → skip
- Ya implementado parcialmente con `loaded_modules` HashSet, pero el contenido inline se ejecuta via `execute()` (no `load_module()`), así que el URL no se registra

### 3. CSS loading blocks rendering — NO IMPLEMENTADO

**Browser real**: `<link rel="stylesheet">` bloquea rendering hasta que el CSS carga. Los scripts después del stylesheet esperan.

**NeoRender**: CSS se ignora completamente (no hay rendering engine). Esto generalmente está bien, EXCEPTO cuando:
- Un script espera que `getComputedStyle()` devuelva valores reales
- Un framework checks si un elemento es visible via CSS antes de montar

**Fix necesario**: Ninguno inmediato — `getComputedStyle` ya tiene un polyfill proxy.

### 4. Dynamic `import()` en runtime — PARCIAL

**Browser real**: `import('/chunk.js')` fetchea, compila, evalúa el módulo, y resuelve la Promise.

**NeoRender**: `import()` funciona via deno_core, PERO:
- Si el módulo no está en el ScriptStore, el NeoModuleLoader lo fetchea on-demand
- El fetch usa la misma fetch budget que los scripts de la página
- Si la budget se agotó (10s), `import()` falla con "fetch budget exceeded"
- Factorial tiene 99 modules → budget se agota en los primeros 26

**Fix necesario**:
- Separar budgets: script loading budget vs runtime fetch budget
- Module fetches (import()) should have their own unlimited budget (o mucho más alto)
- O: pre-fetch TODOS los modulepreload antes de ejecutar nada

### 5. Web Components lifecycle — PARCIAL

**Browser real**:
1. `customElements.define('my-el', class extends HTMLElement {...})`
2. Cuando `<my-el>` aparece en el DOM → `connectedCallback()` se llama
3. `attachShadow({mode:'open'})` → crea shadow DOM
4. `attributeChangedCallback()` → reacciona a cambios de atributos

**NeoRender**:
- `customElements.define()` registra la clase ✅ (polyfill)
- `document.createElement('my-el')` busca en registry y aplica prototype ✅ (patch)
- `connectedCallback()` se llama al crear ✅
- `attachShadow()` crea un DocumentFragment básico ✅
- **`observedAttributes` + `attributeChangedCallback`** → NO implementado
- **Upgrade de elementos existentes en el DOM** → NO implementado
- **Slotting** (shadow DOM slots) → NO implementado
- **CSS encapsulation** (shadow DOM styles) → NO implementado (no tenemos CSS engine)

**Fix necesario para factorial**:
- Cuando el parser HTML encuentra `<custom-tag>` y hay un constructor registrado → upgrade in place
- `attributeChangedCallback` debe dispararse cuando se setean atributos
- Esto requiere un MutationObserver en el registry que observe childList del document

### 6. Script inline + Module import de mismo archivo — CRASH

**Browser real**: Si un `<script>` inline ejecuta vendor.js y un `<script type="module">` también importa vendor.js, el browser lo maneja (el módulo ya está evaluado, reutiliza).

**NeoRender**:
- `<script>vendor.js</script>` → `rt.execute(code)` (registra en V8 como script, NO como módulo)
- `<script type="module" src="vendor.js">` → `rt.load_module(url)` → deno_core intenta registrar como módulo → panic "already evaluated"

**Causa raíz**: deno_core no distingue scripts de módulos internamente para el mismo código. Si el código fue evaluado como script, no puede re-registrarse como módulo.

**Fix necesario**:
- Opción A: Cuando un External script se va a ejecutar Y también hay un Module con la misma URL → ejecutar SOLO como módulo (skip el script inline)
- Opción B: Detectar duplicados en phase 2 (discovery) y deduplicar
- **Opción C (recomendada)**: En `execute_scripts()`, mantener un set de URLs ejecutadas. Antes de `load_module(url)`, check si ya se ejecutó como inline → skip el module load.

### 7. `<script type="module">` con imports estáticos — TRANSFORMACIÓN INCOMPLETA

**Browser real**:
```html
<script type="module">
import { createApp } from '/vendor.js'
createApp('#root')
</script>
```
El browser resuelve el import, evalúa vendor.js primero, luego ejecuta el inline module.

**NeoRender**: Transforma esto a:
```javascript
(async () => {
  const { createApp } = await import('/vendor.js')
  createApp('#root')
})()
```
Esto FUNCIONA en la mayoría de casos, PERO:
- El `await import()` es dinámico — pierde la semántica de module evaluation order
- Si vendor.js tiene side effects que dependen del timing de import estático, se rompe
- Top-level `this` es `undefined` en modules vs `window` en scripts — nuestro IIFE usa `window`

**Fix necesario**: Usar `rt.load_module()` en vez de transformar a IIFE cuando el inline module solo tiene imports estáticos.

### 8. fetch() durante hidratación — LIMITADO

**Browser real**: React hydration hace `fetch('/api/user')` → respuesta con datos → state update → re-render con datos.

**NeoRender**: `fetch()` funciona via `op_fetch`, PERO:
- La settle loop espera DOM stability, no pending fetches
- Un fetch que tarda 2s puede terminar DESPUÉS de que la settle loop declare "stable"
- El WOM se extrae sin los datos del fetch → página "vacía" o parcial

**Fix necesario**:
- La settle loop debe considerar `pending_fetches` además de DOM mutations
- Quiescence check: `idle_ms > X AND pending_timers == 0 AND pending_fetches == 0 AND dom_mutations == 0`
- Esto ya existe parcialmente en `Quiescence` struct pero no se usa para fetch count

### 9. SSE / Streaming responses — MISSING

**Browser real**: `new EventSource('/stream')` o `fetch().body.getReader()` para streaming.

**NeoRender**:
- `op_fetch` lee todo el body de golpe
- `op_fetch_start` + `op_fetch_read_chunk` implementados pero EventSource no existe
- ChatGPT conversation responses son SSE → bloqueado

**Fix necesario**:
- EventSource polyfill en bootstrap.js (usa streaming fetch internamente)
- ReadableStream polyfill funcional (happy-dom's está roto)
- Ya diseñado en PDR-100-PERCENT.md §1.1

### 10. Error recovery — NO EXISTE

**Browser real**: Si un script falla, el browser continúa con el siguiente. Si React crash, el error boundary lo atrapa.

**NeoRender**:
- Script errors se loguean pero ✅ no bloquean (ya arreglado)
- Module errors se loguean pero ✅ no bloquean (ya arreglado)
- **Panic en deno_core** → `catch_unwind` ✅ (ya arreglado)
- **V8 isolate corruption después de panic** → NO recuperable. Si deno_core paniquea, el isolate puede quedar en estado inconsistente. Módulos posteriores pueden fallar sin razón aparente.

**Fix necesario**: Cuando hay un panic en mod_evaluate, considerar recrear el runtime (como cross-origin nav). Es drástico pero garantiza estado limpio.

---

## Priorización

### P0 — Sin esto nada funciona (desbloquea Factorial, ChatGPT)

| # | Fix | Impacto | Esfuerzo |
|---|---|---|---|
| 1 | **Deduplicar script/module por URL** | Elimina panic vendor.js | 0.5 sesión |
| 2 | **Pre-fetch module dependencies recursivo** | Módulos tienen sus imports listos | 1 sesión |
| 3 | **Budget separado para module imports** | SPAs con 100+ modules cargan | 0.5 sesión |
| 4 | **Settle loop considera pending fetches** | Datos de API llegan antes de extraer | 0.5 sesión |
| 5 | **Web Components upgrade automático** | Elementos custom se hidratan | 1 sesión |

### P1 — Mejora significativa de compatibilidad

| # | Fix | Impacto | Esfuerzo |
|---|---|---|---|
| 6 | **EventSource polyfill** | SSE/streaming funciona | 1 sesión |
| 7 | **ReadableStream funcional** | Streaming fetch body | 1 sesión |
| 8 | **Inline modules como ES modules reales** | Mejor compat con import semantics | 1 sesión |
| 9 | **attributeChangedCallback** | Web Components reactivos | 0.5 sesión |
| 10 | **Runtime recreation after panic** | Recuperación limpia | 0.5 sesión |

### P2 — Polish

| # | Fix | Impacto | Esfuerzo |
|---|---|---|---|
| 11 | CSS media queries en getComputedStyle | Responsive frameworks | 0.5 sesión |
| 12 | Shadow DOM slotting | Advanced Web Components | 1 sesión |
| 13 | Module evaluation order (post-order DFS) | Spec compliance | 1 sesión |

---

## Plan de implementación

### Sesión 1: Dedup + Budget (P0 #1, #3)
- Deduplicar scripts con misma URL (inline vs module)
- Budget separado para module imports (ilimitado o 30s)
- Test: Factorial no paniquea Y carga más módulos

### Sesión 2: Module prefetch recursivo (P0 #2)
- En phase 3, para cada modulepreload:
  - Parse JS, extract `import` declarations
  - Fetch transitivas hasta depth 3
  - Guardar en ScriptStore
- Test: Factorial module chain se resuelve

### Sesión 3: Settle + Web Components (P0 #4, #5)
- Settle loop espera pending_fetches == 0
- Web Components: upgrade automático de custom elements en DOM
- Test: Factorial form renderiza, ChatGPT sidebar carga datos

### Sesión 4: Streaming (P1 #6, #7)
- EventSource polyfill
- ReadableStream fix
- Test: ChatGPT PONG recibe respuesta streaming

---

## Métricas de éxito

| Site | Hoy | Target |
|---|---|---|
| sesamehr.com | OK | OK |
| app.sesametime.com | OK (form visible) | OK |
| app.factorialhr.com/login | FALLA (root vacío) | Form visible + interactable |
| chatgpt.com | Parcial (sidebar vacío) | Sidebar con conversations |
| chatgpt.com PONG | 403 TLS | Envía mensaje + recibe respuesta |

---

## Referencias

- [Vite module preloading](https://vite.dev/guide/features)
- [React 18 Selective Hydration](https://github.com/reactwg/react-18/discussions/37)
- [ES Module Preloading](https://guybedford.com/es-module-preloading-integrity)
- [Web Components SSR](https://dev.to/stuffbreaker/web-components-and-ssr-2024-edition-1nel)
- PDR-100-PERCENT.md (gaps internos)
- PDR-SESSION-ISOLATION.md (cross-origin)
