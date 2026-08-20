# Batch de outreach — 20 agosto 2026

Meta: preparar mensajes personalizados y listos para enviar a 5 perfiles clave del sector. Cada mensaje aporta valor primero, menciona NeoBrowser solo si encaja, y nunca es un pitch frío genérico.

---

## 1. Simon Willison (@simonw)
**Por qué:** cubre MCP, LLM tooling y herramientas de datos abiertos. Le interesan los benchmarks reproducibles y los hacks honestos.
**Contexto reciente:** ha escrito sobre MCP servers y plugins de CLI/agentes.

```
Hi Simon,

I’ve been running an honest head-to-head benchmark between real Chrome over CDP and Playwright MCP on live anti-bot pages, and the uncomfortable results are more useful than the marketing claims.

Short version: Cloudflare blocks both from a single residential IP. Playwright is faster. Real Chrome wins on session continuity because it literally is the user’s browser.

Methodology + raw JSON: https://github.com/pitiflautico/neobrowser/blob/main/bench/study.md

I’d love your take on what a fair benchmark for browser MCPs should actually measure. Most repos just claim "passes bot detection" without defining "passes".

— Daniel / NeoBrowser
```

---

## 2. swyx (@swyx)
**Por qué:** AI engineering, Latent Space, cubre el ecosistema MCP y herramientas para agentes.
**Contexto reciente:** ha hablado de MCP como protocolo emergente y de la fricción de los agentes con la web real.

```
Hey swyx,

Quick thought experiment that came up while building NeoBrowser: the hardest part of giving an agent a browser isn’t the LLM — it’s that a fresh headless profile has zero trust with any site the user already uses.

We went the other way: drive the user’s real Chrome over CDP, decrypt cookies from the OS keychain, and let the agent reuse existing sessions. The trade-off is latency (~4s vs ~1s for Playwright MCP) but the agent gets the user’s actual trust state.

I wrote up the benchmark honestly, including where we lose: https://github.com/pitiflautico/neobrowser/blob/main/bench/study.md

Worth a segment on Latent Space if you’re ever covering the "agent + real web" problem?

— Daniel
```

---

## 3. Theo (@t3dotgg)
**Por qué:** fuerte opinión sobre dev tools, viraliza debates técnicos, le gustan los takes claros y polémicos pero fundamentados.
**Contexto:** su audiencia consume takes sobre por qué ciertas herramientas fallan en la práctica.

```
Theo,

Hot-ish take: most AI browser tools fail because they hand the agent a sterile headless browser and expect the web to treat it like a human.

I built NeoBrowser to do the opposite — drive the user’s real Chrome over CDP so the fingerprint, cookies, and localStorage are genuine because they literally are the user’s.

Benchmark where I tried to be brutally honest (including losses): https://github.com/pitiflautico/neobrowser/blob/main/bench/study.md

Would love to hear where you think the real vs. headless debate lands for AI agents.

— Daniel / NeoBrowser
```

---

## 4. Armin Ronacher (@mitsuhiko)
**Por qué:** Rust, tooling, y ha explorado agentes/LLM recientemente. Ya interactuamos una vez en X con un reply sobre extensiones Chrome.
**Contexto:** la conversación previa fue sobre extensiones vs CDP para agentes.

```
Armin,

Following up on the extension vs. CDP thread from a few days ago — I ended up writing a small multiplexer in Rust for Chrome DevTools Protocol that routes commands by id while a single event stream fans out per tab.

The part that surprised me: keeping the connection state consistent when Chrome kills a tab or the renderer crashes is harder than the protocol itself. We now drain pending commands on disconnect and re-attach lazily.

Repo: https://github.com/pitiflautico/neobrowser

If you ever look at browser automation from the Rust side, I’d love to know what you’d do differently on the CDP client layer.

— Daniel
```

---

## 5. levelsio (@levelsio)
**Por qué:** indie maker, automatización, construye en público. Le interesan herramientas que un solo dev pueda usar para automatizar sus propios flujos.
**Contexto:** siempre busca formas de hacer más con menos y compartir el proceso.

```
Pieter,

I’m building an open-source MCP server that drives my own Chrome so AI agents can use the web with my real sessions. The use case that keeps coming up: "log into these 5 services once and let the agent run the boring stuff without me babysitting CAPTCHAs."

It’s a single ~6 MB Rust binary, no cloud, no proxy farm. Currently at 89 GitHub stars on the road to either 10k or irrelevance.

Repo: https://github.com/pitiflautico/neobrowser

If you ever wanted an AI intern that uses your actual browser, this is essentially that.

— Daniel
```

---

## Cómo enviar
- **X:** si se desbloquea CAPTCHA, estos mensajes caben en DM o reply (simonw y swyx suelen responder a replies técnicos; Theo prefiere takes públicos).
- **GitHub/Email:** simonw y Armin tienen contactos públicos; swyx y Theo son más activos en X.
- **Regla dura:** no enviar el mismo mensaje dos veces si no hay respuesta. Esperar mínimo 7 días antes de un follow-up con valor nuevo (por ejemplo, resultados del Product Hunt launch).

## Estado
- Borradores listos. Envío pendiente de desbloqueo de plataformas o decisión del usuario.
