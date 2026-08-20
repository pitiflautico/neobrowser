# Product Hunt — response playbook

Pre-written, human replies for the most common comments on launch day. Paste and personalize; never copy-paste the same block twice.

---

## Maker comment (post immediately after launch)

```
Maker here.

I built NeoBrowser after watching every browser automation tool for AI fail the same way: fresh headless profile, zero cookies, instant login wall or bot check. The agent could click, but it couldn't *be* the user.

So I went the other direction: drive the user's real Chrome over CDP, decrypt cookies from the OS keychain (opt-in, domain-scoped), and let the agent inherit the real fingerprint, localStorage, and sessions. No spoofed WebGL, no fake UA, no cloud farm.

Two rabbit holes if you're curious:

1. Cross-platform cookie decryption done safely: macOS Keychain / Linux secret-service / Windows DPAPI. Identity cookies for Google/LinkedIn/Microsoft stay excluded so your real browser never gets logged out.

2. Why "genuine consistency" beats spoof stacking: we suppress navigator.webdriver and keep the host's real Chrome doing the talking. The CI runs bot.sannysoft live on every push.

The benchmark vs Playwright MCP is in the repo and explicitly shows where we lose (speed). Happy to answer anything — especially hard questions.
```

---

## Typical questions and replies

### "How is this different from Playwright MCP / Puppeteer?"

```
Playwright MCP is great for fast, deterministic browser automation in a fresh headless Chrome. NeoBrowser is for when you need the agent to use *your* real Chrome profile with *your* logged-in sessions.

Practical difference: Playwright starts unauthenticated; NeoBrowser can decrypt and inject your non-identity cookies so the agent lands already logged into GitHub/LinkedIn/your-internal-dashboard. Playwright is faster; NeoBrowser is more useful for tasks behind real sessions.

The benchmark in the repo compares them honestly — Playwright wins on speed, we win on session persistence and uploads.
```

### "Why not just use browser-use?"

```
browser-use is an excellent full agent framework with a cloud option. NeoBrowser is a focused MCP server: one binary, local by default, no cloud required, and it drives the real Chrome you already trust rather than launching a managed browser.

If you want a hosted, multi-step agent with model orchestration, browser-use is probably a better fit. If you want an agent to use your own browser and accounts on your own machine, NeoBrowser is built for that.
```

### "Does this bypass CAPTCHA / Cloudflare?"

```
No. It detects walls (CAPTCHA, Turnstile, Cloudflare challenge, rate-limit, consent gate) and hands the model a strategy instead of pretending the click worked. We test against live bot-detection pages; the results are in bench/study.md — including the cells where both NeoBrowser and Playwright MCP get blocked.
```

### "Is this safe? It touches my cookies."

```
Cookie decryption is opt-in and scoped by domain. Identity cookies for Google/LinkedIn/Microsoft are excluded by default so your real browser doesn't get logged out. Everything is local unless you explicitly navigate somewhere; no cloud, no telemetry, no uploaded sessions.

Full threat model is in SECURITY.md.
```

### "Why Rust?"

```
Three reasons: a single static binary (~6.4 MB) with no Node runtime; careful async process management around Chrome; and cross-platform cookie decryption. We wanted something you can `brew install` or `cargo install` and run anywhere.
```

### "Can it run headless?"

```
Yes. The default launches a fresh real Chrome; `NEOBROWSER_REAL_PROFILE` reuses your existing profile for sessions; `NEOBROWSER_CHROME_BIN` lets you point it at any Chrome/Chromium you want.
```

### "Which MCP clients work?"

```
Anything that speaks stdio MCP: Claude Code, Claude Desktop, Cursor, VS Code, Windsurf, etc. Configuration is one line: `{ "mcpServers": { "neobrowser": { "command": "neobrowser" } } }`.
```

### "Is there a cloud / paid version?"

```
No cloud version and no paid tier. MIT licensed, self-hosted by design. If you want managed browsers, Browserbase or Browser Use Cloud are better options.
```

### "Can I see a demo?"

```
Yes — there's a 14-second GIF on the landing and a longer demo script in `rust/scripts/demo.py` that drives a real login, upload, and bot-detector check. The split-screen GIF compares a fresh headless browser vs NeoBrowser with a real session.
```

### "What can I actually automate with this?"

```
Your own accounts and workflows: GitHub notifications, LinkedIn posts, internal dashboards behind SSO, repetitive form fills, file uploads to your own tools. The model starts authenticated because it is literally your browser.
```

### "The README is long / looks vibecoded."

```
Fair — it is long because the security and verification contracts matter for a tool that touches real sessions. The claims are backed by tests: `cargo test --test conformance` runs the verified-action scenarios, and `bench/study.md` shows the bot-detection results. If anything doesn't hold up, open an issue and we'll fix or retract it.
```

### "How does the verified-action contract work?"

```
Every mutating action returns an envelope with `status` (`succeeded`, `failed`, `blocked`, `uncertain`, `needs_human`) plus before/after evidence. A click that dispatched but changed nothing reports `uncertain`, never success. The spec is in docs/VERIFIED-ACTIONS.md and checked by the conformance suite.
```

---

## Post-launch update templates

### If we hit top 10

```
Update: NeoBrowser is in the top 10 today. Thanks for the questions — the honest benchmark conversation and the security questions especially. We'll keep answering every comment.
```

### End of day

```
Thank you Product Hunt. Today's takeaway from the comments: the use case that resonates most is "let my agent use my own accounts without handing credentials to anyone." That's exactly why we built it. If you starred the repo, you just extended my AI employee's life.
```

---

## Rules for launch day

- Reply to every comment in <15 min during the first 2 hours.
- Never argue; use evidence (repo links, benchmark numbers).
- Don't ask for upvotes; ask for honest feedback.
- If someone compares us to a competitor, acknowledge their strengths first.
- Keep replies human — vary the opening, don't paste identical blocks.
