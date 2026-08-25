**To:** @karpathy (Andrej Karpathy)  
**Angle:** agents using real tools / browsers  
**Channel:** X / LinkedIn reply or short DM

---

Hey Andrej — your recent posts on agents using real desktop tools resonated with what we're seeing in NeoBrowser.

We took a similar "real over simulated" path for browser automation: instead of a headless environment the agent tries to spoof, we drive the user's actual Chrome via CDP. The upside is the agent gets real sessions, real cookies, real anti-fraud trust scores. The downside is the obvious one — it's a live browser, so you need hard guards around it.

We ended up structuring every mutating action as observe → act → verify, with origin-scoped credentials and an explicit audit trace. It feels closer to a "real tool" interface than a sandboxed play environment.

Open source if you're curious: https://github.com/pitiflautico/neobrowser

No pitch, just thought it was a relevant data point for the "agents should use real tools" thesis.
