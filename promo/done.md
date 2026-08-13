# Log de acciones

## 2026-08-13 — ciclo 0 (setup)
- Repo metadata: descripción, homepage (landing Pages), 12 topics añadidos.
- Workspace promo/ creado (estrategia, backlog, métricas).
- Cron de promoción programado 2×/día (id d77462a9).
- Release v0.1.3: workflow OK (binarios multiplataforma publicados), CI verde, landing desplegada en Pages.
- PR abierto a punkpeye/awesome-mcp-servers: https://github.com/punkpeye/awesome-mcp-servers/pull/12089 (sección Browser Automation, fast-track 🤖). Pendiente de merge.

## 2026-08-13 — ciclo 1
- Métricas: 0★ / 0 forks (sin cambio aún — esperado, PR #12089 aún OPEN).
- appcypher/awesome-mcp-servers: **archivado**, no acepta PRs. Descartado.
- wong2/awesome-mcp-servers: **PRs desactivados** (404 en /pulls). Descartado. El ecosistema awesome-list se ha consolidado en punkpeye — buena noticia: un solo merge cubre el canal.
- glama.ai: NeoBrowser NO indexado (solo hay "NexBrowser", homónimo ajeno). Pendiente; los topics nuevos pueden disparar auto-indexación.
- README: badges de instalación 1-click para VS Code y Cursor (commit cbaf80b, pusheado).
## 2026-08-13 — ciclo 3
- PR #12089 (punkpeye): sigue OPEN sin merge. Estrellas: 0.
- mcp.so: submission creada como issue en chatmcp/mcpso (#3546), formato estándar del directorio.
- Borrador Show HN completo en promo/drafts/show-hn.md (2 títulos, texto, respuestas preparadas a objeciones típicas).
- Investigación de directorios: el MCP Registry oficial es el canal canónico 2026 (alimenta downstream); sigue bloqueado por OAuth interactivo del usuario.

## 2026-08-13 — ciclo 2
- demo.gif generado: grabación asciinema (venv /tmp/promo-tools) del demo con wrapper streaming (/tmp/demo_live.py, mismos pasos MCP que rust/scripts/demo.py), convertido a GIF con pyte+Pillow (/tmp/cast2gif.py). 14 frames, 931x448, 89 KB. Embebido en README.md y docs/index.html.

## 2026-08-13 — ciclo 4
- PR #12089 y issue mcp.so #3546: ambos siguen OPEN. Estrellas: 0.
- Borradores listos para el usuario: r/mcp, hilo Twitter/X (con demo.gif adjunto), LinkedIn (ES, tono first-person).

## 2026-08-13 — ciclo 5
- PulseMCP: no estamos listados; **submissions pausadas hasta mediados de agosto** (rework de su ingestion). Reintentar en próximos ciclos. Bonus: la comprobación se hizo con el propio NeoBrowser — curl recibía 403 de Cloudflare, NeoBrowser pasó, detectó el captcha y leyó la página. Dogfooding + anécdota de marketing.
- Smithery: no estamos listados; el submit requiere cuenta/CLI auth → pendiente credenciales del usuario.
- Mantenimiento: actualizado el binario instalado ~/.local/bin/neobrowser a v0.1.3 (había un server viejo vivo de otra sesión con un Chrome fugado; limpiado). Ojo macOS: al copiar el binario a mano, quitar el xattr com.apple.provenance o Gatekeeper lo mata (SIGKILL) en el primer exec.

## 2026-08-14 — ciclo 6
- MCP Registry oficial: `server.json` creado en la raíz y **validado contra el schema oficial 2025-12-11** (variante websiteUrl, descripción 89 chars). Instrucciones exactas para el usuario en `promo/drafts/registry-publish.md` — solo falta `mcp-publisher login github` (OAuth interactivo).
- glama.ai: sigue sin indexar (solo el homónimo NexBrowser). Tener server.json en la raíz puede ayudar a su crawler; re-check en próximos ciclos.
- Estrellas: 0. PR #12089 y issue mcp.so #3546 siguen OPEN.

## 2026-08-14 — ciclo 7
- Borrador artículo dev.to (`promo/drafts/devto.md`): ángulo "benchmark honesto vs Playwright MCP", con front matter listo y nota sobre la API de dev.to (si el usuario pasa DEV API key, el agente puede publicar drafts).
- Prep completa de Product Hunt (`promo/drafts/producthunt.md`): tagline, descripción, galería, maker comment y checklist del día de launch.
- Estrellas: 0. PR #12089 y mcp.so #3546 siguen OPEN.

## 2026-08-14 — ciclo 8
- Monitor de menciones: 0 reales (la única en HN es un artículo de 2022 sobre "neobrowsers" como categoría, sin relación). Estrellas: 0; PR #12089 OPEN.
- Newsletters: research hecho — no hay forms públicos de submission relevantes que no requieran cuenta/email del usuario; canal aplazado a cuando haya tracción que enseñar.
- Asset nuevo: `docs/assets/og.png` (1200×630, captura real de la landing hecha con el propio NeoBrowser) + meta tags og:image/twitter:card en la landing — los links compartidos en X/LinkedIn/PH ahora salen con imagen.

## 2026-08-14 — ciclo 9
- PulseMCP re-check (con NeoBrowser): submissions SIGUEN pausadas. Reintentar próximos ciclos.
- Nuevo contenido: `promo/drafts/tech-cookies.md` — deep-dive "How to decrypt Chrome cookies on macOS/Linux/Windows without being evil", verificado contra cookies.rs real. Para publicar 1-2 semanas después del primer artículo.
- Estrellas: 0. PR #12089 y mcp.so #3546 OPEN.

## 2026-08-14 — ciclo 10
- Nuevo contenido: `promo/drafts/tech-cdp.md` — deep-dive "Multiplexing CDP in Rust: one reader, zero races", verificado contra cdp.rs real. Cierra la serie técnica (benchmark → cookies → CDP).
- Estrellas: 0. PR #12089 y mcp.so #3546 OPEN.
- El backlog autónomo de contenido queda agotado; próximos ciclos = mantenimiento (re-checks directorios, monitor, mejoras de assets si hacen falta).

## 2026-08-14 — ciclo 11
- Nuevo asset: `docs/assets/architecture.png` (diagrama AI client → NeoBrowser → real Chrome → web, dark theme acorde a la landing), embebido en la landing y referenciado como imagen 5 de la galería de Product Hunt. Todos los assets de PH quedan completos.
- Estrellas: 0. PR #12089 y mcp.so #3546 OPEN.
