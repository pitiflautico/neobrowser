---
title: "Why I stopped injecting cookies and started bridging to the real browser"
published: false
description: "The hardest lesson in AI browser automation: a cloned session is a suspicious session."
tags: rust, ai, browserautomation, mcp, opensource
---

A few days ago I got the kind of bug report every maintainer dreads: **NeoBrowser was logging users out of their real accounts**.

Not because we wrote to their Chrome profile. We never do. We were reading cookies from the user's real profile and injecting them into a fresh headless Chrome so the agent could act authenticated. It worked for some sites. For others, Google/LinkedIn/X saw the same session cookie appear from a different fingerprint and did the safe thing: kill the session everywhere.

## The trap

Browser automation has two classic answers to "how do I act as the user?":

1. **Launch a sterile browser** and log in programmatically. Works until 2FA, CAPTCHA, or SSO show up.
2. **Copy the user's cookies** into the sterile browser. Works until the platform notices the duplicate fingerprint.

Both fail the moment the site cares about trust signals. And the sites that matter — GitHub, LinkedIn, Google Workspace, internal dashboards — all care.

## What we changed

First, the immediate fix: `NEOBROWSER_REAL_PROFILE` no longer injects cookies by default. You must now opt in per domain with `NEOBROWSER_REAL_PROFILE_DOMAINS=x.com,twitter.com`. This stops the accidental blast of thousands of cookies into the wrong fingerprint.

Second, the real fix: we stopped trying to clone the browser at all.

NeoBrowser Bridge is a tiny Chrome extension. Instead of copying cookies, the user opens their real Chrome, clicks **Share** on the tab they want automated, and NeoBrowser sends CDP commands directly to that tab. Same browser. Same session. Same fingerprint. The platform sees nothing unusual because nothing unusual is happening.

## Why an extension, not just `--remote-debugging-port`?

`--remote-debugging-port` on your everyday browser exposes every tab to anything that can reach the port. The bridge is per-tab, per-consent, and revocable. Chrome even shows its own "debugging" banner while a tab is shared, so it can never be silent.

## The lesson

In browser automation, "more session data" is not "more real". The web's trust model is built on continuity: same device, same browser, same history, same fingerprint. The closer your automation gets to that continuity, the fewer walls it hits.

We're at 88/10,000 GitHub stars. The repo is open source, MIT, and the bridge code is in `extension/`.

If you've fought this problem before — cloned cookies, dead sessions, bot detection arms races — I'd love to hear how you solved it.
