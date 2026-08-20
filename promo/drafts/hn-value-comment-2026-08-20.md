# Hacker News — value-first comment draft (2026-08-20)

## Hilo objetivo
Cualquier hilo reciente sobre AI agents, browser automation, MCP o "LLMs that can use the web".

## Borrador de comentario

The browser automation problem for agents is less about "can it click?" and more about "does the site trust the browser?".

A fresh headless profile has no cookies, no device history, and no reputation. It works for public pages, but the moment you need a real session it hits login walls, 2FA, and "unusual activity" checks. The model then spends its context budget on auth flows instead of the task.

The alternative is to drive the user's real Chrome via CDP. The agent inherits the user's existing trust state: cookies, localStorage, fingerprint, and even SSO sessions. The trade-off is latency (~4s vs ~1s per action in my measurements), but tasks behind real sessions actually complete.

I've been building around this idea and the surprising part is how much effort goes into *not* spoofing things. Passing bot.sannysoft is easy if you just don't lie about WebGL, hardwareConcurrency, or the UA; the hard part is defining "success" honestly when Cloudflare still blocks both approaches from a residential IP.

If you're building agents that need the real web, my suggestion: measure by task completion rate, not by how clean the benchmark screenshot looks.

---

## Reglas
- No link al repo en el primer comentario; solo si alguien pregunta.
- Tono: ingeniero pragmático, no evangelista.
- Publicar solo cuando la cuenta tenga karma suficiente y sin parecer promocional.
