# Inteligencia: tendencia MCP + browser automation (2026-08-20)

## Observación
En r/mcp, el hilo "Is Puppeteer MCP useful for scraping, or mainly browser automation?" generó comentarios value-first incluso con pocos upvotes. La pregunta subyacente que resuena: **¿para qué sirve realmente un navegador headless cuando los agents necesitan sesiones reales?**

## Patrón
Los posts técnicos que contrastan "headless rápido pero sin sesión" vs "real Chrome lento pero con sesión" obtienen engagement porque:
1. Tocan un dolor real (login walls quemando contexto).
2. No son puros pitches; ofrecen una regla de decisión.
3. Invitan a discusión en lugar de cerrarla.

## Táctica aplicable a NeoBrowser
- **No vender "más rápido"**; vender "completa tareas que headless no puede empezar".
- **Usar la regla de decisión** como gancho en futuros posts:
  - Stateless/public pages → headless.
  - Real login/upload/adversarial → real Chrome.
- **Responder en hilos de comparación** (Puppeteer MCP vs Playwright MCP vs browser-use) con el ángulo de "trust", no con un link directo.

## Canales donde este ángulo funciona
- r/mcp: posts de comparación y comentarios técnicos.
- Hacker News: hilos sobre AI agents o browser automation.
- dev.to / Medium: artículos técnicos con benchmarks honestos.
- LinkedIn/X: carruseles/GIFs con la regla de decisión visual.

## Próximo experimento
Crear un post/carrusel titulado "Headless vs Real Chrome: when to use each for AI agents" y publicarlo en r/mcp, LinkedIn y dev.to con el GIF recién generado.
