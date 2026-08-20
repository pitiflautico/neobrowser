# Borrador Reddit — ciclo 2026-08-20

## r/selfhosted

**Título**: NeoBrowser — self-hosted MCP server that drives your own Chrome for AI agents

**Cuerpo**:
```
If you're running local LLMs with tool use, you've probably noticed that "browser" tools either need cloud APIs or launch headless Chromium that fails on anything with a login wall.

NeoBrowser is a single Rust binary (~5.6 MB) that acts as an MCP server and drives *your* Chrome. Attach to an existing profile (your sessions, cookies, extensions) or let it launch a fresh real Chrome. Everything stays local unless you explicitly navigate somewhere.

What changed this week: we had to tighten real-profile cookie injection because platforms were logging users out when they detected a cloned session. Now it's opt-in per domain (`NEOBROWSER_REAL_PROFILE_DOMAINS=x.com,reddit.com`). Lesson learned: more cookies != more real.

GIF of the difference (6s): https://pitiflautico.github.io/neobrowser/assets/neobrowser-vs-headless.gif

Repo: https://github.com/pitiflautico/neobrowser

Caveats are in the README: it's not faster than Playwright for pure scraping, and it won't bypass Cloudflare on sites that hate automation — nothing honest does. Questions welcome.
```

## r/mcp

**Título**: [Showcase] NeoBrowser — MCP server that drives your real Chrome instead of a blank headless browser

**Cuerpo**:
```
Hey r/mcp,

We've been hitting a wall with agents and real websites: the moment a site needs a logged-in session, a fresh headless browser becomes useless.

NeoBrowser is an MCP server that drives *your* actual Chrome (or launches a real one) with your real profiles and sessions. It exposes the usual tools — navigate, click, type, screenshot, extract, search — but the browser behind them is genuinely yours, not a sterile puppet.

Key bits:
- Single static Rust binary (~5.6 MB), zero runtime dependencies.
- Real Chrome with real sessions (attach to your own or let it launch one).
- Genuine anti-detection: real WebGL, real permissions, real trust signals — no spoofing.
- Verified-action contract + audit log for destructive ops.
- Honest benchmark vs Playwright MCP published in the repo.

This week's fix: real-profile cookie injection is now allow-list per domain, after we saw platforms log users out on cloned sessions.

GIF: https://pitiflautico.github.io/neobrowser/assets/neobrowser-vs-headless.gif
Repo: https://github.com/pitiflautico/neobrowser

We're at 88 GitHub stars. Happy to answer questions or take punches on the benchmark methodology.
```

## Notas
- Publicar solo una versión (no spam en ambos subreddits el mismo día).
- Si la sesión de Reddit no está viva, dejar como borrador para publicación manual.
- Mejor horario: mañana ET para r/mcp, tarde ET para r/selfhosted.
