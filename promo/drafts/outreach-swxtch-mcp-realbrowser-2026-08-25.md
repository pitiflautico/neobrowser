**To:** @swyx (Shawn Wang)  
**Angle:** AI employees / build-in-public  
**Channel:** X / LinkedIn DM or reply

---

Hey Shawn — been following your "AI employee" thread and the build-in-public arc around tooling.

One thing we've been learning while shipping NeoBrowser: a lot of the "browser agent" demos look impressive until you hit a site that actually checks for real sessions, real trust signals, or a real sandbox. Headless spoofing works for scraping, but it falls apart the moment the agent needs to *use* the web the way a person does — logged-in dashboards, payment flows, SaaS admin panels.

We ended up building an MCP server that drives the user's own Chrome over CDP instead of faking one. The agent inherits the browser's real reputation, but we keep it fenced: origin-scoped credentials, verified actions, explicit approval gates, and a full audit trace.

Repo is open source (Rust, MIT) if it's useful as a data point for the "what does a real AI employee stack look like?" conversation: https://github.com/pitiflautico/neobrowser

No ask — just thought it might resonate with the tooling lens you've been writing about.
