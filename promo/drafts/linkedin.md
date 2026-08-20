# Borrador LinkedIn (usuario publica)

**Tono**: más sobrio que Twitter, orientado a "lo que aprendí construyéndolo". Adjuntar el demo.gif funciona bien en LinkedIn.

---

```
Los agentes de IA llevan meses fingiendo usar la web: navegadores headless de fábrica, sin cookies, detectados al instante por cualquier bot-check serio.

He publicado NeoBrowser, un MCP server open source que toma otro camino: controla tu Chrome real (vía Chrome DevTools Protocol) y puede reutilizar tus sesiones ya logueadas — de forma opt-in, descifrando las cookies a través del keychain del sistema operativo. El agente llega autenticado porque ES tu navegador.

Tres decisiones de diseño que me llevaron más trabajo del esperado:

1. Stealth genuino, no parcheado. Nada de falsear WebGL o el User-Agent: la consistencia real es lo que pasa los fingerprint checks. Verificado contra bot.sannysoft en CI en cada push.

2. Honestidad ante los muros. Ninguna herramienta puede prometer superar reCAPTCHA o Turnstile. NeoBrowser detecta el tipo de muro (captcha, consent, rate-limit, login) y devuelve al modelo una estrategia en vez de martillear la página.

3. Benchmarks honestos. Comparé contra Playwright MCP con un harness neutro: Playwright es más rápido; NeoBrowser hace cosas que no puede (persistencia de sesión, uploads). Los números y la metodología están en el repo para quien quiera auditarlos.

67 herramientas, un único binario estático de 6.4 MB en Rust, sin runtime. Multi-tab, formularios, subida/descarga de archivos, búsqueda multi-fuente, grabación y replay de tareas.

Repo (MIT): https://github.com/pitiflautico/neobrowser

#opensource #rust #ai #mcp #automation
```

## Notas
- LinkedIn premia posts en primera persona con aprendizaje; evita que parezca anuncio.
- Si tienes red en español e inglés, publica primero en el idioma de tu audiencia mayoritaria; la otra versión (traducir) puede ir una semana después.
