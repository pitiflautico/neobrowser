---
title: "I tested my own browser MCP against Playwright MCP on live bot detection — here's the honest table"
published: false
description: "A reproducible head-to-head between NeoBrowser (real Chrome via CDP) and Playwright MCP on sannysoft, creepjs, nowsecure.nl and deviceandbrowserinfo.com. No overclaiming."
tags: mcp, browserautomation, aiagents, rust, playwright
---

Every browser automation tool for AI agents claims to "pass bot detection." Most of those claims are based on a single synthetic test site, or worse, they don't mention where they fail.

I built [NeoBrowser](https://github.com/pitiflautico/neobrowser), an MCP server that drives the real Google Chrome binary over CDP. It can reuse your actual logged-in profile, so the model lands already authenticated. The stealth comes from being genuine — real UA, real GPU WebGL, real Client Hints — not from spoofing signals.

But I don't want to overclaim. So I ran a reproducible head-to-head against Playwright MCP on live anti-bot pages.

## The setup

- **NeoBrowser:** 0.1.7 (real Chrome via CDP)
- **Playwright MCP:** `npx -y @playwright/mcp@latest --headless`
- **Environment:** same machine, same IP, no proxies
- **Harness:** identical JS blobs per target, shared wall classifier
- **Runs per cell:** N=2 (enough to show stability, not for statistical claims)

Targets:

- **bot.sannysoft.com** — the classic fingerprint test suite
- **creepjs** — trust-score based detection
- **nowsecure.nl** — real Cloudflare challenge page
- **deviceandbrowserinfo.com/info** — general browser info page

## The honest table

| target | NeoBrowser | Playwright MCP headless |
|---|---|---|
| sannysoft | 11/11 pass | 10/11 pass (fails `UA: HeadlessChrome`) |
| nowsecure.nl | blocked (Cloudflare challenge) | blocked (Cloudflare challenge) |
| latency | ~4 s | ~1 s |

CreepJS loaded for both but its trust score wasn't present at read time, so I report "not read" rather than pass/fail.

## The uncomfortable truths

**Cloudflare blocked both tools.** From a single residential IP, no browser automation tool reliably bypasses a real Cloudflare wall. If someone tells you otherwise, ask for reproducible evidence.

**Playwright MCP is faster.** 3-5× on every target. NeoBrowser forces frames so deferred content actually renders; that's the correctness-over-speed trade-off.

**`navigator.webdriver` is weird.** NeoBrowser reads `undefined`, Playwright headless reads `false`. A real human Chrome reads `false`. `undefined` is itself an automated tell, but none of the test sites flagged it.

## What NeoBrowser is actually for

Not bypassing strangers' anti-bot systems. Automating *your own* accounts and workflows:

- Your LinkedIn, already logged in.
- Your GitHub notifications.
- Your internal dashboards behind SSO.

The model starts authenticated because it is literally your browser. When a wall appears, NeoBrowser detects it and hands the model a strategy instead of pretending the click worked.

## Reproducibility

Full methodology, raw JSON, and the harness are in the repo:

<https://github.com/pitiflautico/neobrowser/blob/main/bench/study.md>

I'd rather be called out for a bad measurement than quietly overclaim.

---

*Disclosure: I built NeoBrowser. The repo is MIT-licensed; the benchmark is open for anyone to audit or break.*
