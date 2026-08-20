# Borrador Reddit — r/mcp v2

**Subreddit**: r/mcp  
**Cuándo**: martes o miércoles, 9-11am ET  
**Regla**: quedarse a responder comentarios técnicos; no postear y huir.

---

## Título

```
NeoBrowser — an MCP server that drives your real Chrome (with your real sessions), benchmarked honestly vs Playwright MCP
```

## Cuerpo

```
I got tired of every browser MCP launching a fresh headless Chrome and immediately hitting login walls or bot checks. So I built the opposite.

NeoBrowser is an MCP server that drives the real Google Chrome binary over CDP. It can reuse your actual logged-in profile (opt-in, decrypts cookies via the OS keychain), so the model lands already authenticated. The fingerprint is genuine — same Chrome version, same GPU WebGL, same Client Hints — because it literally is your browser.

A few things that made it fun to build:

- The stealth isn't spoofed. No fake WebGL, no mismatched UA. It passes bot.sannysoft in CI using the host's real fingerprint.
- It detects walls instead of pretending to break them. reCAPTCHA, Turnstile, consent, rate-limit, login gate — `navigate` flags what it found and hands the model a strategy.
- Human-cadence input: clicks travel along eased paths with jitter; typing can be per-key with realistic timing.
- 67 tools, single ~6.4 MB static Rust binary, no Node runtime.

I also wrote a neutral benchmark against Playwright MCP (same task matrix, common layer). Playwright is faster on several tasks. NeoBrowser passes upload and session persistence tasks that Playwright MCP can't, and on adversarial pages both were walled equally. The numbers and methodology are in bench/ for anyone to audit.

Repo: https://github.com/pitiflautico/neobrowser (MIT)

If anyone is curious about the CDP multiplexer or cross-platform cookie decryption, happy to go deep in the comments.
```

## Notas
- Formato limpio: párrafos cortos + lista con contexto, no muro de texto.
- El ángulo es "resolvi un problema que me molestaba", no "usen mi producto".
- Responder primero a las preguntas técnicas; el link ya está arriba.
