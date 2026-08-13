# Backlog priorizado (el agente coge la primera pendiente cada ciclo)

## Ahora (impacto inmediato, ejecutable autónomo)
- [x] PR a punkpeye/awesome-mcp-servers añadiendo NeoBrowser → [PR #12089](https://github.com/punkpeye/awesome-mcp-servers/pull/12089) OPEN, pendiente de merge
- [x] ~~PR a appcypher/awesome-mcp-servers~~ **imposible: repo archivado** (solo lectura, no acepta PRs). Entrada ya preparada en fork por si reaparece.
- [x] ~~PR a wong2/awesome-mcp-servers~~ **imposible: PRs desactivados** en ese repo (verificado vía API).
- [x] demo.gif generado → docs/assets/demo.gif, embebido en README y landing
- [ ] Publicar en el **MCP Registry oficial** (modelcontextprotocol/registry) vía `mcp-publisher` — canal de mayor leverage, alimenta agregadores downstream. Requiere login GitHub OAuth interactivo → necesita al usuario 1 minuto
- [ ] Verificar/submit en glama.ai (hoy NO está indexado — solo aparece "NexBrowser", que no somos nosotros); los nuevos topics pueden disparar la auto-indexación, re-chequear en próximos ciclos
- [x] Submit a mcp.so → issue creado: https://github.com/chatmcp/mcpso/issues/3546 (pendiente de revisión)
- [~] PulseMCP: no listados y **submissions pausadas hasta mediados de agosto** (rework de ingestion). Reintentar submit en ciclos a partir del ~18 ago. La comprobación se hizo con NeoBrowser itself (curl recibía 403; NeoBrowser pasó) — anécdota usable en contenido.
- [ ] Submit a Smithery (smithery.ai) — requiere cuenta/auth del usuario

## Contenido (borradores para el usuario)
- [x] Borrador Show HN → `promo/drafts/show-hn.md` (título + texto + respuestas preparadas)
- [x] Borrador post r/mcp → `promo/drafts/reddit.md`
- [x] Borrador hilo Twitter/X con demo → `promo/drafts/twitter.md`
- [x] Borrador LinkedIn → `promo/drafts/linkedin.md`
- [x] Borrador artículo dev.to (benchmark honesto como gancho) → `promo/drafts/devto.md`

## Después
- [x] Preparar launch de Product Hunt → `promo/drafts/producthunt.md` (tagline, descripción, galería, maker comment, checklist)
- [ ] Buscar newsletters MCP/AI-agent con form de submission y enviar
- [ ] Monitor semanal de menciones ("neobrowser") en HN/Reddit/GitHub y responder donde aporte
- [ ] Ideas de contenido técnico: post sobre cookie decryption cross-platform, post sobre el CDP multiplexer en Rust

## Reglas del backlog
- Máx. 2 acciones por ciclo. Si una acción es PR externo, comprobar antes que no existe ya (no duplicar).
- Si todo lo autónomo está hecho, dedicar el ciclo a mejorar el asset más débil (demo, docs, README).
