# Borrador Show HN — estudio de bot detection (original research)

**Cuándo**: martes o miércoles, 9–11am ET.  
**URL**: https://github.com/pitiflautico/neobrowser/blob/main/bench/study.md  
**Importante**: responder a todos los comentarios en las primeras 4-6 horas.

---

## Título

```
Show HN: I tested NeoBrowser and Playwright MCP against live bot detection — honest table
```

## Texto del post

```
I keep seeing browser-automation tools claim they "pass bot detection" or "evade CAPTCHA". Most of those claims are either synthetic (a single test site) or dishonest (they don't mention where they fail).

So I ran a reproducible head-to-head: NeoBrowser (my MCP server that drives real Chrome) vs Playwright MCP against sannysoft, creepjs, nowsecure.nl (Cloudflare), and deviceandbrowserinfo. Same machine, same IP, same harness, N=2.

The honest table:

- Sannysoft: NeoBrowser 11/11, Playwright MCP headless 10/11 (fails UA — HeadlessChrome).
- nowsecure.nl / Cloudflare: BOTH blocked in both runs. No tool bypasses real Cloudflare from a single IP.
- Latency: Playwright MCP is 3-5x faster. NeoBrowser forces frames so deferred content renders; that's the cost.
- navigator.webdriver: NeoBrowser reads undefined, Playwright headless reads false. Both are automated tells in theory; neither site flagged them here.

What I'm NOT claiming: that NeoBrowser evades real anti-bot. It doesn't. The difference is it uses your real Chrome with your real sessions, so for your own accounts it starts already trusted; and when a wall appears it detects it instead of hallucinating success.

Methodology and raw results: https://github.com/pitiflautico/neobrowser/blob/main/bench/study.md

Repo: https://github.com/pitiflautico/neobrowser
```

## Respuestas preparadas

- **"N=2 is not a study"** → De acuerdo. Es suficiente para mostrar que el harness funciona y que ambos se comportan de forma estable; no para claims estadísticos. El repo incluye el código para que cualquiera corra N=10/100.
- **"undefined vs false en webdriver"** → Exacto, undefined es un tell potencial. Lo reportamos como observado, no como ventaja.
- **"Playwright MCP no headless pasa el UA"** → Cierto. El estudio usa --headless para que sea comparable con la config por defecto de bench/compare.py. Un run headed cambiaría eso.
- **"¿Para qué sirve si Cloudflare bloquea?"** → Para automatizar TUS cuentas en sitios donde ya estás logueado. El foso es la sesión real, no eludir walls ajenos.
