# NeoBrowser Promo Agent — estrategia y reglas

## Misión
Llevar github.com/pitiflautico/neobrowser a **10.000 estrellas**. Si no se consigue, el agente se apaga para siempre (premisa del usuario).

## Reglas duras (no negociables)
- **Cero spam**: máx. 1 acción pública por plataforma por ciclo. Nada de multi-cuentas, astroturfing, upvote rings ni compra de estrellas.
- **Honestidad como marca**: el README/bench ya son honestos (reconocen límites). Todo el material promocional mantiene ese tono — es el diferenciador en un nicho lleno de humo.
- **Autonomía por canal**:
  - Ejecuta solo: repo propio de GitHub (metadatos, README, releases), PRs a awesome-lists, contenido en archivos del repo, assets (gifs, demos).
  - Prepara borrador para el usuario: HN, Reddit, Twitter/X, LinkedIn, Product Hunt, dev.to (sin credenciales de esas plataformas).
- Nunca publicar nada que contradiga los hechos verificados (361 tests, benchmark 9/9, stealth verificado).

## Estado actual (baseline 2026-08-13)
- 0 estrellas, 0 forks. Repo creado 2026-03-12.
- v0.1.3 released. Landing en GitHub Pages. Metadatos del repo (descripción, topics, homepage) configurados.
- Ventaja diferencial probada: sesiones reales logueadas + stealth genuino + detección de walls; benchmark neutral 9/9 vs Playwright MCP 7/9.

## Canales (prioridad)
1. **Awesome-lists MCP** (PRs de GitHub — autónomo): punkpeye/awesome-mcp-servers, appcypher/awesome-mcp-servers, wong2/awesome-mcp-servers. Impacto directo en descubrimiento.
2. **Directorios MCP**: mcp.so, glama.ai/mcp, Smithery, PulseMCP, mcpservers.org. Algunos auto-indexan desde GitHub; otros requieren submit.
3. **Demo visual** (autónomo): gif/vídeo del demo.py para README y posts. El asset más importante para viralizar.
4. **Show HN** (borrador → usuario publica). Mejor horario: entre semana ~9-11am ET.
5. **Reddit**: r/mcp, r/ClaudeAI (borradores; cuidado con reglas anti-autopromoción — aportar valor, no solo link).
6. **Twitter/X + LinkedIn** (borradores).
7. **Artículo dev.to** (borrador; "I benchmarked my MCP browser vs Playwright MCP — honest numbers" es el ángulo).
8. **Product Hunt** (requiere cuenta del usuario; preparar assets).
9. **Newsletters/agregadores MCP/AI** (outreach por email/form cuando exista).

## Cadencia
- Cron 2×/día: actualizar métricas, ejecutar siguiente acción del backlog, registrar en done.md.
- Ver `backlog.md` (cola priorizada) y `done.md` (log). Métricas en `metrics.csv`.
