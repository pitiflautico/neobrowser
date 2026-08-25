# Outreach draft: Petr Baudis (pasky / chrome-cdp-skill)

**Contact:** pasky@ucw.cz  
**Project:** https://github.com/pasky/chrome-cdp-skill (3,233★)  
**Why him:** chrome-cdp-skill is the closest existing project to NeoBrowser's attach-mode workflow. Petr solved the "connect to the user's live Chrome" problem elegantly, and his HN mention brought us real traffic. A collaboration or cross-link would help both communities.

---

## Subject

chrome-cdp-skill and NeoBrowser — similar problem, different defaults

## Body

Hi Petr,

Someone mentioned chrome-cdp-skill in the HN thread for my project NeoBrowser yesterday, and it immediately clicked: we're attacking the same failure mode from opposite sides.

You built the cleanest attach-mode experience I've seen — connect to the user's already-running Chrome, no extra browser, no session cloning. NeoBrowser started from the opposite pain: people wanted to run an agent headlessly but still reuse real sessions, so we added encrypted cookie import from the real profile. After getting bitten by the logout-every-account bug, we just shipped an opt-in per-domain filter (`NEOBROWSER_REAL_PROFILE_DOMAINS`) so the import only touches the sites you name.

I think both approaches deserve to be listed together: attach when you can, import-only-what-you-need when you can't. Would you be open to a small cross-link between our READMEs, or just leaving this as a heads-up that NeoBrowser exists? No pressure either way — I genuinely admire how minimal and reliable chrome-cdp-skill looks.

Either way, thanks for open-sourcing your take on the problem.

Daniel
https://github.com/pitiflautico/neobrowser

---

## Notes

- Tone: humble, no hard sell, acknowledges his work first.
- No spam follow-up unless he replies.
- Send only when we have a stable domain/landing, ideally after Product Hunt launch.
