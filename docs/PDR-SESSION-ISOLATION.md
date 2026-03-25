# PDR: Session Isolation — Un V8 por origen

## Fecha: 24 March 2026

## Problema

NeoRender V2 usa **UN solo V8 isolate** para toda la vida del MCP server. Cuando navegas de `sesametime.com` a `factorialhr.com`:

1. **Módulos colisionan**: los ES modules de sesame siguen en V8 → factorial intenta registrar "main" → error `Trying to create "main" module when one already exists`
2. **Globals contaminados**: `window`, `document`, `navigator` del site anterior persisten
3. **Import maps se mezclan**: el import map de sesame aplica a factorial
4. **Timer/fetch budgets no se resetean**: limits del site A afectan al site B
5. **Script store acumula**: scripts pre-fetched de todos los dominios en memoria

Un browser real aísla por **browsing context** (tab/origin). Nosotros no aislamos nada.

---

## Estado actual

| Componente | Compartido entre dominios | Debería aislarse |
|---|---|---|
| V8 Isolate | SÍ — uno global | SÍ — por origen |
| Módulos ES | SÍ — cache global | SÍ — por origen |
| Import map | SÍ — uno global | SÍ — por página |
| Script store | SÍ — acumula | SÍ — por origen |
| Globals (window/document) | SÍ — mutados | SÍ — fresh por página |
| Timer budget | SÍ — persiste | SÍ — reset por navegación |
| Fetch budget | SÍ — persiste | SÍ — reset por navegación |
| DOM | NO — replaced | OK |
| Cookies | Shared (domain-aware) | OK — domain matching correcto |
| HTTP cache | Shared (disk) | OK — standard HTTP semantics |

---

## Diseño propuesto

### Opción A: Nuevo V8 por navegación cross-origin (RECOMENDADA)

Cuando `navigate(url)` detecta un **cambio de origen** (`origin_a != origin_b`):

1. **Destruir** el runtime actual (`runtime.take()` + drop)
2. **Crear** un nuevo `DenoRuntime` con state limpio
3. **Preservar** solo lo que debe persistir: cookies, HTTP cache, history
4. **Bootstrap** fresco: re-inyectar bootstrap.js, happy-dom, etc.

**Para navegaciones same-origin** (ej: `/login` → `/dashboard`):
- Reusar el runtime existente (SPA-friendly)
- Limpiar sólo los timers/pendientes

**Coste**: crear un DenoRuntime tarda ~50-100ms (V8 snapshot). Aceptable.

### Opción B: V8 Context per origin (dentro del mismo isolate)

V8 soporta múltiples `Context` dentro de un `Isolate`. Cada contexto tiene sus propios globals pero comparte el heap.

- Pro: más rápido que crear nuevo isolate
- Contra: deno_core no expone contextos múltiples fácilmente, requiere cambios profundos

### Opción C: Pool de runtimes por origen

Mantener un `HashMap<String, DenoRuntime>` — uno por origen visitado.

- Pro: volver atrás a un site es instantáneo (state preserved)
- Contra: memoria crece con cada dominio visitado, complejidad

---

## Decisión: Opción A

Razones:
1. **Simplicidad**: un cambio en `browser_impl.rs::navigate()` — check origin, drop+recreate
2. **Correcta**: es lo que Chrome hace al navegar cross-origin (nuevo renderer process)
3. **Segura**: sin leaks de estado entre dominios
4. **Aceptable en coste**: 50-100ms por cross-origin nav, invisible comparado con HTTP latency

---

## Implementación

### Cambios en `neo-engine`

**Archivo**: `crates/neo-engine/src/session/browser_impl.rs`

```rust
fn navigate(&mut self, url: &str) -> Result<PageResult, EngineError> {
    let new_origin = extract_origin(url);
    let old_origin = self.current_origin.clone();

    // Cross-origin: destroy and recreate V8 runtime
    if old_origin != new_origin && !old_origin.is_empty() {
        eprintln!("[session] cross-origin: {} → {} — recreating V8", old_origin, new_origin);
        self.runtime = None; // drop old runtime
        self.runtime = Some(create_fresh_runtime(
            self.http_client.clone(),
            self.cookie_store.clone(),
            self.raw_client.clone(),
        ));
    }

    self.current_origin = new_origin;

    // ... rest of navigate as before
}
```

**Archivo**: `crates/neo-engine/src/session/mod.rs`

```rust
pub struct NeoSession {
    // ... existing fields ...

    /// Current page origin (e.g., "https://app.sesametime.com")
    current_origin: String,

    /// Factory for creating fresh V8 runtimes on cross-origin nav
    runtime_factory: Option<RuntimeFactory>,
}

/// Captures the config needed to create a new DenoRuntime
struct RuntimeFactory {
    http_client: Arc<dyn HttpClient>,
    cookie_store: Option<Arc<dyn CookieStore>>,
    raw_client: Option<Arc<wreq::Client>>,
    config: RuntimeConfig,
}
```

### Cambios en `neo-runtime`

**Archivo**: `crates/neo-runtime/src/v8.rs`

Añadir método para resetear state sin destruir el isolate (para same-origin):

```rust
impl DenoRuntime {
    /// Reset per-page state for same-origin navigation
    pub fn reset_page_state(&mut self) {
        // Clear script store
        self.store.borrow_mut().clear();
        // Clear import map
        self.import_map.borrow_mut().clear();
        // Reset budgets
        // Reset task tracker
        // Reset module tracker
    }
}
```

### Cambios en `neo-mcp`

Ninguno — MCP sigue con un solo `McpState.engine`. La aislación ocurre internamente en navigate.

### Cambios en `src/main.rs`

Pasar la factory al `NeoSession`:

```rust
let factory = RuntimeFactory {
    http_client: http_for_v8,
    cookie_store: cookie_store_arc.clone(),
    raw_client: raw_client.clone(),
    config: rt_config.clone(),
};

NeoSession::new_shared(...)
    .with_runtime_factory(factory)
```

---

## Qué se preserva en cross-origin nav

| Componente | Acción |
|---|---|
| V8 runtime | **DESTRUIR + RECREAR** |
| Módulos cargados | **DESTRUIR** (vienen con el runtime) |
| Import map | **DESTRUIR** |
| Script store | **LIMPIAR** |
| Timer budget | **RESET** |
| Fetch budget | **RESET** |
| DOM | **REPLACE** (ya lo hace) |
| Cookie store | **PRESERVAR** (domain-aware) |
| HTTP cache | **PRESERVAR** (disk-backed) |
| History stack | **PRESERVAR** |
| Page ID counter | **INCREMENTAR** |
| Network log | **PRESERVAR** |

---

## Qué se preserva en same-origin nav

| Componente | Acción |
|---|---|
| V8 runtime | **REUSAR** |
| Módulos cargados | **REUSAR** (SPA-friendly) |
| Import map | **REUSAR** |
| Script store | **REUSAR** |
| Timer budget | **RESET** |
| Fetch budget | **RESET** |
| DOM | **REPLACE** |
| Cookie store | **PRESERVAR** |
| Everything else | **PRESERVAR** |

---

## Tests necesarios

1. **Cross-origin nav**: browse site A → browse site B → no module collision
2. **Same-origin nav**: browse /page1 → browse /page2 → modules reutilizados
3. **Back cross-origin**: browse A → browse B → back → A re-bootstraps
4. **Cookies persist**: browse A (get cookies) → browse B → back to A → cookies still there
5. **Memory no crece**: 10 navegaciones cross-origin → memoria estable

---

## Estimación

- Implementación: 1 sesión
- Tests: 0.5 sesión
- Riesgo: BAJO (cambio localizado en navigate, no toca runtime internals)

---

## Origin extraction

```rust
fn extract_origin(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(u) => format!("{}://{}", u.scheme(), u.host_str().unwrap_or("")),
        Err(_) => String::new(),
    }
}
```
