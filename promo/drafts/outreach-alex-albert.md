# Outreach draft: Alex Albert (Anthropic developer relations)

**Handle:** @alexalbert__ on X/Twitter  
**Role:** Developer Relations at Anthropic. Highly visible in the MCP ecosystem.  
**Why him:** A single retweet or mention from Anthropic's DevRel would put NeoBrowser in front of the exact audience that installs MCP servers today.

---

## DM / short email

Subject: An MCP server that runs on the user's real Chrome, not a cloud browser

Hi Alex,

Quick one. I built NeoBrowser, an MCP server that drives the user's actual Google Chrome over CDP instead of launching a fresh headless browser.

The problem it solves: every browser MCP I tried hits login walls and bot checks because it starts cookie-less. NeoBrowser can reuse the user's real logged-in sessions locally, with the renderer sandbox on and identity cookies excluded by default.

It's a single Rust binary, MCP-native, no cloud or API key of its own. Would love your honest take if you have two minutes:

https://github.com/pitiflautico/neobrowser

No pressure — just trying to get it in front of people who actually ship with MCP.

Daniel

---

## Notes

- Lead with the problem, not the pitch.
- Mention MCP-native and local-first to match Anthropic's framing.
- Ask for opinion, not a retweet.
- Keep under 150 words for X DM.
