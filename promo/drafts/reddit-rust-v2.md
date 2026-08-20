# Borrador Reddit — r/rust v2

**Subreddit**: r/rust  
**Cuándo**: viernes (día del hilo "What's everyone working on?") o miércoles 9-11am ET  
**Regla**: r/rust premia el contenido técnico; evitar lenguaje de marketing.

---

## Opción A — post en el hilo semanal "What's everyone working on?"

```
NeoBrowser — an MCP server in Rust that drives the real Google Chrome binary over CDP

Repo: https://github.com/pitiflautico/neobrowser

What it does: lets LLMs/agents use the web through your own Chrome, with your own logged-in sessions if you opt in. The stealth comes from being genuine (real UA, real Client Hints, real GPU WebGL) rather than spoofing signals.

Tech bits that were fun:
- Tokio-based CDP client with one connection per tab, command multiplexing, and typed timeouts.
- Cross-platform cookie decryption (macOS Keychain, Linux secret-service, Windows DPAPI).
- Bot-wall detection (Cloudflare, reCAPTCHA, Turnstile, consent gates) instead of pretending to break them.
- Single static binary, ~6.4 MB.

I also wrote a neutral benchmark against Playwright MCP. Playwright wins on speed; NeoBrowser wins on session persistence and uploads. Both lose equally to Cloudflare on a single IP, because I'm not going to claim magic.

MIT, feedback welcome.
```

## Opción B — post standalone (si hay algo nuevo/único que mostrar)

**Título:**
```
I built an MCP server in Rust that drives your real Chrome instead of a headless clone
```

**Cuerpo:**
```
Most browser tools for LLMs spawn a fresh headless browser and hope the site doesn't notice. I went the other way: NeoBrowser drives the real Google Chrome binary over CDP, so the agent can use your actual sessions and real fingerprint.

Built in Rust because the protocol layer needed a real async runtime and careful process management. Some details:

- CDP multiplexer on tokio: each tab gets its own connection, commands routed by id, events buffered per tab.
- Real-session mode decrypts cookies from your Chrome profile via the OS keychain. Identity cookies are excluded so your real browser stays logged in.
- Stealth by consistency, not spoofing: the real UA matches real Client Hints, real GPU WebGL, real everything. Passes bot.sannysoft in CI.
- Detects walls instead of hammering them: captcha, consent, rate-limit, login gate — `navigate` reports what it found.
- 67 tools, single ~6.4 MB static binary.

I benchmarked it neutrally against Playwright MCP (same task matrix, common layer). Playwright is faster; NeoBrowser passes upload + session persistence tasks it can't. On adversarial pages both got blocked equally. Methodology is public in bench/.

Repo: https://github.com/pitiflautico/neobrowser (MIT)

Happy to answer questions about the CDP layer or the cross-platform cookie stuff.
```

## Notas
- Opción A es más segura para r/rust (formato de la comunidad).
- Opción B solo si hay un ganch técnico fuerte reciente (nueva release, benchmark, arquitectura).
- Responder a preguntas sobre Rendimiento, CDP, o seguridad con datos concretos.
