# Borrador Reddit — r/mcp (usuario publica)

**Subreddit**: r/mcp (principal). Alternativas: r/ClaudeAI (relee sus normas de autopromoción; suele tolerar herramientas útiles si aportan), r/LocalLLaMA NO (off-topic).
**Regla de oro de Reddit**: no postees y desaparezcas. Responde comentarios. Si el sub pide flair, usa el de "Tool/Project".

---

## Título

```
I built an MCP server that drives your real Chrome (with your logged-in sessions) — and benchmarked it honestly against Playwright MCP
```

## Cuerpo

```
Every browser MCP I tried had the same problem: fresh headless browser, no cookies, instant bot detection. The model spends half its tokens hitting login walls.

So I built NeoBrowser — it drives the real Chrome binary over CDP and can reuse your actual logged-in profile (opt-in, decrypts via OS keychain). The model lands already authenticated.

What it does differently:

• Genuine stealth, not spoofed — real UA matching Client Hints, real GPU WebGL. Passes bot.sannysoft in CI. It doesn't claim to beat reCAPTCHA/Turnstile; instead it *detects* the wall (captcha/consent/rate-limit/login) and hands the model a strategy.
• Human-cadence input — clicks move along eased paths, typing is per-key with realistic timing.
• Real sessions — cookie import is opt-in, identity cookies excluded so your browser isn't logged out.
• Single ~6.4 MB static Rust binary, 67 tools, no Node, no runtime.

I also benchmarked it against Playwright MCP with a neutral harness (same task matrix, common layer). Honest numbers: Playwright is ~2x faster on several tasks; NeoBrowser passes upload + session persistence tasks Playwright MCP can't do, and on adversarial pages both got walled equally. No "evades better" claims — methodology is in bench/ if you want to check my work.

Repo: https://github.com/pitiflautico/neobrowser (MIT)

Happy to answer technical questions — the CDP multiplexer and the cross-platform cookie decryption were the fun parts to build.
```

## Notas
- El último párrafo ("happy to answer") es deliberado: r/mcp premia al dev que se queda a hablar de detalles técnicos.
- Si alguien pregunta por casos de uso: automatizar TUS propias cuentas/flujos (LinkedIn, dashboards internos, webs con login), no scraping de terceros.
