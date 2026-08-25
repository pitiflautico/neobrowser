# Competitor intel: browser-use

**Repo:** https://github.com/browser-use/browser-use  
**Homepage:** https://browser-use.com  
**Stars:** 110,506  
**Forks:** 12,158  
**Open issues:** 390  
**Stack:** Python + Playwright  
**Last release:** 0.13.8 (2026-08-16)

---

## Positioning

browser-use plays two games at once:

1. **Open-source adoption engine.** `pip install browser-use`, one-liner `Agent(task=...)`, lots of GIFs. Low friction = high share.
2. **Cloud revenue business.** Browser Use Cloud is the recommended production path: stealth, proxies, CAPTCHA solving, remote browsers, and their own `ChatBrowserUse` model.

Their message: *"tell your computer what to do and it does it."* Consumer-friendly dev tool aiming to be the default way AI agents use the browser.

## Key differences from NeoBrowser

| Dimension | browser-use | NeoBrowser |
|---|---|---|
| Architecture | Python on Playwright | Rust on CDP |
| Browser | Playwright / cloud browsers | User's real Google Chrome |
| Sessions | Real profiles supported, but cloud requires trusting their infra | Local Chrome, local sessions, no cloud |
| Stealth | "Stealth fingerprinting" as cloud upsell | Real events, no spoofing, mandatory sandbox |
| Business model | Open source + Cloud SaaS ($0.02/h + $0.17/task) | Standalone MCP server, no proprietary cloud |
| Integration | Python library + MCP skill | MCP-native server |
| Benchmarking | BU Bench V1, #1 on Odysseys leaderboard | Honest benchmark in `bench/`, not marketed as #1 |
| Sandbox | Cloud recommended for production | `--no-sandbox` never default |

## Growth tactics we can learn from

- **Visual one-liner demos.** README is full of GIFs and `Agent(task="...")` examples. This is their main viral fuel.
- **Systematic cloud upsell.** Almost every README section links to `cloud.browser-use.com`. Open source is the funnel; cloud is the revenue.
- **Agent ecosystem integration.** `browser-use skill install` for Claude Code, Cursor, Codex. They piggyback on other agents.
- **Own benchmark + leaderboard.** Credibility + marketing content.
- **Own LLM model.** `ChatBrowserUse` lets them monetize inference, not just infra.
- **Brand building.** Merch, Discord, X, LinkedIn, blog.
- **Aggressive pricing.** $0.02/browser-hour + $0.17/task, no subscription.

## Our differentiation opportunity

browser-use deliberately leaves a gap: **anything that requires not sending your browser, sessions, and credentials to someone else's cloud.**

NeoBrowser should own:

> **"Runs on your real Chrome. Your sessions. Your sandbox. No proprietary cloud."**

Tactical differentiators:

1. **Session sovereignty.** Already logged into LinkedIn/Gmail/bank? NeoBrowser uses it locally.
2. **Lightweight Rust binary.** ~5.5 MB vs Python + Playwright dependency chain.
3. **Security as a feature, not an upsell.** Real sandbox, no `--no-sandbox` default, origin-scoped credentials.
4. **No vendor lock-in.** Bring your own LLM and keys.
5. **MCP-first.** Designed as an MCP server from the start, not a Python library with an MCP wrapper.
6. **Ethical stealth.** Real browser, real events vs fingerprint spoofing.

## Social / contact intel

- X/Twitter: https://x.com/browser_use
- LinkedIn: https://www.linkedin.com/company/browser-use
- Discord: https://link.browser-use.com/discord
- Founders: Magnus Müller (`@mamagnus00` / `MagMueller`) and Gregor Žunič (`gregpr07`)

## Outreach recommendation

**Do not reach out for collaboration.** They are a well-funded 110k-star competitor with a cloud business. Direct contact mostly helps them study us.

**Do learn passively:** study their README structure, cloud funnel, benchmarks, and skill-install tactic. Replicate what fits our local-first positioning.

## Action items for NeoBrowser

1. Make README/landing contrast explicit: "Not another cloud agent. Your Chrome, controlled by AI."
2. Make installation in Cursor/Claude Code as easy as `browser-use skill install`.
3. Keep the honest benchmark narrative; consider a reproducible leaderboard entry.
4. Target developers and security/privacy teams, not casual end users.
5. Do not compete on cloud price. Compete on control, privacy, and local architecture.
