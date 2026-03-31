# PDR Recovery FINAL — Validado por GPT (2 rounds)

## Orden de ejecución (26 tareas)

### Fase A: Fix + Quality (secuencial)
- **R1**: Fix engine_tests compilation
- **R2**: Quality gates (fmt, unwrap, size <300, docs)

### Fase B: Infra de observabilidad (secuencial, antes de portar)
- **R2.5**: Observability — tracing por fase (fetch graph, stub decisions, cache hit/miss, hydration patches)
- **R2.6**: Compat contract — orden exacto del rewrite pipeline: pre-fetch → stub → source rewrite → bytecode cache → hydration patches
- **R2.7**: Parity harness + fixtures corpus (synthetic → known-hard → real sites)
- **R2.8**: Baseline snapshots de artefactos intermedios (graph.json, stub_map.json, rewritten_modules/, hydration_patch_log.json)
- **R2.9**: Invariants/abort criteria por fase (no ciclos, no pérdida exports, idempotencia)

### Fase C: Port V1 features (paralelo donde posible, con gate por cada una)
- **R3**: Pre-fetch modules (depth 2, parallel, disk cache)
  - Gate: unit tests + fixture SSR page with 5 modules + parity delta
- **R4**: Module stubbing (>1MB → proxy stubs)
  - Gate: unit tests + fixture heavy module + parity delta
- **R5**: Promise.allSettled source rewrite
  - Gate: unit tests + fixture module with allSettled
- **R5.5**: Generalizar scope rewrites (aliases, minified variants)
  - Gate: fixture with minified allSettled variant
- **R6**: V8 bytecode cache (save/load compiled bytecode)
  - Gate: unit tests + second load faster + parity delta

### Fase D: React hydration (secuencial, depende de C)
- **R7a**: React interception primitives (pipeThrough no-op, Object.prototype.getAll)
- **R7b**: Script rewrite (inline module → async IIFE, regex transforms)
- **R7c**: Entry module boot (direct load from __reactRouterManifest)
- **R7d**: Module resolution correctness (relative/absolute URLs, base href, import maps)
  - Gate por cada sub: fixture React app + parity with V1 hydration markers

### Fase E: Performance correctness (paralelo)
- **R8a**: Timer semantics correctness (microtask before macrotask, ordering parity)
- **R8b**: Fetch prioritization/budgeting (5s script fetch, 6s execution)
- **R8c**: Watchdog/abort (V8 terminate_execution 3s per script)
- **R8d**: Skip-list heuristics (73+ telemetry patterns)
- **R8e**: Scheduler/task ordering parity (interaction with timers/fetch/hydration)
  - Gate por cada sub: unit tests + fixture JS-heavy page

### Fase E.5: Interaction completeness (después de performance)
- **R8f**: doubleclick support (mousedown→mouseup→click→mousedown→mouseup→click→dblclick)
- **R8g**: right-click / context menu detection
- **R8h**: hover (mouseenter→mouseover sequence)
- **R8i**: keyboard events (keydown→keypress→keyup, Enter for submit)
- **R8j**: file upload in NeoSession (read file, build multipart)
  - Gate: fixture page with all interaction types, verify event sequence

### Fase F: Validation (secuencial)
- **R9.0**: Define exit criteria por site:
  - content extracted (links > 0, text > 100 chars)
  - forms detected (inputs, buttons counted)
  - classification correct (Article, DataTable, LoginForm, etc.)
  - hydration markers (routeModules for React, __vue_app__ for Vue)
  - interactive elements found (buttons, links with actions)
  - tolerancias: allow 10% variance between V1 and V2
- **R9**: Real sites validation tiers:
  1. Synthetic fixtures (local HTML)
  2. Known-hard (React SPA, Vue SPA, heavy JS)
  3. Top 10 (Google, ChatGPT, Reddit, Wikipedia, SO, Amazon, YouTube, GitHub, NYT, Netflix)
  - Cada site <15s, content extracted, classification correct

### Fase G: Benchmark (secuencial)
- **R10.0**: Freeze entorno (warm/cold cache, network, repetitions, hardware, timeout)
- **R10a**: Benchmark correctness parity (same output V1 vs V2)
- **R10b**: Benchmark performance (time-to-data, memory, cache hits)
- **R10c**: Benchmark stability (variance across 5 runs)

## Gate por cada R3-R8
Cada tarea debe terminar con:
1. Unit tests pasan
2. Fixture tests pasan
3. Parity delta report vs V1 generado
4. `pipeline/validate.sh` para el crate modificado pasa 9/9
5. Snapshot de artefactos intermedios guardado

## Dependencias
```
R1 → R2 → R2.5 → R2.6 → R2.7 → R2.8 → R2.9
                                         ↓
                              R3 + R4 + R5 + R6 (parallel)
                                    ↓
                              R5.5 (after R5)
                                    ↓
                              R7a → R7b → R7c → R7d
                                    ↓
                              R8a + R8b + R8c + R8d + R8e (parallel)
                                    ↓
                              R9.0 → R9
                                    ↓
                              R10.0 → R10a → R10b → R10c
```
