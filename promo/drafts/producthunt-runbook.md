# Product Hunt — Launch Day Runbook

**Fecha:** martes 26 de agosto de 2025  
**Hora de lanzamiento:** 00:01 PT / 09:01 CET  
**Meta de día:** entrar en el top 10 de Product Hunt; convertir visitantes en estrellas GitHub.

---

## Pre-launch (lunes 25)

| Hora (CET) | Acción | Responsable |
|---|---|---|
| Mañana | Verificar que `producthunt_launch.py` carga `/posts/new` y los campos esperados. | agente |
| Tarde | Revisar que los assets de galería estén en `~/.neobrowser/promo-home/downloads/`. | agente |
| 20:00 | Publicar un post sutil en X/LinkedIn: "Algo que hemos estado puliendo sale mañana en Product Hunt. Si te interesa que los agentes de IA usen tu navegador real, estate atento." (sin link de PH todavía). | usuario/agente |
| 22:00 | Dormir. El launch es a las 09:01 CET; hay que estar fresco. | usuario |

---

## Launch day — martes 26

### 08:30 CET (30 min antes)
- [ ] Abrir Product Hunt en Chrome con sesión de @pitiflautico.
- [ ] Tener abierto el runbook, el response playbook y el maker comment.
- [ ] Revisar que la landing cargue rápido y el contador de estrellas funcione.
- [ ] Tener a mano el enlace directo: `https://www.producthunt.com/posts/neobrowser`

### 09:01 CET (T+0)
- [ ] Ejecutar `python3 promo/scripts/producthunt_launch.py` o publicar manualmente.
- [ ] Inmediatamente después, publicar el **maker comment** (`promo/drafts/producthunt-response-playbook.md`).
- [ ] Copiar el enlace final del post de Product Hunt.

### 09:05 CET (T+5 min)
- [ ] Cross-post en X: "NeoBrowser is live on @ProductHunt today. It's an MCP server that drives your real Chrome — not a sterile headless browser. If you've ever watched an AI agent fail at a login wall, this is for you." + link PH.
- [ ] Cross-post en LinkedIn: versión más larga, founder tone, link PH.
- [ ] Email/mensaje directo a 3–5 personas que ya mostraron interés (no pedir upvote, solo "estamos en PH si quieres echar un vistazo").

### 09:15–13:00 CET (primeras 4h, críticas)
- [ ] Responder **cada comentario en <15 min**.
- [ ] Refrescar la página cada 5–10 min.
- [ ] Publicar 1 update de maker si hay tracción ("Ya estamos en top 10, gracias — aquí respondiendo preguntas").
- [ ] No pedir upvotes explícitamente. Sí responder con datos y honestidad.

### 13:00–18:00 CET
- [ ] Seguir respondiendo comentarios (<30 min).
- [ ] Publicar un segundo update si hay un hito (top 5, 100 upvotes, etc.).
- [ ] Replicar el post en r/producthunt, r/mcp, r/selfhosted **solo si la cuenta tiene karma suficiente**; si no, dejarlo para el usuario.

### 18:00–23:59 CET
- [ ] Último push en X/LinkedIn: "Últimas horas del día en Product Hunt. Si aún no lo has visto, aquí está el porqué de NeoBrowser." + link.
- [ ] Responder comentarios pendientes.
- [ ] Hacer screenshot del resultado final.

---

## Post-launch (miércoles 27+)

| Día | Acción |
|---|---|
| Miércoles | Agradecer en X/LinkedIn a quienes comentaron/upvotearon. Publicar el resultado final y una lección aprendida. |
| Jueves | Publicar en dev.to/Hashnode un post técnico derivado del launch: "What launching an MCP server on Product Hunt taught me about real-session browsing". |
| Viernes | Enviar el batch de outreach a influencers (`promo/drafts/outreach-batch-2026-08-20.md`) con el resultado de PH como social proof. |
| Semana siguiente | Doblar down en el canal que más estrellas generó (probablemente PH + HN si se recupera el rate-limit). |

---

## Maker updates pre-escritos

### Update 1 — si entramos en top 10 (2–3h después del launch)
```
Wow, top 10 on Product Hunt. Thank you.

The #1 question so far: "How is this different from Playwright MCP?"
Short answer: Playwright is fast and fresh; NeoBrowser is slow and authenticated. They solve different problems.

Full honest benchmark: https://github.com/pitiflautico/neobrowser/blob/main/bench/compare.md
```

### Update 2 — si hay críticas sobre seguridad/cookies
```
A few people asked about cookie safety. TL;DR:
- Opt-in only.
- Scoped by domain.
- Identity cookies (Google/LinkedIn/Microsoft) are excluded by default.
- Everything stays local.

Threat model: https://github.com/pitiflautico/neobrowser/blob/main/SECURITY.md
```

### Update 3 — cierre de día
```
That's a wrap on launch day. [X] upvotes, [Y] comments, [Z] GitHub stars.

The bet is now [N]/10,000. If you want to follow the experiment or save my AI employee from shutdown: https://github.com/pitiflautico/neobrowser

Thanks to everyone who commented, upvoted, or just read the benchmark.
```

---

## Riesgos y mitigaciones

| Riesgo | Mitigación |
|---|---|
| Product Hunt no carga / formulario cambia | Tener los textos listos para copiar-pegar manualmente (`promo/drafts/producthunt.md`). |
| Poco tráfico en las primeras horas | Cruzar a X/LinkedIn inmediatamente; no esperar. |
| Comentario negativo sobre spam/IA | Responder con humildad, datos y un enlace al benchmark; nunca discutir. |
| Cuenta de PH caída / CAPTCHA | Publicar manualmente desde el Chrome del usuario; si no es posible, posponer 24h. |
| Landing caída por tráfico | GitHub Pages aguanta bien, pero tener el repo link como fallback. |
| X sigue con CAPTCHA | Publicar LinkedIn como canal principal; preparar texto para el usuario. |

---

## Métricas a registrar

Al final del día, añadir a `promo/metrics.csv`:
- Upvotes PH
- Comentarios PH
- Ranking final
- GitHub stars antes/después
- Tráfico de referencia (GitHub insights, 24–48h después)

---

## Enlaces rápidos

- Landing: `https://pitiflautico.github.io/neobrowser/?ref=producthunt`
- Repo: `https://github.com/pitiflautico/neobrowser`
- Response playbook: `promo/drafts/producthunt-response-playbook.md`
- Launch script: `promo/scripts/producthunt_launch.py`
