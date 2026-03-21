# Tier Execution Recipe — OBLIGATORIO para cada Tier

## La receta (NO se puede saltar ningún paso)

### Paso 1: DEFINIR tareas granulares
- Escribir PDR del tier con tareas numeradas
- Cada tarea tiene: descripción, files, effort, dependencies, gate

### Paso 2: CONSULTAR GPT (sesión v2-planning)
- Enviar PDR a GPT
- Preguntar: "¿Qué falta? ¿El orden es correcto?"
- Incorporar feedback

### Paso 3: CONFIRMAR con GPT
- Enviar plan actualizado
- Preguntar: "¿Correcto? Solo sí/no + lo que falta"
- Si dice no → volver a paso 2
- Si dice sí → continuar

### Paso 4: LANZAR agentes
- Cada agente recibe spec YAML con:
  - Trait interface
  - Files to create/modify
  - Files forbidden
  - Acceptance criteria
  - Gate: pipeline/validate.sh debe pasar 9/9
- Agentes en paralelo donde no hay conflictos de files

### Paso 5: VERIFICAR cada agente
- Leer output del agente
- Ejecutar `pipeline/validate.sh` en los crates modificados
- Si falla → reenviar al agente con errores
- Si pasa → commit

### Paso 6: INTEGRACIÓN
- `cargo test --workspace` — todos los tests pasan
- `cargo clippy --workspace -- -D warnings` — 0 warnings
- Count total tests → anotar en PROCESS-LOG

### Paso 7: CONSULTAR GPT resultado
- Enviar estado actual (crates, tests, lo que funciona/falta)
- GPT dice qué falta antes del siguiente tier
- Incorporar feedback

### Paso 8: ACTUALIZAR docs
- PROCESS-LOG.md — anotar qué se hizo, resultado, feedback GPT
- CAPABILITY-MATRIX.md — actualizar estado features
- pipeline/state.json — actualizar wave/status

### Paso 9: COMMIT + siguiente tier
- Commit con mensaje descriptivo
- Solo avanzar al siguiente tier si GPT confirma que el actual está cerrado

## Checklist rápido (copiar y pegar para cada tier)

```
TIER [X]: [nombre]
[ ] Paso 1: PDR escrito
[ ] Paso 2: GPT review 1 — feedback incorporado
[ ] Paso 3: GPT confirmación — "sí"
[ ] Paso 4: Agentes lanzados con specs
[ ] Paso 5: Pipeline 9/9 en crates modificados
[ ] Paso 6: cargo test --workspace pasa
[ ] Paso 7: GPT review resultado
[ ] Paso 8: PROCESS-LOG + CAPABILITY-MATRIX actualizados
[ ] Paso 9: Commit + ready for next tier
```
