---
title: How I stopped my browser agent from logging users out of their real Chrome
published: false
description: The first version of my MCP server imported every cookie from the user's real profile. Google detected the cloned session and revoked the original login. Here's the fix.
tags: rust, mcp, chrome, security
---

## The bug

I built an MCP server that drives the user's real Chrome over CDP. The first version imported every cookie from the real profile into the automated browser. The idea was simple: more cookies = more "real" = less bot detection.

Then a user emailed me: "Your tool logged me out of Gmail."

My first reaction was "impossible — we don't write to your profile." But we didn't need to. The problem was that we imported *too many* cookies into a *different* browser, and Google detected the inconsistency.

## What actually happened

When you use your real Chrome profile, Google sets session cookies like `GMAIL_AT`, `OSID`, `GAUSR`, and fingerprint cookies like `AEC`, `SOCS`, `1P_JAR`. These cookies are tied to your device, your IP, your browser instance, and your behavior.

When NeoBrowser imported those cookies into a headless Chrome with a different user-data-dir, Google saw:

- Same session token, different browser fingerprint.
- Same `AEC` cookie, different device characteristics.
- Two active sessions from what looked like the same account but with inconsistent signals.

So Google did the safe thing: it revoked the session. Both in the automated browser *and* in the user's real browser.

## The fix

The fix was to stop importing everything and start filtering aggressively.

We now maintain three categories:

1. **Identity cookies** — tokens that prove "you are you" to a specific service. `GMAIL_AT`, `OSID`, `li_at` (LinkedIn), `auth_token` (X), etc. These are **excluded by default** from real-profile imports.

2. **Fingerprint cookies** — cookies that help the platform recognize your device. `AEC`, `SOCS`, `1P_JAR`, `DV`, `OTZ`. These are also **excluded by default** because importing them into a different browser creates the inconsistency that triggers revocation.

3. **Functional cookies** — CSRF tokens, preferences, feature flags. These are safe to import because they don't prove identity.

The rule is: **opt-in per domain, exclude identity and fingerprint cookies by default**.

```rust
// Simplified from rust/src/cookies/exclude.rs
const HIGH_RISK_IDENTITY: &[&str] = &[
    "GMAIL_AT", "OSID", "GAUSR", "ACCOUNT_CHOOSER",
    "li_at", "JSESSIONID", // LinkedIn
    "auth_token", "ct0",   // X
];

const FINGERPRINT: &[&str] = &[
    "AEC", "SOCS", "CONSENT", "1P_JAR", "DV", "OTZ",
];
```

## The lesson

In a browser agent, **"more real session" is not always "more stealth"**.

Platforms don't just check "does this browser have the right cookies?" They check "does this browser *look like* the browser that owns these cookies?" If you import real cookies into a headless browser with a sterile fingerprint, you create a contradiction. And contradictions get flagged.

The most undetectable automation is the one that *is* the user's real browser. The second most undetectable is the one that knows what *not* to copy.

## The code

NeoBrowser is open source, MIT licensed. The cookie filtering logic is in `rust/src/cookies/exclude.rs`. The real-profile import is opt-in via `NEOBROWSER_REAL_PROFILE`.

Repo: https://github.com/pitiflautico/neobrowser

---

*If you've fought session hygiene in browser automation, I'd love to hear your war stories in the comments.*
