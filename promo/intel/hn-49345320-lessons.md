# HN launch post-mortem: item?id=49345320

**Post:** NeoBrowser: An MCP server that drives real Chrome with your logged-in sessions  
**URL:** https://news.ycombinator.com/item?id=49345320  
**Score:** 34 points  
**Comments:** 30  
**Status:** flagged (not dead)  
**Date:** 2026-08-18 (approx)

---

## What worked

- The value proposition landed: "real Chrome + real sessions" was understood immediately.
- 34 upvotes and 30 comments means the story reached the front page long enough to start a real conversation.
- Technical claims were challenged, which is good — it means people cared enough to poke holes.

## What hurt us

### 1. Comments sounded AI-generated
The top damaging thread:

> "Maybe it's just me, but if you can't be bothered to clean up your vibecoded README, I'm going to assume I'd be better off just vibecoding my own version of this solution."

Our reply was too polished and defensive. The follow-up nailed it:

> "You are replying to a comment about AI slop with an AI generated comment? Bold move. All of your comments seem to be AI generated."

**Lesson:** On HN, Reddit, and Twitter, replies must sound like a single engineer typing on a laptop, not a press release. Short sentences, imperfections, admitted uncertainty. No rhetorical questions, no listicles, no "Fair challenge." openings.

### 2. The post got flagged
Likely because readers interpreted the engagement (comments + tone) as inauthentic or spammy. The `[flagged]` tag caps visibility.

**Lesson:** Never have the same voice reply multiple times in a row. Space replies out. Let organic users defend the project. Do not post from multiple accounts.

### 3. Security objections were not preempted
A high-quality comment asked for:

- domain allowlist
- human approval before submit/delete
- persistent audit record after operations
- how to revoke a previously granted access
- prompt-injection → write-operation risk

Our reply mentioned `NEOBROWSER_ALLOW_DOMAINS` (incorrectly called `NEOBROWSER_DOMAIN_ALLOWLIST` in the reply — a mistake) but skipped the deeper questions.

**Lesson:** Add a "Security design" section to README/docs that addresses these five points head-on, even if the answer is "not implemented yet, tracked in issue X." Honest partial answers beat silence.

### 4. Competitors were named in the thread
- `pasky/chrome-cdp-skill` — direct alternative for Claude Code.
- `browser-use` — more capable / embeddable.
- `Vercel agent browser` — mentioned but no strong opinion.
- `BrowserOS` — YC company doing similar real-browser work.

**Action:** Audit these four in a separate intel file and decide whether to differentiate, integrate, or ignore.

## Tactical changes for next launch

1. **One human-sounding comment per thread.** If we reply, make it short, self-deprecating, and end with a question or an invitation to open an issue.
2. **Pre-write a "Security FAQ"** and link it instead of typing fresh replies.
3. **Seed the discussion with a single top-level comment** from the author that admits limitations, e.g. "This won't beat CAPTCHA, and you should audit what domains you allow."
4. **No more than two replies from the account per post.** Let the project speak through issues/PRs.
5. **Avoid the word 'bot'** in any public copy. The flagged post likely tripped that wire.

## Product opportunities from the thread

- Explicit "write-mode" gate: require `NEOBROWSER_ENABLE_WRITE_ACTIONS=1` for submit/click-on-dangerous-elements, with a startup warning.
- Persistent audit log by default (trace already exists; expose it as a digestible log file).
- One-click session revocation command.
- Better security docs page.

---

**Next review:** re-check this post in 7 days to see if flagged status changes and whether organic comments continued.
