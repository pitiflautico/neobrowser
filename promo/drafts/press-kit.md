# Press kit — NeoBrowser

One-pager para compartir con influencers, newsletters, inversores o directorios.

---

## En una línea

NeoBrowser es un MCP server open source que permite a agentes de IA controlar tu Google Chrome real con tus sesiones ya logueadas.

## El problema

Los navegadores MCP actuales lanzan un Chrome headless fresco, sin cookies y con fingerprint detectable. El agente pierde la mitad del tiempo en login walls y bot checks.

## La solución

NeoBrowser conecta con tu Chrome real vía Chrome DevTools Protocol:

- **Sesiones reales**: reutiliza cookies de tu perfil de Chrome (opt-in, descifrado via keychain del SO).
- **Fingerprint genuino**: pasa bot.sannysoft con la señal real de tu máquina, no con spoofing.
- **Detección de muros**: reconoce CAPTCHA, consent, rate-limit y login gates, y devuelve una estrategia al modelo.
- **Input humano**: clicks con trayectoria eased/jittered y typing per-key con timing realista.
- **67 herramientas**: navigate, forms, upload/download, multi-tab, search, screenshots, playbooks.
- **Un único binario estático** de ~6.4 MB en Rust, MIT.

## Diferenciador clave

Ningún otro MCP browser ofrece **Chrome real + sesiones reales + fingerprint genuino**. browser-use y Playwright MCP son grandes, pero no pueden empezar desde tu sesión logueada.

## Métricas

- 88★ / 4 forks en GitHub (en crecimiento).
- 11/11 en bot.sannysoft con fingerprint genuino.
- Benchmark honesto vs Playwright MCP publicado en `bench/compare.md` y `bench/study.md`.
- 361+ tests, CI con Chrome real en cada push.

## Historia con stakes

Un agente de IA promociona el proyecto con meta de 10.000 estrellas. Progreso público en la landing.

## Links

- Repo: https://github.com/pitiflautico/neobrowser
- Landing: https://pitiflautico.github.io/neobrowser/
- Benchmark: https://github.com/pitiflautico/neobrowser/blob/main/bench/study.md
- Twitter/X: @pitiflautico

## Assets

- Logo: `docs/assets/og.png`
- GIF comparativo: `promo/assets/neobrowser-vs-headless/neobrowser-vs-headless.gif`
- Demo GIF: `docs/assets/demo.gif`

## Contacto

Daniel Perez Pinazo — pitiflautico3@gmail.com
