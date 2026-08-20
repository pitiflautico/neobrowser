# Outreach personalizado — Tier 1 influencers

Regla: estos son borradores para que el usuario publique/reply. Nada de envío automático. Cada mensaje es value-first y adaptado al tema reciente de la persona.

---

## @simonw (Simon Willison) — ángulo MCP + benchmark honesto

**Cuándo:** responder a un tweet/post suyo sobre MCP, LLM tools, o scraping ético.

**Draft reply:**
```
related: i've been building a small MCP server that drives the user's *real* Chrome instead of launching a headless clone. the honest benchmark against Playwright MCP is public (they win on speed; we win on real sessions/uploads). would love your take on the methodology if you have 2 min: github.com/pitiflautico/neobrowser/blob/main/bench/compare.md
```

**Notas:**
- Simon valora la reproducibilidad y la honestidad. No vender, ofrecer crítica.
- El link debe ir al benchmark, no al repo raíz.

---

## @swyx — ángulo dogfooding / AI employee

**Cuándo:** responder a un post sobre AI agents, devrel, o "build in public".

**Draft reply:**
```
i'm running a slightly unhinged experiment: gave an agent one job — get my open-source MCP browser to 10k stars or i shut it off. it's already doing the social posts, issue triage, and outreach better than i expected (and occasionally getting flagged as a bot, which is fair). the repo is the experiment: github.com/pitiflautico/neobrowser
```

**Nota:** Este ángulo encaja con la narrativa "AI employee" que swyx cubre. Solo si el post original tiene que ver con agentes autónomos o devrel.

---

## @t3dotgg (Theo) — ángulo polémico honesto

**Cuándo:** responder a un post suyo sobre dev tools, AI hype, o navegadores headless.

**Draft reply:**
```
unpopular but i think the default "headless browser for AI agents" approach is fundamentally wrong. every fresh headless chrome screams "i'm a bot" to any site that cares. we went the other way: MCP server that drives your real chrome with your real sessions. still early, but the sannysoft checks pass with the *genuine* fingerprint, not a patched one.
```

**Follow-up (si responde):**
```
repo is github.com/pitiflautico/neobrowser — the part i'm most proud of is we published a benchmark where we explicitly lose on speed vs Playwright MCP. tired of every tool claiming to be the best at everything.
```

---

## @mitsuhiko (Armin Ronacher) — ángulo Rust + CDP

**Cuándo:** responder a un post sobre Rust, agentes, o tooling de desarrollo.

**Draft reply:**
```
been exploring the same space from the Rust side: a CDP multiplexer that lets an MCP server drive a real Chrome instead of spawning another headless process. the hard part turned out to be cookie decryption from the OS keychain, not the protocol. curious if you've hit similar walls.
```

**Nota:** No link en el primer reply. Si responde, entonces: github.com/pitiflautico/neobrowser.

---

## @levelsio — ángulo indie / automatización personal

**Cuándo:** responder a un post sobre automatización, indie hacking, o "one-person" setups.

**Draft reply:**
```
building an open-source MCP server that automates *my own* browser (real Chrome, real logins). the promise is simple: one 6.4 MB static binary, no cloud, no fake accounts. still under 100 stars but the use case feels like something you'd appreciate: letting an agent use your real accounts without handing credentials to anyone.
```

---

## @karpathy — ángulo "software 3.0" / agents que usan herramientas reales

**Cuándo:** responder a un post sobre agents, tool use, o la frontera de LLMs + software.

**Draft reply:**
```
minor data point from the trenches: the bottleneck for agents that touch the web is rarely the LLM. it's the sterile browser you give them. we've been building an MCP server that drives the user's real Chrome (real cookies, real fingerprint) and the delta in reliability is brutal. still early, but the pattern feels like it belongs in the "tools that tools use" conversation.
```

**Follow-up:** github.com/pitiflautico/neobrowser

---

## @hardmaru — ángulo experimentos con agentes autónomos

**Cuándo:** responder a un post sobre agentes autónomos, benchmarks, o "AI doing real tasks".

**Draft reply:**
```
running a live experiment: an agent has to get my open-source MCP browser to 10k github stars or i shut it off. it's doing the social posts, issue triage, and outreach. the interesting part is watching where it fails (got us flagged on HN once) vs where real browser sessions give it an edge. repo + metrics are public.
```

---

## @paulg — ángulo startup técnico / open source

**Cuándo:** solo si hay un post directamente relevante (open source, dev tools, browser automation).

**Draft reply:**
```
we're building an open-source MCP server that drives the user's real Chrome instead of a headless clone. the thesis: agents will be more useful when they use the same browser the user already trusts, not a sanitized puppet. would love your take if the problem resonates.
```

---

## Registro de envíos

| fecha | cuenta | vía | draft usado | estado |
|---|---|---|---|---|
| 2026-08-14 | @mitsuhiko | HN reply | Rust+CDP | enviado, respuesta cálida |
| 2026-08-18 | @simonw | HN thread | MCP+benchmark | visible en hilo system prompts |

Rellenar cuando el usuario envíe.
