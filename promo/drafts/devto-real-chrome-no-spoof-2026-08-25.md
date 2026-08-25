# Why I stopped spoofing headless browsers and started driving real Chrome

Every browser automation tool for AI agents makes the same promise: "We look human."

Then they do one of three things:

1. Patch `navigator.webdriver` to `false`.
2. Randomize the WebGL fingerprint.
3. Launch a "stealth" fork of Chromium that is itself a signal.

I've spent the last few months building an MCP server called **NeoBrowser**, and I think that whole arms race is the wrong direction. Here's why.

## The problem with spoofing

Bot detection is not a checklist of static values. It's a *consistency* check.

When you fake a WebGL vendor, you also have to fake the renderer, the driver, the GL version, the unmasked vendor, the hardware concurrency, the UA-CH headers, the platform, the screen size, the timezone, the fonts, and the perf metrics. Miss one, and the site scores you as "inconsistent" — which is bot-detection speak for "bot."

Worse, every time a site adds a new signal, your patch list gets longer. You're playing whack-a-mole against teams whose full-time job is to find moles.

## The alternative: use the browser the user already trusts

NeoBrowser doesn't spoof. It drives the user's real Google Chrome over the Chrome DevTools Protocol. Same executable, same profile, same cookies, same TLS fingerprint, same mouse events.

The agent inherits the user's actual trust state. Logged-in dashboards just work. CAPTCHAs are rare because the browser isn't a fresh anonymous process. When a wall does appear, we detect it and stop rather than hallucinate success.

The trade-off is honest: it's not the fastest tool on the bench. Playwright MCP wins on raw speed. But there are flows it simply can't do, and walls it hits just as hard.

We published the benchmark so you can see for yourself:
https://github.com/pitiflautico/neobrowser/blob/main/bench/study.md

## What changed in my mental model

I used to think the goal was to make a headless browser indistinguishable from a real one. Now I think the goal is to **not need the distinction at all**.

If the agent is using the same browser the user logs into every day, the question "is this a bot?" becomes "is the user delegating control?" — which is a policy question, not a fingerprint question. That's a much easier problem to reason about.

## The safety piece

Driving a real browser with real sessions is powerful, so the safety model has to be explicit:

- Origin-scoped credentials: a cookie for `example.com` never leaves `example.com`.
- Human-approval gates for mutating actions.
- Audit logs with redaction.
- Renderer sandbox on by default.
- No identity-cookie cloning for Google/LinkedIn/Microsoft/GitHub/etc.

The last one is important. Cloning a session cookie can make the provider log the user's real browser out. We hold those back by default.

## Try it

NeoBrowser is MIT-licensed and ships as a single ~6 MB Rust binary:

```bash
cargo install --git https://github.com/pitiflautico/neobrowser
```

Or use it with any MCP client (Claude, Cursor, Cline, etc.).

I'd love feedback, especially from anyone who's fought bot detection before. The repo has a live issue tracker and a public star bet: 95 → 10,000.

---

*Cross-posted from the NeoBrowser build log. Follow the experiment at https://github.com/pitiflautico/neobrowser.*
