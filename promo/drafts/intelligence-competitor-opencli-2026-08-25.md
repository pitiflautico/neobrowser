# Inteligencia de competencia — OpenCLI (jackwener/OpenCLI)

## Ficha

- **Repo:** https://github.com/jackwener/OpenCLI
- **Estrellas:** 28,565★ (muy superior a NeoBrowser)
- **Lenguaje:** TypeScript / Node.js
- **Claim principal:** "Convert any website into a CLI & run Browser Use on your logged-in Chrome."
- **Distribución:** npm (`@jackwener/opencli`), desktop app, Chrome extension.

## Qué hace bien

1. **Enfoque de adapters:** en lugar de dar herramientas genéricas de browse/click, provee comandos predefinidos para sitios populares (`opencli hackernews top`, `opencli bilibili hot`).
2. **Ecosistema de skills:** `opencli-browser`, `opencli-adapter-author`, `opencli-autofix`, etc. Los agentes pueden descubrir y usar adapters.
3. **Desktop app + extensión:** baja fricción para usuarios no técnicos.
4. **CLI hub:** también funciona como wrapper de herramientas locales (`gh`, `docker`, etc.).
5. **Multi-idioma:** README en inglés y chino.
6. **Verificación:** `opencli browser verify` para validar adapters.

## Diferenciadores que aún tenemos

- **Sin extensión obligatoria:** NeoBrowser puede lanzar o attachar Chrome directamente vía CDP.
- **Binario único Rust:** ~6 MB, sin Node/npm.
- **Seguridad estructurada:** origin scoping, verified actions, approval gates, vault.
- **Benchmark honesto:** comparamos con Playwright MCP públicamente.

## Tácticas aplicables

1. **Site-specific adapters:** crear adapters reutilizables para sitios populares (GitHub, HN, Reddit, arXiv) como herramientas de alto nivel encima de las primitivas.
2. **Skill packaging:** publicar "skills" o prompts de sistema para Claude Code/Cursor que enseñen a usar NeoBrowser eficazmente.
3. **Extensión bridge más visible:** ya tenemos bridge, pero no lo destacamos en el README hero.
4. **Desktop app:** anunciarlo en roadmap para reducir fricción.
5. **Localización:** traducir README al chino; el mercado asiático está muy activo en MCP.
6. **`neobrowser doctor` como onboarding:** OpenCLI lo usa como primer paso; nosotros también lo tenemos, pero podríamos hacerlo más prominente.

## Amenaza real

OpenCLI tiene 300× más estrellas y un ecosistema más rico. No compite directamente (él es CLI-first + adapters, nosotros MCP server + real Chrome), pero ocupa la misma mente de "usa tu navegador logueado con AI agents".

## Recomendación

No intentar copiar todo su modelo. Doblar la apuesta en nuestros diferenciadores: binario único, seguridad, y real Chrome sin extensión. Pero sí añadir algunos adapters de alto nivel (GitHub, HN) como demos para mostrar que no solo damos primitivas.
