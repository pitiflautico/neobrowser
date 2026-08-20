# Borrador Show HN — listo para publicar (usuario)

**Cuándo**: entre semana, 9–11am ET (14–16h CET). Cuenta con algo de karma ayuda; si es cuenta nueva, mejor esperar a tener actividad previa.
**URL a compartir**: https://github.com/pitiflautico/neobrowser
**Importante**: responde a TODOS los comentarios las primeras 4-6 horas. Eso decide si sube o muere.

---

## Título (elige uno)

1. `Show HN: NeoBrowser – MCP server that drives real Chrome with your logged-in sessions` (recomendado, 86 chars — recorta si HN se queja: quita "logged-in ")
2. `Show HN: MCP server that drives real Chrome – genuine fingerprint, your real sessions`

## Texto del post

```
I built NeoBrowser because every browser MCP I tried had the same failure mode: it launches a fresh, fingerprintable headless browser with no cookies, so the model hits login walls and bot checks constantly.

NeoBrowser drives the real Google Chrome binary over CDP and can reuse your actual logged-in profile, so the model lands already authenticated and looks like a genuine user — because it is one.

What's different:

- Real sessions: optionally decrypts + injects cookies from your real Chrome profile (macOS Keychain / Linux secret-service / Windows DPAPI). Opt-in; session-identity cookies are excluded so your real browser isn't logged out.
- Genuine stealth, not spoofing: real UA matching its Client Hints, real GPU WebGL, navigator.webdriver gone. Passes bot.sannysoft live in CI. It doesn't pretend to beat interactive challenges — reCAPTCHA/Turnstile can still wall you — instead it detects the wall and tells the model how to react.
- Human-like input: clicks travel along an eased, jittered path; typing can be per-key with realistic timing.
- One ~6.4 MB static Rust binary, 67 tools (multi-tab, forms, upload/download, search, playbooks), zero runtime deps.

I also ran a neutral benchmark against Playwright MCP with a shared task matrix, nothing tuned to make either win. Honest results: Playwright MCP is faster (my headless frame-forcing costs ~2x latency); NeoBrowser passes upload + session persistence tasks Playwright MCP can't, and on adversarial pages both get walled equally. Full methodology in bench/ if you want to poke holes in it — I'd rather be called out than overclaim.

Repo: https://github.com/pitiflautico/neobrowser
```

## Respuestas preparadas (para comentarios típicos)

- **"How is this different from Playwright MCP?"** → Real Chrome + real sessions + wall detection as first-class signals; benchmark table; acknowledge Playwright is faster.
- **"This is for scraping/botting"** → Señalar la sección "Security & responsible use" del README: opt-in, SSRF-guarded, pensado para automatizar TUS propias cuentas. No promete evadir controles de acceso.
- **"Stealth is a cat-and-mouse game you'll lose"** → De acuerdo para challenges interactivos; por eso NeoBrowser detecta walls en vez de fingir invisibilidad. La filosofía es consistencia genuina, no apilar spoofs.
- **"Why Rust?"** → Single static binary, zero runtime; el core CDP es un multiplexer tokio con timeouts tipados; Python original se mantiene como oráculo de tests.
- **"Cookie decryption sounds scary"** → Es opt-in vía env var, mismas técnicas que usa Chrome para sincronizar; ficheros 0600; identity cookies excluidas; todo el código es auditable (MIT).
