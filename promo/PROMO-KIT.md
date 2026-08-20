# Promo Kit — NeoBrowser (88★ / 4 forks)

Kit rápido para publicar contenido y usar sesiones reales sin repetir configuración.

## Sesión real de Chrome: dos métodos

### A) Cold Profile Mirror (recomendado para automatización)

Cierra Chrome, copia el perfil, vuelve a abrir Chrome, y publica contra la copia.

```bash
# 1. Cierra Chrome completamente (Cmd+Q).
# 2. Copia el perfil.
python3 promo/scripts/cold_profile_mirror.py

# 3. Vuelve a abrir tu Chrome normal.
# 4. Publica con el perfil copiado.
NEOBROWSER_PROFILE=real python3 promo/scripts/linkedin_post_mcp.py
NEOBROWSER_PROFILE=real python3 promo/scripts/reddit_post_mcp.py
```

### B) Attach Mode (más rápido, más interactivo)

Reinicia Chrome con `--remote-debugging-port` y conecta NeoBrowser al navegador real.

```bash
# Reinicia Chrome (guarda pestañas con --restore-last-session).
python3 promo/scripts/attach_mode_helper.py

# Publica contra el Chrome real.
NEOBROWSER_ATTACH_PORT=9222 python3 promo/scripts/linkedin_post_mcp.py
NEOBROWSER_ATTACH_PORT=9222 python3 promo/scripts/reddit_post_mcp.py
```

### C) Cookies por dominio (X solamente, sin cerrar Chrome)

Para X funciona con cookies inyectadas del perfil real:

```bash
python3 promo/scripts/x_post_mcp.py
```

## Borradores listos

| Plataforma | Archivo | Estado |
|---|---|---|
| LinkedIn | `promo/drafts/linkedin-next.md` | listo para publicar |
| Reddit r/selfhosted / r/mcp | `promo/drafts/reddit-next.md` | listo para publicar |
| HN outreach — Webctl | `promo/drafts/hn-outreach-webctl.md` | listo para publicar |
| HN outreach — BrowserOS | `promo/drafts/hn-outreach-browseros.md` | listo para publicar |
| dev.to artículo | `promo/drafts/devto-bridge.md` | listo para publicar |
| Product Hunt | `promo/drafts/producthunt.md` | listo para martes 25 00:01 PT |

## Directorios MCP

| Directorio | Estado | Acción |
|---|---|---|
| punkpeye/awesome-mcp-servers #12089 | OPEN | esperando merge |
| chatmcp/mcpso #3546 | OPEN | esperando merge |
| glama.ai | no indexado | esperando crawler |

## Métricas

Actualizar `promo/metrics.csv` después de cada acción:

```bash
date=$(date +%Y-%m-%d)
stars=$(gh api repos/pitiflautico/neobrowser --jq .stargazers_count)
forks=$(gh api repos/pitiflautico/neobrowser --jq .forks_count)
echo "$date,$stars,$forks,nota" >> promo/metrics.csv
```
