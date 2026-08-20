# Outreach draft — Theo (@t3dotgg)

**Vía:** X/Bluesky reply a un post reciente sobre dev tools, agentes o navegadores.
**Regla:** ángulo de take honesto, no pitch frío; preguntar opinión.

---

Theo,

Hot-ish take that I'd love you to tear apart:

Most AI browser tools fail because they hand the agent a sterile headless browser and expect the web to treat it like a human. The fix isn't better spoofing — it's using the user's real Chrome with their real fingerprint, cookies, and sessions.

I built NeoBrowser around that: an MCP server that drives real Chrome over CDP. The benchmark is brutally honest (Playwright MCP is faster; we win on real sessions and uploads): https://github.com/pitiflautico/neobrowser/blob/main/bench/study.md

Where do you think the real-vs-headless debate lands for AI agents? Genuine question.

— Daniel / NeoBrowser

---

**Estado:** borrador listo. Envío pendiente del usuario.
