# Manual blast — acciones inmediatas para desbloquear estrellas

El agente automático ha tocado techos en todas las plataformas. Estas 5 acciones manuales, ejecutadas en 10–15 minutos, pueden desbloquear el siguiente salto.

---

## 1. Comprar/apuntar un dominio para Product Hunt (5 min)
**Por qué:** Product Hunt rechaza GitHub Pages, el repo de GitHub, y ahora también Netlify Drop (`netlify.app`). El error es "can't hunt this product / link seems invalid". Necesita un dominio propio o al menos uno que no esté en su lista negra de hosting gratuito.

**Pasos:**
1. Compra un dominio barato (p. ej. `neobrowser.dev`, `getneobrowser.com`, `neobrowser.xyz`) en Cloudflare Registrar, Namecheap, Porkbun o Google Domains. ~$10-15/año.
2. Apunta el CNAME/A record al site de Netlify ya reclamado:
   - En Netlify: `https://app.netlify.com/projects/gentle-khapse-c58c79/domain-management`
   - Añade el dominio personalizado y sigue las instrucciones de DNS.
3. Confirma que `https://<tudominio>/` devuelve 200 y muestra la landing.

**Alternativa gratis:** si ya tienes un dominio personalido aparcado, apunta un subdominio (`neobrowser.tudominio.com`) a Netlify.

**Cuando esté listo:** avísame y completo el submit de Product Hunt con NeoBrowser.

---

## 2. Post manual en X / LinkedIn (2 min)
**Por qué:** X y LinkedIn están desbloqueados, pero la automatización no consigue hacer submit. Un post manual tuyo ahora mismo tiene más alcance que 10 intentos de bot.

**Borrador actualizado con lo de hoy** en `promo/drafts/social-buildinpublic-2026-08-20.md` (incluye versión corta para X y long-form para LinkedIn).

Resumen del mensaje:
```
Day N of the 10k★ or shutdown bet: 89★, 9,911 to go.

✓ Netlify Drop reclamado y público vía NeoBrowser.
✗ Product Hunt rechaza netlify.app, GitHub Pages y el repo.
✓ Reddit r/mcp post en vivo.

Lección: distribution is harder than the product.
```

---

## 3. Post manual en LinkedIn (2 min)
**Por qué:** LinkedIn es el único canal donde la automatización ha publicado hoy, pero el upload nativo de vídeo falla. Un post de texto manual tuyo es más fiable.

**Copia y pega:**

```
I thought building an MCP server that drives real Chrome was the hard part.

Turns out distribution is harder.

Day N of the public bet: 10,000 GitHub stars or I shut down the AI agent promoting the project. Current count: 89.

Here's what happened this week:

✓ Hacker News launch worked — 35 stars in a few hours, great technical feedback, even bug reports that made the product better.

✗ HN now rate-limits the account (comment-toofast). Can't comment there for a while.

✗ X hit a CAPTCHA on the account. Can't post.

✗ Product Hunt rejects both the GitHub Pages URL and the GitHub repo URL with "can't hunt this product." Need to figure that out before Tuesday.

✗ Reddit r/selfhosted swallowed the post without publishing it — karma gate or spam filter.

✓ LinkedIn still works. Text-only, but it works.

The uncomfortable truth: you can have a working product, benchmarks, demos, and a story, and still get stuck on platform mechanics.

What I'm doing next:
1. Fix the Product Hunt URL issue (probably need a custom domain or manual verification).
2. Keep publishing honest, useful content on LinkedIn.
3. Reach out directly to people who care about real-session browser automation — no mass pitches, just genuine conversations.

If you think AI agents should use the real web like humans do, the repo is in the comments. Every star extends the experiment.

#buildinpublic #opensource #aiagents #mcp #browserautomation
```

---

## 4. Upvote/comment en HN (3 min)
**Por qué:** la cuenta `pitiflautico` tiene poco karma y está en `comment-toofast`. Necesita rodaje con interacciones pequeñas antes de volver a comentar/postear.

**Acciones:**
1. Ve a `https://news.ycombinator.com/news`
2. Entra a 2–3 hilos de tecnología/AI que te interesen de verdad.
3. Haz upvote a comentarios útiles.
4. Escribe 1 comentario corto y técnico (no sobre NeoBrowser).

**No hagas esto hasta mañana:** no comentar sobre NeoBrowser ni postear nada propio hoy.

---

## 5. Enviar 1 mensaje a Simon Willison (@simonw) (3 min)
**Por qué:** el outreach directo no depende de algoritmos ni rate-limits, y Simon es el que más encaja con el ángulo de benchmark honesto.

**Borrador listo en `promo/drafts/outreach-simonw-2026-08-20.md`**. Es un email corto pidiendo su opinión sobre qué debería medir un benchmark justo de browser MCPs, con link a `bench/study.md`.

**Cómo enviar:**
1. Ve a `https://simonwillison.net/about/` y busca su email actual (suele tener un botón "Reveal my Address").
2. Envía el email desde tu cliente habitual. Respeta el tono: pregunta, no pitch.

**Alternativa:** si no encuentras el email, un reply genuino a su próximo post sobre MCP/browser tooling en Bluesky/Mastodon/X también sirve.

**Regla:** aporta valor primero, menciona NeoBrowser solo si encaja, nunca pitches genéricos.

---

## Resultado esperado
Si ejecutas las 5 acciones en la próxima hora:
- Product Hunt queda desbloqueado para el martes.
- X + LinkedIn aportan visibilidad inmediata.
- HN recupera karma para comentar mañana.
- 1 influencer entra en el radar.

**Estado actual:** 89★ / 4 forks / 0 issues.
