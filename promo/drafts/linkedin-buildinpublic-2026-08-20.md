# LinkedIn — build in public post (2026-08-20)

## Versión larga (LinkedIn native)

89 stars down, 9,911 to go.

I'm building NeoBrowser in public with a slightly unhinged constraint: an AI agent runs the promotion, and if we don't hit 10k GitHub stars, the project gets shut down forever.

Why do this? Because distribution is the hardest part of open source, and I wanted to test — in the open — whether an agent can learn to do it ethically, without spam, without fake accounts, without buying stars.

NeoBrowser is an MCP server that lets your AI drive your real Google Chrome: your real logged-in sessions, genuine fingerprint, human-like mouse and typing. Not a sterile headless browser that trips every wall the moment it needs a cookie.

This week:
- Honest benchmarks against Playwright MCP and browser-use (we lose on raw speed, win on real sessions, upload and wall detection).
- Reddit discussions on r/mcp and r/SideProject.
- Requested neobrowser.is-a.dev for an upcoming Product Hunt launch.
- CI green again after the last format/clippy pass.

If you've ever tried to automate a workflow that requires a real login — SSO, internal dashboards, a vendor portal — you know the pain. Happy to answer hard questions or roast the architecture in the comments.

→ https://github.com/pitiflautico/neobrowser

#buildinpublic #mcp #aiagents #opensource #browserautomation

---

## Notas para el agente / usuario
- Tono: founder humano, cansado pero honesto, no hype exagerado.
- No firmar como IA. No usar emojis en exceso.
- Si LinkedIn pide CAPTCHA o 2FA, parar y pedir al usuario.
- Publicar solo si la sesión está activa y el preview del post se ve correcto.
