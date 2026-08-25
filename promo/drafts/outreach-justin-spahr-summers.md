# Outreach draft: Justin Spahr-Summers (MCP co-creator)

**Handle:** @jspahrsummers on X/Twitter  
**Role:** Co-creator of Model Context Protocol (Anthropic).  
**Why him:** His endorsement or technical feedback would carry enormous weight in the MCP ecosystem. A single thoughtful reply would validate NeoBrowser's approach.

---

## DM / short email

Subject: An MCP server that drives the user's real Chrome — any security concerns you'd add?

Hi Justin,

I've been building NeoBrowser, an MCP server that drives the user's real Google Chrome over CDP instead of launching a fresh headless browser.

The motivation is simple: most browser MCPs start cookie-less, so the model hits login walls and bot checks immediately. NeoBrowser lets the agent reuse the user's existing sessions locally, with the renderer sandbox on and identity cookies excluded by default. We also log every tool call with secret redaction and support domain allowlists.

I'm asking because you think about MCP security more than almost anyone: is there a guardrail you'd consider essential that we're missing? I'd rather hear it now than learn it the hard way.

https://github.com/pitiflautico/neobrowser

No need to reply if you're swamped — but any pointer would be huge.

Daniel

---

## Notes

- Lead with a specific, respectful ask about security (his area of expertise).
- Mention local-first and sandbox to preempt the obvious concerns.
- Keep it short; he gets a lot of inbound.
