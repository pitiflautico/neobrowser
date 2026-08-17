# Bugs: automatización de formularios multipaso

> **Estado: los cuatro arreglados el 2026-08-13.** Cobertura en `rust/tests/multistep_forms.rs`
> (7 tests, hermético con fixture `data:` URL) más `same_page_ignores_query_and_fragment`
> en `sessions.rs`. Suite completa: 94 tests en verde, clippy sin warnings.
> Los tests comprueban **efectos** (¿quedó marcado el checkbox?, ¿qué botón recibió el
> click?), nunca el valor de retorno — que es justo lo que fallaba.

> **Las rutas de este documento son de la versión `0.1.3`.** Las referencias
> `archivo.rs:línea` describen el código del 2026-08-13, antes de que el crate se
> reorganizase en módulos por responsabilidad. Los módulos citados siguen existiendo, pero la
> lógica concreta se ha repartido: el click y su verificación (con el enum `ClickOutcome`)
> están en [`rust/src/page/pointer.rs`](../rust/src/page/pointer.rs), el filtro de
> visibilidad en [`rust/js/find_and_click.js`](../rust/js/find_and_click.js), el lock de
> perfil en [`rust/src/chrome/lock.rs`](../rust/src/chrome/lock.rs), el reap del proceso en
> [`rust/src/chrome/process.rs`](../rust/src/chrome/process.rs) y el heurístico de login en
> [`rust/src/sessions/login.rs`](../rust/src/sessions/login.rs). Los números de línea ya no
> valen: lo que conserva el valor es el diagnóstico, no la ubicación.

> **La regla que sale de estos cuatro bugs está ahora escrita como especificación:**
> [The Verified Action Contract](VERIFIED-ACTIONS.md). El bug nº2 es literalmente su
> escenario C2 — un click sobre un target tapado por un overlay debe devolver `blocked`
> nombrando la obstrucción, y le está prohibido devolver `succeeded`.

Detectados el 2026-08-13 automatizando el alta de un proyecto en `thefwa.com` — un formulario React de 4 pasos con acordeón, overlays y elementos fuera del viewport. Versión afectada: `0.1.3`.

Los cuatro comparten una misma raíz:

> **Se reporta éxito por haber despachado la acción, no por haber comprobado que surtió efecto.**

Un agente que se fía del valor de retorno da por hecho un formulario que en realidad está a medias. En la sesión que los descubrió esto produjo un diagnóstico completamente falso: concluí que el código de descuento del usuario no era válido cuando el problema era que el paso 2 del formulario nunca llegaba a enviarse. Coste real: ~40 min de depuración a ciegas y una afirmación errónea al usuario.

Orden de arreglo sugerido: **1 → 2 → 3 → 4**. El 1 y el 2 son la misma clase de fallo y conviene arreglarlos juntos.

---

## 1. `find_and_click` clica nodos invisibles — CRÍTICO ✅ ARREGLADO

> Filtra por visibilidad, cuenta descartados (`matched_total` / `matched_visible`) y delega en el click de ratón real.
> Tests: `find_and_click_skips_collapsed_matches`, `find_and_click_reports_when_all_matches_are_hidden`.

**Ubicación:** `rust/src/ops.rs:249-268`

### Síntoma

Clica el primer nodo del DOM cuyo texto coincide, aunque esté dentro de un contenedor colapsado (`height: 0`), oculto o fuera de pantalla. Devuelve `{"ok": true}`.

### Reproducción

Un acordeón de varios pasos donde cada paso tiene su propio botón "Continue". Los pasos cerrados siguen en el DOM con altura 0:

```html
<form id="step1" style="height:0; overflow:hidden">  <!-- colapsado -->
  <button>Continue</button>                           <!-- ← se clica ESTE -->
</form>
<form id="step2">                                     <!-- abierto, visible -->
  <button>Continue</button>                           <!-- ← el que quería -->
</form>
```

`find_and_click({text: "Continue"})` reenvía el paso 1 indefinidamente. El paso 2 no se envía nunca y el servidor devuelve su sección vacía. Nada en la respuesta de la tool lo delata.

### Causa raíz

El filtro es puramente textual — no hay ninguna comprobación de visibilidad:

```js
var matches = els.filter(function(e) {
    return e.textContent.toLowerCase().indexOf(textQ) !== -1 ||
           (e.getAttribute('aria-label')||'').toLowerCase().indexOf(textQ) !== -1;
});
var target = matches[Math.min(nth, matches.length-1)];
target.click();
```

Dos problemas más en esas mismas líneas:

- **`textContent` incluye texto de descendientes ocultos.** Un contenedor cuyo hijo oculto dice "Continue" matchea.
- **`target.click()` es un click de JS, no un evento de ratón.** Contradice directamente lo que promete la descripción del servidor MCP (`mcp.rs:36`): *"Clicks are real (isTrusted) mouse events"*. Un sitio que valide `event.isTrusted` distingue este click del de `click`.

### Fix

Filtrar por visibilidad real y delegar en el click de ratón que ya existe, en vez de duplicar un `.click()` de JS:

```js
var visible = function(e) {
    var r = e.getBoundingClientRect();
    if (r.width === 0 || r.height === 0) return false;
    var s = getComputedStyle(e);
    if (s.visibility === 'hidden' || s.display === 'none' || s.opacity === '0') return false;
    // descartar nodos dentro de un ancestro colapsado (acordeones)
    for (var p = e.parentElement; p; p = p.parentElement) {
        var pr = p.getBoundingClientRect(), ps = getComputedStyle(p);
        if (pr.height === 0 && ps.overflow === 'hidden') return false;
    }
    return true;
};
var all = matches.length;
matches = matches.filter(visible);
if (matches.length === 0) {
    return JSON.stringify({ok: false,
        error: "matched " + all + " node(s) for " + textRaw + ", all hidden or collapsed"});
}
```

Y en lugar de `target.click()`, devolver el `backendNodeId` del target para que `ops::find_and_click` encadene con `page::click_backend_node` — así se hereda el click real, el scroll y la verificación del fix nº2.

Incluir siempre `matched_total` y `matched_visible` en la respuesta: que el agente pueda ver que había 2 candidatos y uno se descartó es la mitad del valor.

---

## 2. `click` no hace scroll ni verifica el impacto — CRÍTICO ✅ ARREGLADO

> `scrollIntoViewIfNeeded` + relectura de la caja + hit-test con `DOM.getNodeForLocation`. Nuevo enum `ClickOutcome`.
> Tests: `click_scrolls_target_into_view`, `click_detects_an_overlay_instead_of_claiming_success`, `click_distinguishes_a_missing_target`.

**Ubicación:** `rust/src/page.rs:175-204` (lógica) y `rust/src/tool_impls.rs:252-256` (mensaje)

### Síntoma

Devuelve `"Clicked"` cuando el click no ha tenido ningún efecto. Dos escenarios, ambos reproducidos en la misma sesión:

1. **Elemento fuera del viewport.** Tres checkboxes en `y ≈ 1240` con viewport de 993 px: los tres devolvieron `"Clicked"` y ninguno quedó marcado.
2. **Elemento tapado por un overlay.** Un banner de cookies `position: fixed` cubría el punto de click. `"Clicked"` — y el click se lo llevó el banner.

### Causa raíz

`click_backend_node` calcula el centro con `DOM.getBoxModel` y dispara `Input.dispatchMouseEvent` en esas coordenadas:

```rust
if let Some((cx, cy)) = box_center(client, backend_node_id).await? {
    human_mouse_move(client, cx, cy).await?;
    // ... mousePressed / mouseReleased en (cx, cy)
    return Ok(true);   // ← éxito incondicional
}
```

`DOM.getBoxModel` devuelve coordenadas **relativas al viewport actual**. Si el elemento está por debajo del fold, `cy` cae fuera de pantalla y el evento se pierde. Y aunque caiga dentro, nada garantiza que el elemento del target sea el que hay en ese punto. `Ok(true)` significa solo "he despachado dos eventos de ratón".

`human_mouse_move` está muy cuidado para el realismo anti-bot, pero mueve el cursor a un punto que puede no contener el elemento.

### Fix

Tres pasos antes del `Ok(true)`:

```rust
// 1. Llevar el elemento al viewport (CDP lo hace nativamente).
client.send("DOM.scrollIntoViewIfNeeded",
            json!({ "backendNodeId": backend_node_id })).await.ok();

// 2. Releer la caja DESPUÉS del scroll — las coordenadas de antes ya no valen.
let Some((cx, cy)) = box_center(client, backend_node_id).await? else {
    return js_click_backend_node(client, backend_node_id).await;
};

// 3. Comprobar que el punto pertenece al target antes de pulsar.
let hit = client.send("DOM.getNodeForLocation",
                      json!({ "x": cx as i64, "y": cy as i64 })).await;
let hit_id = hit.ok()
    .and_then(|r| r.get("backendNodeId").and_then(|v| v.as_i64()));
if let Some(hit_id) = hit_id {
    if hit_id != backend_node_id && !is_descendant(client, backend_node_id, hit_id).await? {
        return Err(CdpError::Obscured { expected: backend_node_id, got: hit_id });
    }
}
```

La comprobación tiene que aceptar descendientes: es normal que el punto caiga sobre un `<span>` interno del `<button>`. Lo que hay que rechazar es un nodo de otra rama.

En `tool_impls.rs`, distinguir los fallos en vez de colapsarlos en un booleano:

```rust
Ok(ToolOutput::text(match result {
    ClickResult::Ok            => "Clicked",
    ClickResult::NotFound      => "Click target not found",
    ClickResult::NoLayout      => "Target has no layout (display:none?)",
    ClickResult::Obscured{by}  => &format!("Target obscured by another element ({by}) — dismiss the overlay first"),
}))
```

Ese último mensaje es el que habría ahorrado el rato de depuración: el agente sabe qué hacer con "obscured", no sabe qué hacer con "Clicked".

---

## 3. `SingletonLock` huérfano mata el arranque con error mudo — ALTO ✅ ARREGLADO

> `clear_stale_lock` (solo si el PID está muerto) + stderr a `~/.neobrowser/logs/chrome-{port}.log`, cuyo tail viaja en el error.
> Tests: `stale_singleton_lock_does_not_block_launch`, `live_singleton_lock_is_left_alone`.

**Ubicación:** `rust/src/chrome.rs:286-328` (launch), `311-312` (stdio), `218` (error)

### Síntoma

```
Error: chrome did not become ready on port 49244 within timeout
```

Se repite en cada intento. El mensaje no menciona la causa real y no hay ningún log donde mirarla. En la sesión costó dos arranques fallidos y un diagnóstico manual con `ps` y `ls` sobre el directorio de perfil.

### Reproducción

Dejar un Chrome de NeoBrowser colgado (o matarlo con `SIGKILL`, o que el proceso padre muera sin reapearlo). Queda:

```
~/.neobrowser/profiles/default/SingletonLock -> mac.lan-25902
```

Todo lanzamiento posterior con ese mismo `--user-data-dir` sale inmediatamente: Chrome ve el lock, intenta delegar en la instancia existente y termina. El puerto de debug no llega a abrirse nunca.

### Causa raíz

Dos decisiones que se combinan mal:

```rust
cmd.stdout(std::process::Stdio::null())
   .stderr(std::process::Stdio::null())   // ← chrome.rs:311-312
```

Chrome sí escribe el motivo en stderr, pero va a `/dev/null`. Y `launch()` no comprueba en ningún momento si el lock del perfil está huérfano antes de usarlo.

### Fix

**a) Limpiar el lock huérfano antes de lanzar.** El `SingletonLock` es un symlink cuyo target es `hostname-pid`:

```rust
/// Elimina los Singleton* si el PID que los creó ya no existe.
fn clear_stale_lock(profile_dir: &Path) {
    let lock = profile_dir.join("SingletonLock");
    let Ok(target) = std::fs::read_link(&lock) else { return };
    let Some(pid) = target.to_string_lossy().rsplit('-').next()
                          .and_then(|p| p.parse::<i32>().ok()) else { return };
    // señal 0: comprueba existencia sin enviar nada
    let alive = unsafe { libc::kill(pid, 0) } == 0;
    if !alive {
        for f in ["SingletonLock", "SingletonCookie", "SingletonSocket"] {
            let _ = std::fs::remove_file(profile_dir.join(f));
        }
    }
}
```

Llamarlo justo después de `create_dir_all(&profile_dir)`. Ojo: si el PID **sí** vive, no borrar nada — hay una instancia legítima y lo correcto es reutilizarla o fallar con un mensaje claro.

**b) Capturar stderr y adjuntarlo al error.** Redirigir a un pipe o a `~/.neobrowser/logs/chrome-{port}.log` en vez de `null`, y que `ChromeError::NotReady` incluya las últimas líneas:

```rust
#[error("chrome did not become ready on port {port} within timeout.\nchrome stderr:\n{stderr}")]
NotReady { port: u16, stderr: String },
```

Solo con (b) el bug se habría diagnosticado en un minuto en lugar de en diez.

---

## 4. `login` da falsos negativos y envía el form equivocado — MEDIO ✅ ARREGLADO

> Cruza tres señales (password **visible** + URL sin cambiar), expone `confidence`, y ancla el submit al `form` del campo de contraseña.
> Test: `same_page_ignores_query_and_fragment`.

**Ubicación:** `rust/src/sessions.rs:202-232`

### Síntoma A — falso negativo

Login correcto reportado como fallo:

```json
{"ok": false, "still_has_password_field": true,
 "title": "The FWA - Account", "url": "https://thefwa.com/account/settings"}
```

La sesión estaba perfectamente iniciada. La página de destino (`/account/settings`) tiene campos `oldPassword` / `newPassword` para cambiar la contraseña, y el heurístico los cuenta como "seguimos en el login".

### Causa raíz A

```rust
let still_login = /* !!document.querySelector('input[type=password]') */;
Ok(json!({ "ok": !still_login, ... }))
```

"Hay un campo password ⇒ ha fallado" no distingue un formulario de login de uno de cambio de credenciales, ni de un campo oculto en un panel lateral.

### Fix A

Combinar tres señales en lugar de una:

```rust
let failed = still_has_password
    && url_unchanged                    // seguimos en la misma URL de login
    && !has_session_cookie;             // no ha aparecido cookie de sesión
```

Y comprobar que el campo password que queda es **visible** (`getBoundingClientRect().height > 0`), no simplemente que exista en el DOM. Cuando las señales se contradigan, decirlo:

```json
{"ok": true, "confidence": "medium",
 "note": "URL changed and session cookie set, but a password field is still present"}
```

### Síntoma B — envía el formulario equivocado

Mismo bug que el nº1, en otro sitio:

```js
var btn = document.querySelector('button[type=submit],input[type=submit]');
if (btn) btn.click();
```

`querySelector` sobre todo el documento. En una página con un panel de login en la cabecera **y** un formulario de login en el cuerpo, esto pulsa el de la cabecera. Reproducido en `thefwa.com`, donde además había dos checkboxes de términos con IDs distintos (`tos11` en la cabecera, `tos1` en la página).

### Fix B

Anclar el submit al formulario que contiene el campo de contraseña que se acaba de rellenar:

```js
var pw = document.querySelector('input[type=password]');
var form = pw && pw.form;
var btn = form ? form.querySelector('button[type=submit],input[type=submit]')
              : document.querySelector('button[type=submit]');
if (btn) btn.click(); else if (form) form.submit();
```

---

## Regla transversal

Las cuatro correcciones son la misma idea aplicada en cuatro sitios:

| En lugar de | Hacer |
|---|---|
| "he despachado el evento" ⇒ `ok` | comprobar el efecto y reportar el resultado |
| colapsar los fallos en `bool` | tipar el fallo y decir cuál fue |
| una sola señal para decidir éxito | cruzar señales y exponer la confianza |
| `querySelector` sobre el documento | anclar la búsqueda al contenedor relevante |
| tragarse stderr | propagarlo al mensaje de error |

Para un servidor MCP esto no es cosmético: **el valor de retorno es la única percepción que el agente tiene del mundo.** Un `"Clicked"` falso no es un log ruidoso, es una alucinación inducida por la herramienta — y el agente construirá encima de ella hasta que algo reviente varios pasos más adelante, lejos de la causa.

## Tests de regresión sugeridos

Una página estática con los cuatro casos cubre todo lo anterior sin depender de un sitio externo:

1. Dos botones con el mismo texto, uno dentro de un `<div style="height:0;overflow:hidden">` → `find_and_click` debe alcanzar el visible.
2. Un checkbox a 2000 px de scroll → `click` debe marcarlo.
3. Un checkbox tapado por un `position: fixed` → `click` debe devolver *obscured*, no *Clicked*.
4. Un perfil con `SingletonLock -> host-999999` (PID inexistente) → `launch` debe limpiarlo y arrancar.
