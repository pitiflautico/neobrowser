# Prep Product Hunt — assets listos para el launch (usuario publica)

**Cuándo lanzar**: martes o miércoles, 00:01 PT (para maximizar el día). Nunca lunes ni viernes.
**Requisito**: cuenta de Product Hunt; idealmente haber comentado/upvoteado antes unos días para que la cuenta no sea nueva.

---

## Ficha del producto

- **Name**: NeoBrowser
- **Tagline** (60 chars máx): `Your AI drives real Chrome — with your real logged-in sessions`
- **Topics**: Developer Tools, Open Source, Artificial Intelligence
- **Website**: https://pitiflautico.github.io/neobrowser/
- **GitHub**: https://github.com/pitiflautico/neobrowser

## Descripción (primera persona, PH style)

```
Hey Product Hunt 👋

I built NeoBrowser after watching every browser automation tool for AI fail the same way: fresh headless browser, no cookies, instant bot detection.

NeoBrowser drives your real Google Chrome via CDP — with your real logged-in sessions (opt-in), a genuine fingerprint (passes bot.sannysoft live in CI), human-cadence clicks, and first-class bot-wall detection so the agent reacts to CAPTCHAs instead of hallucinating success.

It's a single 5.6 MB Rust binary with 43 tools: multi-tab browsing, forms, upload/download, multi-source search, record/replay playbooks.

What I'm proudest of: the honest benchmark vs Playwright MCP in the repo. Playwright is faster; we do things it can't. Both get walled equally on adversarial pages. No hype.

Current bet: 88/10,000 GitHub stars, documented publicly at pitiflautico.github.io/neobrowser.

MIT licensed. Feedback welcome — especially from folks who've fought bot detection before.
```

## Galería (5 imágenes/vídeo, en orden)

1. **neobrowser-vs-headless.gif** (720×720, ~1 MB; el pitch visual en 6 segundos)
2. **demo.gif** (ya existe: `docs/assets/demo.gif` — 89 KB)
3. Screenshot de la landing hero (pitiflautico.github.io/neobrowser)
4. Screenshot de la tabla del benchmark (`bench/compare.md` renderizado)
5. Screenshot de la tabla "Why NeoBrowser" del README
6. Diagrama de arquitectura: "AI client → NeoBrowser → real Chrome → web (authenticated)" → **ya generado: `docs/assets/architecture.png`** (también embebido en la landing)

## First comment (el maker comment, publicarlo nada más lanzar)

```
Maker here — happy to answer anything. Two technical rabbit holes if you're curious: (1) cross-platform Chrome cookie decryption (Keychain/secret-service/DPAPI) done safely and opt-in, (2) why "genuine consistency" beats spoof stacking for fingerprint checks. Both are in the codebase, MIT.
```

## Checklist día del launch

- [ ] Publicar martes 25 a las 00:01 PT
- [ ] First comment inmediato
- [ ] Responder cada comentario en <30 min durante el día
- [ ] Cross-post el mismo día: Twitter ("we're on Product Hunt today"), no antes
- [ ] NO pedir upvotes en masa (viola ToS de PH) — sí avisar en tus canales propios
