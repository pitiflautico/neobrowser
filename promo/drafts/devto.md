# Borrador artículo dev.to (usuario publica)

**Cómo publicar**: dev.to → "Create Post" → pegar el markdown de abajo. Tags: `mcp`, `rust`, `ai`, `webdev`. Cover image: el demo.gif no vale como cover (dev.to quiere estática 1000x420); genera una captura del gif o usa la landing.
**Alternativa automatizable**: dev.to tiene API (DEV API key en settings) — si me pasas la key, el agente puede publicar borradores directamente.

---

```markdown
---
title: I benchmarked my browser MCP against Playwright MCP — here are the honest numbers
published: false
tags: mcp, rust, ai, webdev
---

Every "AI uses the web" tool has the same dirty secret: it launches a fresh, fingerprintable headless browser with no cookies. The model then spends half its context window bouncing off login walls and bot checks.

I built [NeoBrowser](https://github.com/pitiflautico/neobrowser) to fix that — it drives the *real* Google Chrome binary over the Chrome DevTools Protocol and can reuse your actual logged-in sessions. Then I did the thing most tool authors avoid: I benchmarked it against Microsoft's Playwright MCP with a neutral harness, and published numbers that don't flatter me.

## The setup

A shared task matrix drives both tools through the same abstract steps: navigation, reading, login flows, DOM extraction, SPA rendering, screenshots, multi-tab, file upload, session persistence, and crash recovery. Nothing tuned to make either side win — the same neutral layer talks to both.

Two metrics, kept separate on purpose:

- `task_execution_success` — the steps ran
- `destination_access_success` — you actually reached the content (a walled page is exec-success but access-failure, so a detected wall never inflates the score)

## The numbers

| | NeoBrowser | Playwright MCP |
|---|---|---|
| Functional tasks | **9/9** | 7/9 |
| Avg latency | 4760 ms | **2597 ms** |
| Crashes | 0 | 0 |

Playwright MCP is roughly 2x faster on several tasks. That's a deliberate trade-off on my side: NeoBrowser forces compositor frames so deferred/virtualized content actually renders in headless Chrome — Playwright skips that and sometimes reads an empty page fast.

Where NeoBrowser wins outright:

- **Session persistence** — save/restore cookies, resume authenticated. Playwright MCP exposes no such tool.
- **Upload** — works via CDP `setFileInputFiles`, independent of the native file chooser.

And on adversarial pages (Cloudflare, Google Images)? **Both got walled equally** from a single IP. No "evades better" claim — that would need residential proxies and repeated runs, and I'd rather be called out than overclaim.

## What NeoBrowser actually is

- Real Chrome, real fingerprint: UA rewritten to the installed Chrome version so Client Hints stay consistent, real GPU WebGL, `navigator.webdriver` gone. Passes bot.sannysoft in CI on every push.
- Real sessions (opt-in): decrypts your Chrome profile cookies via macOS Keychain / Linux secret-service / Windows DPAPI. Identity cookies excluded so your browser isn't logged out.
- Bot-wall aware: `navigate` detects captchas, consent gates, rate limits, login walls and tells the model how to react instead of hammering.
- One ~5 MB static Rust binary, 43 tools, zero runtime deps.

## Lessons

1. **Stealth is consistency, not camouflage.** Every spoofed value that mismatches a genuine signal is a detection vector. Real beats fake.
2. **Publish benchmarks that can hurt you.** The credibility is worth more than the win.
3. **Detecting a wall is a feature.** An agent that knows it's blocked beats one that hallucinates success.

Repo: https://github.com/pitiflautico/neobrowser — MIT, PRs welcome. The full benchmark methodology is in `bench/` if you want to poke holes in it. Please do.
```

## Notas
- `published: false` lo crea como draft en dev.to si usas la API; si lo pegas en el editor, ignora el front matter y revisa antes de publicar.
- Mejor horario dev.to: martes-jueves mañana ET.
