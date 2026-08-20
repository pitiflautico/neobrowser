# Borrador Reddit — r/selfhosted v2

**Subreddit**: r/selfhosted  
**Cuándo**: miércoles o jueves, 9-11am ET  
**Regla**: r/selfhosted permite autopromoción si aporta valor; explicar el self-hosting angle claramente.

---

## Título

```
NeoBrowser — self-hosted MCP server that drives your real Chrome instead of a cloud headless browser
```

## Cuerpo

```
I've been running local LLMs and MCP servers at home, but every browser MCP I tried either spawns a fresh headless Chrome (instant bot detection) or calls a cloud browser service (not self-hosted, not my session).

So I built NeoBrowser: a single static Rust binary that drives the real Google Chrome on your own machine over CDP. It stays local, uses your own Chrome profile if you opt in, and never phones home to a browser-as-a-service.

Why it fits here:

- Fully local. One ~6.4 MB binary. No Docker, no Node, no cloud API.
- Your sessions stay yours. Cookie import is opt-in and decrypts via your OS keychain (macOS/Linux/Windows). Identity cookies for Google/LinkedIn/Microsoft are excluded so your real browser doesn't get kicked out.
- Genuine fingerprint. It passes bot.sannysoft using the real Chrome binary and real GPU WebGL, not spoofed signals.
- Bot-wall aware. It detects CAPTCHA/consent/rate-limit/login gates and tells the model instead of hammering the page.
- 67 tools: navigate, forms, upload/download, screenshot, multi-tab, search, playbooks.

I benchmarked it against Playwright MCP with a neutral harness. Playwright is faster; NeoBrowser does session persistence and uploads that Playwright MCP can't. Full methodology is public in the repo.

Repo: https://github.com/pitiflautico/neobrowser (MIT)

Curious if others here are using MCP servers locally and what your setup looks like.
```

## Notas
- Pregunta final genuina para generar comentarios y que no parezca spam puro.
- Enfatizar "self-hosted" y "local" en cada sección.
- Responder a comentarios sobre alternativas (Playwright MCP, browser-use, etc.) con datos, no hype.
