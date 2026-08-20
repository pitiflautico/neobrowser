# Outreach draft — Greg Kamradt (@gregkamradt)

**Contexto:** Presidente de ARC Prize Foundation, creador de contenido sobre AI agents, activo en X/Twitter (@gregkamradt) y YouTube (@dataindependent). Su audiencia es técnica y pragmática; le interesan benchmarks honestos y casos de uso reales de agents.

**Vía:** reply genuino en X a un post reciente sobre agents, benchmarks o herramientas de navegación; o comentario en un video de YouTube si encaja. No hay email público.

**Regla:** aportar valor primero; conectar con su interés en benchmarks reales y evaluación de agents.

---

Hey Greg,

Quick question sparked by your recent agent content: when you hand an agent a browser, do you care more about raw speed or about whether the site trusts the browser?

I've been running NeoBrowser, an MCP server that drives the user's real Chrome over CDP instead of a fresh headless profile. The honest trade-off so far:

- ~4s average action latency vs ~1s for Playwright MCP.
- But it completes tasks that need real sessions or file uploads, because it inherits the user's cookies/fingerprint.
- On adversarial pages from a residential IP, both get blocked equally.

I wrote up the benchmark methodology and a real demo: https://github.com/pitiflautico/neobrowser

Would love your take on what a fair evaluation for browser agents should measure. Most repos claim "passes bot detection" without defining the test, the IP, or the failure mode — I'm trying to fix that, but I may be missing dimensions that matter for ARC-style evaluation.

— Daniel / NeoBrowser

---

**Estado:** borrador listo. Envío pendiente de encontrar un post reciente adecuado para reply genuino.
