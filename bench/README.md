# NeoBrowser benchmark

A reproducible harness that drives an MCP browser tool through a task matrix and
reports **task-success rate, bot-wall detection rate, latency, crashes, and
self-healing recovery**. It ships a NeoBrowser adapter and is built so other tools
(Playwright MCP, browser-use, manual Chrome) plug in behind the same interface.

```bash
python3 bench/run.py               # runs NeoBrowser against bench/tasks.json
# -> bench/report.md + bench/report.json
```

## First-pass results (NeoBrowser)

Single machine, single IP, single run — a first evidence pass, not the full
comparative matrix. From `bench/report.md`:

| metric | value |
|---|---|
| task success rate | **12/12 = 100%** |
| bot-wall detection | 3/12 (login, Google-images `bot_wall`, nowsecure.nl `captcha`) |
| avg latency / task | ~4.0 s (dominated by real page loads + render waits) |
| crashes | 0 |
| crash-recovery (kill Chrome mid-session → self-heal) | **PASS** |

Coverage: navigation, login, file upload, table extraction, SPA/deferred loading,
long navigation, JSON APIs, **bot-wall detection** (incl. a live Cloudflare
challenge page correctly flagged), multi-tab, crash recovery, persistent sessions.

## What this does and does NOT prove (honest)

- **Does** show NeoBrowser completes real functional tasks reliably, detects walls
  correctly (Cloudflare/Google), and self-heals from a killed browser.
- **Does not** claim a *bypass rate* against Cloudflare / DataDome / Akamai /
  PerimeterX. Those are adversarial and **IP-reputation sensitive**: repeated runs
  from one IP skew their own results. A credible bypass benchmark needs
  **residential proxies + many runs + statistical treatment**, and warmed accounts.
  Here we measure wall **detection** and content **reachability**, not evasion.
- **tokens/task** is intentionally absent: a tool server consumes no model tokens;
  that metric only exists inside an agent loop. Add it via an LLM-driven adapter.

## Extending to the full matrix

The task specs (`tasks.json`) are tool-agnostic abstract steps
(`navigate/fill/click/find/upload/expect/...`). To compare tools, implement an
adapter with the same `call(name, args) -> {text, isError}` surface:

- **Playwright MCP** — spawn `npx @playwright/mcp`, map abstract steps to its tools
  (`browser_navigate`, `browser_click`, `browser_snapshot`, …).
- **browser-use** — wrap its agent; this is also where `tokens/task` becomes real.
- **Manual Chrome** — a human baseline for success-rate/latency sanity.

For adversarial sites at scale, run the same `tasks.json` (expanded to 50–100
sites) through a residential-proxy pool, N repetitions per site, and aggregate with
confidence intervals. The harness records per-task wall/latency/crash already; only
the site list, proxy rotation, and repetition loop need adding.
