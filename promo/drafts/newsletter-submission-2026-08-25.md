# Newsletter / forum submission draft

Date: 2026-08-25

## Targets considered

1. **JavaScript Weekly** — `editor@cooperpress.com` (Cooperpress family; browser-automation / MCP topic fits the JS/web audience).
2. **PyCoder’s Weekly** — https://pycoders.com/submissions (Python tooling; less of a language fit because NeoBrowser is Rust, but the project has a Python MCP client story and the audience cares about automation).
3. **DEV Community** — https://dev.to/ (self-publish a technical post; can syndicate the benchmark discussion verbatim or shorten it).

## Chosen outlet

**JavaScript Weekly** — concise email submission.

---

To: editor@cooperpress.com  
Subject: Submission: NeoBrowser — MCP server that drives your real Chrome

Hi Peter,

A quick submission for a project I think JavaScript Weekly readers will care about.

**NeoBrowser** is an open-source MCP server that lets AI agents drive a *real* Google Chrome via the Chrome DevTools Protocol. No headless spoofing arms race, no patched Chromium, no fake WebGL vendor strings — just the user’s own browser with their own sessions and a genuine fingerprint.

It matters because most agent browser-automation stacks are stuck patching headless Chromium to look human. Bot detectors correlate dozens of signals (UA, WebGL, permissions, plugin arrays, event trust), so every spoofed value is a new way to fail. NeoBrowser sidesteps that by using real Chrome, real clicks, and real input events.

I published a small, honest benchmark comparing NeoBrowser to Playwright MCP headless on public bot-detection test sites:

https://github.com/pitiflautico/neobrowser/blob/main/bench/study.md

Key takeaway: NeoBrowser passed all Sannysoft checks while Playwright headless leaked `HeadlessChrome` in the UA. It is slower (3–5× per navigate) because it forces frames for deferred content, and it does *not* claim to bypass Cloudflare — both tools were blocked there. The write-up explains exactly what the test does and does not prove.

Repo and install:

https://github.com/pitiflautico/neobrowser

```bash
curl -fsSL https://raw.githubusercontent.com/pitiflautico/neobrowser/main/install.sh | bash
neobrowser doctor
```

Thanks for considering it.

— Daniel

---

## Notes for follow-up

- If JavaScript Weekly does not pick it up, adapt the same body into a DEV post (use the longer discussion version at https://github.com/pitiflautico/neobrowser/discussions/20).
- For Hacker News / lobste.rs, shrink the title to under 80 characters and post the repo URL directly, then add context in a comment.
