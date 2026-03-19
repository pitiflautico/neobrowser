# PDR v3: AI Browser Engine — Sprint Completo

## Objetivo

Construir las 11 piezas que faltan para que NeoRender sea un browser REAL para IAs.
No parches per-site. Abstracciones de browser que hacen que TODO funcione.

## Las 11 piezas

### 1. Interacción semántica
`click("Enviar")` no `click(x=450, y=320)`.

- `click(target)` — busca por texto, selector, aria-label, placeholder, role
- `type(target, text)` — busca input, escribe, dispara input/change events
- `submit(target?)` — recoge FormData, encodea, HTTP request, navega a respuesta
- `select(target, value)` — selecciona opción en <select>
- `check(target, bool)` — checkbox/radio
- Si click en `<a>` → auto-navega. Si submit → auto-POST → auto-navega.
- El event dispatch es completo: pointerdown→mousedown→pointerup→mouseup→click con bubbling.

### 2. DOM como JSON nativo
El árbol DOM completo como JSON, no flat WOM.

```json
{
  "tag": "div", "id": "app", "class": "container",
  "children": [
    {"tag": "h1", "text": "Título"},
    {"tag": "form", "action": "/search", "children": [
      {"tag": "input", "name": "q", "type": "text", "placeholder": "Buscar"},
      {"tag": "button", "type": "submit", "text": "Enviar"}
    ]}
  ]
}
```

API: `session.dom_tree(depth?)` — devuelve el árbol podado (sin script/style/svg).

### 3. Diff de página
"Qué cambió desde la última vez que miré."

- MutationObserver REAL de linkedom (no stub)
- Acumula cambios: nodos added/removed, attributes changed, text changed
- `session.diff()` → devuelve solo los cambios desde el último observe
- Ahorra tokens: no re-enviar toda la página cada vez

### 4. Wait-for-condition
"Avísame cuando aparezca un elemento con este texto" — no polling.

- `session.wait_for(selector, timeout_ms)` → espera que el selector matchee
- Internamente: MutationObserver + timer
- `session.wait_for_text(text, timeout_ms)` → espera que el texto aparezca
- `session.wait_for_network_idle(timeout_ms)` → espera que no haya fetches pendientes
- `session.wait_for_stable(timeout_ms)` → espera que el DOM deje de cambiar

### 5. Interceptor de requests
Cada request/response como evento estructurado.

- Cada fetch() que el JS de la página hace → log estructurado
- `session.network_log()` → devuelve [{method, url, status, size, duration}]
- Filtrable por dominio, método, status
- No hay que habilitarlo — siempre activo
- Las API calls descubiertas se exponen automáticamente

### 6. Detección de estabilidad
"La página dejó de cambiar" — saber cuándo está lista.

- Cuenta nodos DOM cada 100ms
- Cuando el count no cambia en 500ms → "estable"
- También: no hay fetch pendientes + no hay timers activos
- `goto()` espera estabilidad automáticamente (con timeout)
- Elimina el sleep arbitrario post-render

### 7. Extracción automática
Tablas → JSON, artículos → texto limpio, formularios → schema.

- `session.extract_tables()` → [{headers:[], rows:[[]]}]
- `session.extract_article()` → {title, author, date, body, images}
- `session.extract_form_schema()` → {action, method, fields:[{name,type,required,options}]}
- `session.extract_links(filter?)` → [{text, href, context}]
- `session.extract_structured()` → JSON-LD, microdata, Open Graph

### 8. Multi-sesión nativa
10 identidades simultáneas sin perfiles separados.

- `NeoSessionPool` — pool de sesiones con identidades diferentes
- Cada sesión: su propio cookie jar, localStorage, device fingerprint
- `pool.get("amazon")` → sesión con cookies de Amazon
- `pool.get("linkedin")` → sesión con cookies de LinkedIn
- No se contaminan entre ellas

### 9. Mutación como stream
El DOM cambia → evento con exactamente qué cambió.

- MutationObserver real (linkedom lo soporta)
- Acumula mutations en un buffer
- `session.mutations()` → consume el buffer, devuelve [{type, target, added, removed, attr}]
- Útil para detectar: toast notifications, loading states, dynamic content

### 10. Rate limiting integrado
No machacar servidores por accidente.

- Per-domain rate limit configurable
- Default: 1 req/s por dominio
- Burst: hasta 5 req/s
- `session.set_rate_limit("amazon.es", 2, 10)` — 2 req/s, burst 10
- Queue automática: requests se encolan si exceden el rate
- Logging: "Rate limited: amazon.es (5/5)"

### 11. Anti-detección de base
Sin fingerprint de bot, sin navigator.webdriver.

- ✅ Ya tenemos: Chrome TLS via rquest
- ✅ Ya tenemos: no navigator.webdriver (linkedom no lo setea)
- Añadir: canvas fingerprint consistente (no random, deterministic per session)
- Añadir: WebGL renderer string realista
- Añadir: consistent screen/window dimensions
- Añadir: realistic navigator.plugins

## Implementación

Cada pieza es un módulo independiente:

```
src/neorender/
├── mod.rs
├── session.rs          # NeoSession (ya existe)
├── v8_runtime.rs       # V8 + linkedom (ya existe)
├── ops.rs              # V8 ↔ Rust ops (ya existe)
├── net/                # Networking (ya existe)
├── storage.rs          # SQLite localStorage (ya existe)
├── interact.rs         # [1] Interacción semántica
├── dom_tree.rs         # [2] DOM como JSON
├── diff.rs             # [3] Diff de página
├── wait.rs             # [4] Wait-for-condition
├── network_log.rs      # [5] Interceptor de requests
├── stability.rs        # [6] Detección de estabilidad
├── extract.rs          # [7] Extracción automática
├── pool.rs             # [8] Multi-sesión
├── mutations.rs        # [9] Mutación como stream
├── rate_limit.rs       # [10] Rate limiting
└── stealth.rs          # [11] Anti-detección

js/
├── linkedom.js         # DOM engine (ya existe)
├── bootstrap.js        # Browser globals (ya existe)
├── wom.js              # WOM extraction (ya existe)
├── browser.js          # [1] Event bridge + interaction
├── dom_tree.js         # [2] DOM → JSON tree
├── observer.js         # [3][9] MutationObserver → diff/mutations
├── wait.js             # [4] Wait-for-condition
├── intercept.js        # [5] Request interceptor
├── extract.js          # [7] Auto-extraction (tables, articles, forms)
└── stealth.js          # [11] Anti-detection patches
```
