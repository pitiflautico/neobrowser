# Newsletter pitches — NeoBrowser

Plantillas para contactar newsletters de dev tools / AI agents / Rust / open source.

---

## TLDR Newsletter

**Audience:** developers generalistas  
**Pitch:**
```
Subject: NeoBrowser — MCP server that drives your real Chrome (open source)

Hi TLDR team,

I built NeoBrowser, an open-source MCP server that lets AI agents control the user's real Google Chrome instead of launching a fresh headless browser.

Why it matters: most browser MCPs hit login walls and bot checks because they start with no cookies and a fingerprintable headless Chrome. NeoBrowser drives the real binary over CDP and can reuse the user's actual logged-in sessions (opt-in, OS keychain decryption).

I also published a reproducible benchmark against Playwright MCP on live bot-detection sites: NeoBrowser passes all sannysoft checks with its genuine fingerprint, while Playwright headless fails on User Agent. Both get blocked by real Cloudflare — no overclaim.

Repo: https://github.com/pitiflautico/neobrowser
Study: https://github.com/pitiflautico/neobrowser/blob/main/bench/study.md

Happy to share more details if it fits an upcoming issue.

Thanks,
Daniel
```

---

## This Week in Rust

**Audience:** comunidad Rust  
**Pitch:**
```
Subject: Project/tooling spotlight — NeoBrowser (Rust MCP server for browser automation)

Hi team,

NeoBrowser is a Rust MCP server that drives Google Chrome over the Chrome DevTools Protocol. It gives AI agents 43 browser tools (navigate, click, type, forms, upload/download, multi-tab, search, screenshots, playbooks) in a single ~6.4 MB static binary.

Built-in Rust for the CDP multiplexer (tokio, typed timeouts, one connection per tab) and cross-platform cookie decryption from the OS keychain. The repo includes live-Chrome tests, a bot-detection study vs Playwright MCP, and an honest benchmark.

Repo: https://github.com/pitiflautico/neobrowser

Would love a mention in the project/tooling section.
```

---

## Newsletter de AI agents / MCP

**Audience:** builders de agentes de IA  
**Pitch:**
```
Subject: NeoBrowser — real-browser MCP server (open source)

Hi,

Most MCP browser servers launch a headless Chrome and pray the site doesn't notice. NeoBrowser takes the opposite approach: it drives the user's real Chrome via CDP, so agents inherit real sessions, real fingerprints, and real trust.

Key bits:
- 67 tools, single static Rust binary
- Opt-in real-session mode via OS keychain cookie decryption
- Bot-wall detection (captcha/consent/rate-limit/login) as first-class signal
- Honest benchmark vs Playwright MCP published in the repo

Repo: https://github.com/pitiflautico/neobrowser
Study: https://github.com/pitiflautico/neobrowser/blob/main/bench/study.md

Let me know if you'd like a deeper dive or early access to upcoming features.
```

---

## Registro de envíos

| newsletter | fecha | contacto | estado | respuesta |
|---|---|---|---|---|
| | | | | |

Rellenar tras enviar.
