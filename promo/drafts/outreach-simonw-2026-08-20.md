# Outreach draft — Simon Willison (@simonw)

**Vía:** reply en Mastodon (https://fedi.simonwillison.net/@simon) o Bluesky/X (@simonw) si encaja en un hilo sobre benchmarks/browser tooling; no hay email público. El mensaje debe ser un reply genuino a un hilo reciente, no un DM frío.
**Regla:** aportar valor primero; pedir opinión sobre metodología de benchmark, no difusión.

---

Subject: A brutally honest benchmark of browser MCPs (real Chrome vs Playwright MCP)

Hi Simon,

I've been running NeoBrowser, an MCP server that drives the user's real Chrome over CDP instead of a fresh headless browser. The hardest part turned out not to be the protocol, but defining what "passes bot detection" actually means.

I wrote up a head-to-head against Playwright MCP on live anti-bot pages and tried to be honest about where each wins:

- Playwright MCP is faster (~2.6s vs ~4.8s average).
- NeoBrowser passes tasks that need real sessions / uploads because it literally is the user's browser.
- On a residential IP, Cloudflare blocks both equally.

Methodology + raw JSON: https://github.com/pitiflautico/neobrowser/blob/main/bench/study.md

I'd love your take on what a fair benchmark for browser MCPs should measure. Most repos claim "passes bot detection" without defining the test, the IP, or the failure mode. I'm trying to fix that, but I may be missing dimensions that matter.

No pitch — just genuinely curious if this matches what you've seen.

— Daniel / NeoBrowser
https://github.com/pitiflautico/neobrowser

---

**Estado:** borrador listo. Envío pendiente del usuario (no tengo acceso a su cliente de correo ni a Mastodon/Bluesky).
