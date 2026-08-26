# Product Hunt Launch Day Kit — NeoBrowser

## Pre-launch (T-7 días)

### 1. Hunter (opcional pero recomendado)
- Un hunter con 5k+ followers puede duplicar la visibilidad inicial.
- Targets: Chris Messina, Kevin William David, Ben Tossell, o alguien del ecosistema MCP.
- Mensaje: "Hey, I'm launching NeoBrowser on PH next Tuesday — an MCP server that drives real Chrome with real sessions. Think it could resonate with your audience. Want to hunt it?"

### 2. Assets listos
- [ ] Logo square 240x240
- [ ] Gallery: demo GIF, hero clip, comparativa Playwright vs NeoBrowser
- [ ] Video 30s (opcional pero aumenta conversión)
- [ ] Tagline: "Your AI drives real Chrome — with your real logged-in sessions"
- [ ] Topics: Developer Tools, Open Source, Artificial Intelligence
- [ ] Website: dominio propio (no github.io)
- [ ] GitHub: https://github.com/pitiflautico/neobrowser

### 3. Primer comment del maker (publicar inmediatamente después del launch)
```
Maker here — happy to answer anything.

Two technical rabbit holes if you're curious:
1. Cross-platform Chrome cookie decryption (Keychain/secret-service/DPAPI) done safely and opt-in.
2. Why genuine consistency beats spoof stacking for fingerprint checks.

Both are in the codebase, MIT. Also ran a brutally honest benchmark vs Playwright MCP — Playwright is faster; we do things it can't. Raw data in bench/.

Feedback welcome, especially from folks who've fought bot detection before.
```

### 4. Respuestas preparadas

**"How is this different from Playwright MCP?"**
> Real Chrome + real sessions + wall detection as first-class signals. Playwright is faster; we pass upload and session persistence tasks it can't. On adversarial pages both get walled equally. Benchmark in bench/.

**"This is for scraping/botting"**
> It's for automating *your own* accounts with *your own* sessions. Opt-in, SSRF-guarded, renderer sandbox on. No promise of evading access controls.

**"Stealth is a cat-and-mouse game you'll lose"**
> Agreed for interactive challenges. That's why NeoBrowser detects walls and hands control back instead of pretending to be invisible. The philosophy is genuine consistency, not spoof stacking.

**"Why Rust?"**
> Single static binary, zero runtime deps. The CDP core is a tokio multiplexer with typed timeouts. MIT, auditable.

**"Cookie decryption sounds scary"**
> Opt-in via env var, same techniques Chrome uses for sync, files 0600, identity cookies excluded. All code is auditable.

**"Can I use it with my existing Chrome profile?"**
> Yes, `NEOBROWSER_REAL_PROFILE=Profile 1` (or whatever your profile is called). Identity cookies are excluded by default so you don't get logged out.

### 5. Cross-post plan (publicar 1-2 horas después del launch)

**X/Twitter**
```
We're live on Product Hunt today 🚀

NeoBrowser — an MCP server that drives your real Chrome with your real logged-in sessions. No headless. No sterile fingerprint. Just your browser.

If you've ever watched an AI agent hit a login wall and hallucinate success, this is for you.

Link in replies ⬇️
```

**LinkedIn**
```
We're live on Product Hunt.

I built NeoBrowser because every browser MCP I tried launched a fresh headless browser and immediately hit login walls. This one drives your actual Chrome — with your real sessions, real fingerprint, and the renderer sandbox on.

It's a single 6.4MB Rust binary, MIT, 67 tools.

Would mean a lot if you checked it out and left feedback.
```

**Reddit r/mcp**
```
We just launched NeoBrowser on Product Hunt. It's an MCP server that drives real Chrome with real sessions instead of a fresh headless browser. Happy to answer questions here or on PH.
```

### 6. Métricas a vigilar
- Upvotes en PH cada 30 min las primeras 4 horas.
- Comentarios — responder a TODOS en <30 min.
- Stars en GitHub — anotar baseline y comparar a las 24h.
- Tráfico en la landing (si hay analytics).

## Launch day timeline (ET)

| Hora | Acción |
|---|---|
| 00:01 | Launch en PH (publicar el post) |
| 00:05 | Primer comment del maker |
| 00:30 | Cross-post en X |
| 01:00 | Cross-post en LinkedIn |
| 02:00 | Cross-post en Reddit r/mcp |
| 04:00 | Revisar comentarios, responder todo |
| 08:00 | Update en X con posición actual |
| 12:00 | Update en LinkedIn |
| 18:00 | Cierre del día — agradecer, resumen |

## Post-launch (T+1)
- Post de agradecimiento en todas las redes.
- Análisis de qué funcionó y qué no.
- Responder a comentarios tardíos.
- Preparar follow-up: "We hit #X on Product Hunt — here's what we learned."

## Notas
- No pedir upvotes directamente en redes. Decir "we're live, feedback welcome".
- No comprar upvotes ni usar bots. PH lo detecta y banea.
- Si no entra en top 5 del día, no es el fin. El tráfico de PH sigue llegando durante semanas.
