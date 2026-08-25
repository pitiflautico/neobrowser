# Competitor / trend intelligence: Saik0s/mcp-browser-use

**Date:** 2026-08-25  
**Repo:** https://github.com/Saik0s/mcp-browser-use  
**Stars:** 957 · **Forks:** 113 · **Language:** Python  
**Last push:** 2026-02-11

---

## What it is

A Python MCP server that wraps the popular `browser-use` library and exposes it as a long-running HTTP daemon. AI clients call high-level tools like `run_browser_agent` or `run_deep_research` in plain English; the server plans, navigates, extracts, and synthesizes using an LLM-driven agent.

---

## Strengths

1. **Rides the `browser-use` wave.** `browser-use` is the best-known open browser-agent framework, so the repo inherits trust and search traffic.
2. **HTTP transport by default.** The author correctly identifies that stdio timeouts break 30–120 s browser tasks, so it runs as a daemon on `localhost:8383` with `streamable-http`.
3. **Web UI + dashboard.** Task viewer, live logs, skills browser, and history without touching the CLI.
4. **`run_deep_research`.** Built-in 3-phase research (plan → search → synthesize) is a concrete, high-value use case.
5. **Skills system (beta).** Records and replays learned browser workflows, managed via dashboard or CLI.
6. **LLM-provider flexibility.** Supports Anthropic, OpenAI, Google, Groq, DeepSeek, Ollama, Bedrock, etc.
7. **Chromium sandbox on by default.** `browser.chromium_sandbox` defaults to `true`, and it can connect to an existing Chrome via CDP URL.

---

## NeoBrowser differentiators

| Dimension | Saik0s/mcp-browser-use | NeoBrowser |
|---|---|---|
| **Runtime** | Python + uv + Playwright install | Single ~6 MB Rust binary |
| **Tool model** | Coarse: `run_browser_agent` does everything | Fine-grained MCP tools: `navigate`, `click`, `type`, `extract`, `screenshot`, etc. |
| **LLM cost** | Required for agent reasoning | Optional LLM fallback only for `find`; deterministic, pay-per-use not required |
| **Action verification** | Agent decides internally | Explicit **observe → act → verify** with state diffing |
| **Real sessions** | Persistent profile or CDP URL | Real-profile cookie import + decryption (Keychain/secret-service/DPAPI) |
| **Security model** | Sandbox + auth token | Policy classes, origin-scoped credentials, encrypted vault, audit trace, human approval gates |
| **Anti-detection** | Inherited from browser-use/Playwright | Genuine anti-detection (real events, no spoofing) |

Our message should sharpen the contrast: **"tools, not agents"** — deterministic, auditable, and safe enough to run against the user's real Chrome.

---

## Applicable tactics

1. **Offer an HTTP transport option.** Our stdio model is simple but long tasks can hit client timeouts. A `--http` / streamable-http mode would remove a common objection.
2. **Add a local task dashboard.** Even a minimal HTML page showing recent tool calls, screenshots, and traces increases trust and makes NeoBrowser feel "alive."
3. **Ship a `deep_research` playbook.** We already have `search` and `extract`; package a documented sequence or example script so users can replicate the competitor's headline feature.
4. **Lean into install speed.** One-line install (`curl | sh`) vs `uv sync + playwright install chromium` is a real conversion advantage — put it front-and-center in the README and directory submissions.
5. **Document deterministic cost.** Many teams are scared of agent loops burning tokens. Emphasize that NeoBrowser's core tools do not call an LLM.
6. **Consider skill / playbook recording.** Our `playbook` module can record sequences; expose a simple CLI/UI to replay them and close the gap with mcp-browser-use's skills system.

---

## Risks / watch items

- `browser-use` is well-funded and moving fast; it may add an official MCP server or acquisition path.
- Its natural-language UX is lower-friction for non-technical users. We should keep our README example simple and show one-shot English-to-result workflows.

---

## Verdict

**Primary threat in the "agentic browser MCP" mindshare.** Not a drop-in replacement, but the repo users compare us against. Compete on **determinism, security, and zero-dependency install** rather than on agent autonomy.
