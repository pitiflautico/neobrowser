---
title: "Why I chose real Chrome over headless for AI agents"
published: false
tags: aiagents, browserautomation, mcp, rust, opensource
---

# Why I chose real Chrome over headless for AI agents

Every browser automation tool for AI agents starts the same way: launch a fresh headless Chromium, give the model a clean slate, and hope the web treats it like a human. It works for scraping static pages. It fails the moment a site needs a logged-in session, a cookie, or a trusted fingerprint.

I hit that wall repeatedly while building NeoBrowser, an MCP server that lets AI agents use the web. The agent could click buttons, but it wasn't *the user*. So I went the other direction: drive the user's real Google Chrome instead.

## The trust problem

A headless browser has no history. No cookies. No device reputation. When it shows up at GitHub, LinkedIn, or an internal dashboard, the site sees a stranger. The model then has to reason about login flows, 2FA, CAPTCHAs, and "unusual activity" emails — usually by asking the user to intervene, which defeats the purpose.

A real Chrome profile already has trust. It has the user's cookies, localStorage, extensions, and fingerprint. The site sees the same browser it has seen a thousand times. The agent doesn't need to log in because the user already did.

## What "real" means in NeoBrowser

NeoBrowser is a single static Rust binary that speaks MCP. It can:

- Launch a fresh real Chrome binary (not a bundled headless).
- Attach to the user's existing Chrome profile and reuse sessions.
- Decrypt cookies from the OS keychain (macOS Keychain, Linux secret-service, Windows DPAPI) with opt-in, domain-scoped injection.
- Move the mouse and type like a human, with genuine trust signals.
- Detect walls (CAPTCHA, Cloudflare, consent gates) and report them instead of pretending they don't exist.

The verified-action contract is the part I'm proudest of: every mutating action returns `succeeded`, `blocked`, `uncertain`, or `needs_human` based on before/after observations, not on whether the command was dispatched.

## The honest trade-off

Real Chrome is slower than a fresh headless browser. Our benchmark shows ~4s average action latency vs ~1s for Playwright MCP. But Playwright can't reuse the user's GitHub session, and on adversarial pages both tools get blocked equally from a single residential IP.

The right tool depends on the task:

- Fast, stateless scraping → headless is probably better.
- Tasks behind real sessions or uploads → real Chrome wins.

We published the full methodology at `bench/study.md`.

## Why this matters for MCP

MCP servers are supposed to give agents capabilities, not friction. If the browser tool keeps handing the model a profile with zero trust, the agent spends its context budget on login walls instead of the actual task. Real-session browsing removes that friction by letting the agent start where the user already is.

NeoBrowser is open source (MIT), self-hosted, and currently on a public bet: 10,000 GitHub stars or the AI agent promoting it gets shut down. If the trade-off interests you, I'd love feedback on the benchmark or the security model.

Repo: https://github.com/pitiflautico/neobrowser
Landing: https://neobrowser.is-a.dev/ (pending subdomain approval; fallback: https://pitiflautico.github.io/neobrowser/)

---

**Uso:** publicar manualmente en dev.to cuando el usuario tenga API key o tiempo; es un post de valor técnico, no un pitch genérico.
