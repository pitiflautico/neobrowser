# PDR: V2 Recovery — Paridad funcional con V1

## Situación

V2 tiene buena arquitectura (10 crates, 148 tests, traits) pero funciona peor que V1:
- V1: 10/10 top sites, React hydration, 3.1s ChatGPT, 54 tests reales
- V2: 3-4/10 top sites, sin hydration, sin module cache, 148 tests unitarios

## Objetivo

V2 debe hacer TODO lo que V1 hace + tener mejor arquitectura.

## Tareas de Recovery (ordenadas por impacto)

### R1: Fix engine_tests compilation [S]
- MockHttpClient falta `block_pattern` y `is_blocked`
- Añadir a mock.rs
- engine_tests deben compilar y pasar

### R2: Quality gates — cumplir pipeline [M]
- `cargo fmt --all`
- Ficheros >300 líneas: split (cookies.rs 340, file_tracer.rs 302)
- Eliminar 8 unwrap() fuera de tests (reemplazar con `?` o `.ok()`)
- Añadir doc comments mínimos en pub items sin docs
- `bash pipeline/validate.sh neo-http` debe pasar 9/9
- `bash pipeline/validate.sh neo-trace` debe pasar 9/9

### R3: Portar V1 module pre-fetch a V2 [L]
V1 code: neobrowser-rs/src/neorender/session.rs steps 5-7
- Extract scripts from HTML (inline + external)
- Fetch external scripts (10s timeout, disk cache)
- Pre-fetch ES module imports (depth 2, parallel, 8s budget)
- modulepreload link extraction
- Disk cache: ~/.neorender/cache/modules/

Destino V2: neo-runtime/src/modules.rs + neo-engine/src/session.rs

### R4: Portar V1 module stubbing a V2 [M]
V1 code: neobrowser-rs/src/neorender/v8_runtime.rs + session.rs step 7b
- Módulos >1MB no referenciados en HTML → stub con Proxy
- extract_export_names() + generate_stub_module()
- Configurable via NEOBROWSER_STUB_THRESHOLD

Destino V2: neo-runtime/src/modules.rs

### R5: Portar V1 Promise.allSettled source rewrite a V2 [S]
V1 code: neobrowser-rs/src/neorender/v8_runtime.rs NeoModuleLoader
- `code.replace("Promise.allSettled(", "((ps)=>Promise.all(...))(")`
- Aplicar en module loader pre-fetch Y on-demand

Destino V2: neo-runtime/src/modules.rs

### R6: Portar V1 V8 bytecode cache a V2 [M]
V1 code: neobrowser-rs/src/neorender/v8_runtime.rs V8CodeCache
- Guardar bytecode compilado en ~/.neorender/cache/v8/
- Hash del source para invalidación
- ModuleSource.code_cache field

Destino V2: neo-runtime/src/modules.rs o nuevo v8_cache.rs

### R7: Portar V1 React hydration patches a V2 [L]
V1 code: neobrowser-rs/src/neorender/session.rs steps 10b, 11
- pipeThrough no-op (bootstrap.js)
- Object.prototype.getAll fallback
- Inline module → async IIFE (regex transform)
- Dynamic import() base URL resolution
- Entry module direct load from __reactRouterManifest
- SSR stream close/drain

Destino V2: neo-engine/src/session.rs + js/bootstrap.js

### R8: Portar V1 timer/fetch optimizations a V2 [M]
V1 code: neobrowser-rs/src/neorender/ops.rs + session.rs
- Timer: 1ms min, 10ms max (ya parcial en V2)
- setInterval: 10 ticks max (ya en V2)
- Script execution budget: 6s total
- Script fetch budget: 5s total
- Heavy script skip >200KB
- V8 terminate_execution() watchdog 3s per script
- Telemetry URL skip list (73+ patterns)

Destino V2: neo-runtime/src/ops.rs + neo-engine/src/session.rs

### R9: Test battery — 10 top sites [L]
- Google, ChatGPT, Reddit, Wikipedia, SO, Amazon, YouTube, GitHub, NYT, Netflix
- Cada site debe cargar en <15s
- Content extraído (links >0 para todos)
- Script tests/test_v2_sites.sh

### R10: Benchmark V1 vs V2 [M]
- Mismo script, mismos sites
- Medir: render time, nodes extraídos, links, page_type
- V2 debe igualar o superar V1

## Dependencias

```
R1 (fix compilation) → R2 (quality) → PARALLEL:
  R3 (pre-fetch) + R4 (stubs) + R5 (allSettled) + R6 (cache)
    → R7 (React hydration)
    → R8 (timer/fetch optimizations)
      → R9 (10 sites test)
        → R10 (benchmark V1 vs V2)
```

## Verificación final

```bash
# Quality
bash pipeline/validate.sh neo-http   # 9/9
bash pipeline/validate.sh neo-trace  # 9/9
bash pipeline/validate.sh neo-engine # 9/9
# ... todos los crates

# Functional
bash tests/test_v2_sites.sh  # 10/10 sites

# Benchmark
bash tests/benchmark_v1_v2.sh  # V2 >= V1
```
