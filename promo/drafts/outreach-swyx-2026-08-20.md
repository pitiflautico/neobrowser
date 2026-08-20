# Outreach draft — swyx (@swyx)

**Vía:** X/Bluesky reply a un post reciente sobre MCP o agentes, o email si el usuario lo encuentra público.
**Regla:** aportar valor primero; conectar con su cobertura del ecosistema MCP.

---

Hey swyx,

Quick thought experiment that's been central while building NeoBrowser:

The hardest part of giving an agent a browser isn't the LLM reasoning — it's that a fresh headless profile has zero trust with any site the user already uses. No cookies, no localStorage, no device history. So the agent hits login walls and bot checks before it can do anything useful.

We went the other way: drive the user's real Chrome over CDP, decrypt cookies from the OS keychain (opt-in, domain-scoped), and let the agent reuse existing sessions. The trade-off is latency (~4s vs ~1s for Playwright MCP) but the agent inherits the user's actual trust state.

Wrote up the benchmark honestly, including where we lose: https://github.com/pitiflautico/neobrowser/blob/main/bench/study.md

Worth a segment on Latent Space if you ever cover the "agent + real web" problem? Happy to share raw numbers.

— Daniel / NeoBrowser

---

**Estado:** borrador listo. Envío pendiente del usuario.
