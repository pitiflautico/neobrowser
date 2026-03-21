# NeoRender V2 — Process Log

## Método de trabajo

Para cada Tier:
1. Definir tareas granulares con PDR
2. Consultar GPT (sesión `v2-planning`) para validar
3. Incorporar feedback de GPT
4. Consultar GPT segunda vez para confirmar
5. SOLO ENTONCES lanzar agentes
6. Cada agente ejecuta pipeline/validate.sh (9 steps)
7. Verificar output del pipeline antes de aceptar
8. Tests reales contra sites
9. Commit solo si pipeline + tests pasan

## Log

### 2026-03-21 — Sesión 1

**Error cometido**: construimos Tiers 0-4 sin consultar GPT en cada paso. Los agentes no ejecutaron el pipeline de validación. Resultado: arquitectura limpia pero producto degradado (3/10 sites vs V1 10/10).

**Feedback GPT (review del estado real)**:
- P0: engine compile + tests reales
- P0: recuperar features V1 (hydration, pre-fetch, stubs, cache)
- P0: ejecutar quality gates DE VERDAD
- Fallo conceptual: separamos infra sin preservar performance primitives

### 2026-03-21 — Sesión 2

**Paso 1**: PDR recovery con 10 tareas (R1-R10)
**Paso 2**: Consulta GPT (sesión v2-planning) — GPT dice falta:
  - Semantic parity harness
  - Observability/tracing por fase
  - Compat contract para rewrite pipeline
  - Fixtures corpus antes de top sites
  - R7 y R8 demasiado gordos, dividir
  - Gate por cada R3-R8

**Paso 3**: PDR actualizado con feedback GPT. Segunda consulta GPT — confirma + añade:
  - R2.8 Baseline snapshots de artefactos intermedios
  - R2.9 Invariants/abort criteria por fase
  - R5.5 Generalizar scope rewrites
  - R7d Module resolution correctness
  - R8e Scheduler/task ordering parity
  - R9.0 Exit criteria exactos
  - R10.0 Freeze entorno benchmark

**Resultado**: PDR-RECOVERY-FINAL.md con 26 tareas, 7 fases (A-G), gates por cada feature.

**Paso 4**: Lanzado agente R1+R2 (quality gates, 10 crates, pipeline 9/9).

**Requisitos funcionales obligatorios para V2** (definidos por Dani):
- React hydration (ChatGPT, SPAs modernos)
- fill_form (con CSRF auto)
- navigate (multi-page, redirects, back/forward)
- click (con stale recovery)
- doubleclick
- scroll (infinite scroll)
- actions: type, select, check, submit
- Todos los hidratadores de V1 portados
- 10/10 top sites funcionando
