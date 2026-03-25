# PDR: Browser Completo — Plan definitivo para que NeoRender V2 funcione

## Fecha: 24 March 2026
## Estado: Los 10 gaps reales identificados. Chrome fallback desactivado.

---

## DIAGNÓSTICO FINAL

Hemos parchado 29 cosas en esta sesión. Ninguna resolvió el problema de fondo. Los síntomas son:

1. **Factorial**: root vacío → `performance_analytics.js` devuelve 0 bytes (brotli)
2. **ChatGPT**: botón Send no dispara fetch → events no burbujean en happy-dom
3. **ChatGPT API**: 403 → TLS fingerprint de wreq no pasa Cloudflare

Pero estos son **síntomas de 3 problemas de infraestructura**:

### Problema A: HTTP response body se pierde (brotli)
wreq con feature `brotli` habilitada + servers Azure/Cloudflare que fuerzan `content-encoding: br` = body vacío. No es un problema de nuestro código — es wreq/boring2.

**Fix**: Decompresión manual con `brotli` crate. No esperar al maintainer.

```rust
// En op_fetch / module_loader, después de recibir la response:
if content_encoding == "br" && body.is_empty() {
    // wreq failed to decompress — do it manually
    let raw = resp.bytes().await?;
    let mut decompressed = Vec::new();
    let mut reader = brotli::Decompressor::new(&raw[..], 4096);
    std::io::Read::read_to_end(&mut reader, &mut decompressed)?;
    body = String::from_utf8_lossy(&decompressed).to_string();
}
```

**Tiempo**: 30 minutos. Un archivo (ops.rs o client.rs).

### Problema B: Module graph no se resuelve antes de evaluar
Un browser real resuelve TODOS los imports estáticos de un módulo ANTES de evaluar NADA. Nosotros evaluamos en document order, y si un módulo aún no se fetcheó, el import falla.

Ejemplo real:
```
vite.js  → import { Router } from './vendor.js'
vendor.js → import { W } from './performance_analytics.js'
```

Browser: fetch los 3 → resolve graph → evaluate performance_analytics → vendor → vite
NeoRender: evaluate vite → vendor not ready → fetch on-demand → race condition

**Fix**: En `load_module()`, antes de `mod_evaluate`:
1. `load_side_es_module()` ya resuelve el graph (deno_core lo hace internamente)
2. El problema es que el FETCH de las dependencias transitivas puede fallar (brotli, budget)
3. Con el fix de brotli (Problema A), el graph se resuelve correctamente
4. Asegurar que `run_event_loop` se ejecuta COMPLETAMENTE después de `load_side_es_module` para que las dependencias se fetcheen

**Tiempo**: 1 hora. Depende de fix A.

### Problema C: Event bubbling roto en happy-dom
Test empírico: `dispatchEvent(new MouseEvent('click', {bubbles: true}))` → listeners en `body` y `document` NO reciben el event. Results array vacío.

Esto mata React (event delegation en root), Vue (event delegation), y cualquier framework moderno.

**Causa probable**: happy-dom dispara el event SOLO en el target, no bubble up. O nuestro stub de MutationObserver (que patcheamos) interfiere con el event dispatch chain.

**Fix posible 1**: Implementar bubbling manual en nuestro event dispatch:
```javascript
function dispatchWithBubble(target, event) {
    // Walk up the DOM tree dispatching at each level
    var node = target;
    while (node) {
        node.dispatchEvent(event); // ← esto no burbujea en happy-dom
        // Necesitamos llamar los listeners de CADA nodo manualmente
        node = node.parentElement;
    }
}
```

**Fix posible 2**: Verificar que happy-dom SÍ burbujea y que el problema es otro (quizás los listeners se registraron en un document diferente al que usamos).

**Fix posible 3**: Interceptar `addEventListener` en body/document y registrar los handlers en un registry propio. Cuando dispatachemos un event, walkear el DOM tree y llamar handlers.

**Tiempo**: 2-4 horas. Requiere investigación de happy-dom internals.

---

## PLAN DE EJECUCIÓN

### Fase 1: Brotli fix (30 min)
- Añadir `brotli` crate a neo-http
- En `RquestClient::send()`: si response body vacío + content-encoding: br → decompress manual
- Test: `performance_analytics.js` devuelve 74KB

### Fase 2: Event bubbling fix (2-4 horas)
- Test: verificar si happy-dom burbujea events nativamente
- Si no burbujea: implementar manual bubble walk
- Si sí burbujea: encontrar por qué nuestros listeners no reciben
- Test: click en botón → listener en body lo recibe

### Fase 3: Module graph (depende de Fase 1) (1 hora)
- Con brotli arreglado, los módulos se fetchean correctamente
- Verificar que deno_core `load_side_es_module` resuelve el graph completo
- Si no: forzar fetch de todas las dependencias estáticas antes de evaluar
- Test: factorial carga sin "does not provide export 'W'"

### Fase 4: Verificación end-to-end
- Sesame login: type email → click Next → ¿formulario responde?
- ChatGPT: type en textarea → click Send → ¿fetch se dispara?
- Factorial: ¿form renderiza?

---

## SOBRE EL CONSEJO DE GROK

### Lo que Grok dice bien:
1. **Brotli manual**: Correcto. No esperar al maintainer, decompress manual.
2. **Module graph completo antes de evaluar**: Correcto. Es lo que Chrome hace.
3. **ProseMirror como bypass**: Es un hack válido para ChatGPT específicamente, pero no resuelve el problema de fondo (events no burbujean → ningún botón de ningún site funciona via click).

### Lo que Grok dice mal:
1. **"15 min / 20 min"**: Irrealista. El fix de brotli requiere cambiar el pipeline de fetch en ops.rs + client.rs + module_loader, no es solo añadir 5 líneas.
2. **"Usa la vía formulario (ProseMirror)"**: Es un hack para UN site. No es una solución de navegador.
3. **`runtime.has_pending_module_evaluation()`**: Esta API no existe en deno_core 0.311.
4. **"setTimeout(() => {}, 0) fuerza render step"**: No. setTimeout es macrotask, no simula el render step del event loop. rAF fires en el render step, no en macrotask.

### Mi posición:
Los fixes de Grok resuelven ChatGPT hoy. Pero no hacen un navegador. Si arreglamos el bubbling (Problema C), TODOS los sites funcionan, no solo ChatGPT. Es la inversión correcta.

---

## MÉTRICAS DE ÉXITO

| Test | Hoy | Target |
|---|---|---|
| `curl` vs `wreq` para performance_analytics.js | curl=74KB, wreq=0 | Ambos 74KB |
| `dispatchEvent({bubbles:true})` → body listener fires | NO | SÍ |
| Sesame: type email → click Next | Click no hace nada en React | React procesa el click |
| Factorial: root tiene contenido | Vacío | Form visible |
| ChatGPT: click Send → fetch fires | 0 fetches | 1+ fetches |

---

## ARCHIVOS A MODIFICAR

### Fase 1 (brotli):
- `crates/neo-http/Cargo.toml` — añadir `brotli = "7"`
- `crates/neo-http/src/client.rs` — decompress manual si body vacío + br
- `crates/neo-runtime/src/ops.rs` — mismo fix en op_fetch
- `crates/neo-runtime/src/modules.rs` — mismo fix en module fetcher

### Fase 2 (event bubbling):
- `js/bootstrap.js` — investigar y patchear event dispatch
- O `js/happy-dom.bundle.js` — patchear dispatchEvent si happy-dom no burbujea

### Fase 3 (module graph):
- `crates/neo-runtime/src/v8_runtime_impl.rs` — asegurar graph resolution completo
- `crates/neo-engine/src/session/script_exec.rs` — prefetch de dependencias transitivas

---

## ORDEN DE IMPLEMENTACIÓN

```
1. Event bubbling test + fix     ← PRIMERO (desbloquea todo)
2. Brotli manual decompress      ← SEGUNDO (desbloquea factorial)
3. Module graph verify           ← TERCERO (depende de 2)
4. End-to-end verification       ← CUARTO
```

El event bubbling es primero porque sin él, ni siquiera los sites que SÍ cargan (sesame, ChatGPT) funcionan correctamente con clicks. Es el gap más fundamental que tenemos.
