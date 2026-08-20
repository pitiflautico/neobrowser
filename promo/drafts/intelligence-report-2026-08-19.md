# Inteligencia de competencia — browser MCP / agentes de navegador

**Fecha:** 2026-08-19  
**Fuentes:** GitHub, HN/Reddit/busqueda web, repos públicos de competidores.

## Posición de los competidores principales

| Proyecto | Estrellas | Backing | Hook principal | Qué no hace |
|---|---|---|---|---|
| **browser-use** | 80K+ | Comunidad / startup | Agente autónomo con LLM loop + visión | No usa tu Chrome real; no reutiliza sesiones logueadas |
| **Playwright MCP** | 33K+ | Microsoft | Oficial, accessibility tree, 70+ tools, integrado en Claude/Cursor/VS Code/Copilot | No reutiliza sesiones reales; lanzamiento headless por defecto |
| **chrome-devtools-mcp** | 28K+ | Google | Perfilado, Lighthouse, network tracing | No orientado a automatización de agentes; no sesiones reales |
| **cdp-browser-mcp** | menor | Indie | "4.6x fewer tokens than Playwright MCP" | Requiere Chrome ya lanzado con CDP; no sesiones reales |

## Cómo crecieron (patrones observables)

1. **Backing institucional acelera todo.** Microsoft y Google llegaron tarde al MCP pero con estrella: sus repos crecen por integraciones oficiales en Claude Code, Cursor, VS Code y GitHub Copilot. Esto no lo podemos replicar.

2. **Benchmarks originales = link magnet.** cdp-browser-mcp ganó atención con una tabla de tokens medidos con tiktoken en 8 páginas reales. Es reproducible, discutible y citada. HN y dev.to aman esto.

3. **"Best MCP servers" lists.** Aparecer en roundups (Sublime Coding, Kiprio, ModelPiper, Skillselion) genera tráfico constante. La mayoría indexan desde el MCP Registry oficial.

4. **Storytelling técnico.** Playwright MCP vende la narrativa "test like a user, build for an agent" — conectar testing accesible con agentes de IA. browser-use vende el agente autónomo.

5. **Integraciones 1-click.** `claude mcp add playwright -- npx @playwright/mcp@latest`. Cuanto más fácil sea instalar, más se comparte.

## Oportunidades de diferenciación para NeoBrowser

Ningún competidor ofrece estas 3 cosas juntas:

- **Usar el Chrome real del usuario** (no un headless/chromium empaquetado).
- **Reutilizar sesiones ya logueadas** de forma opt-in y segura.
- **Fingerprint genuino** (no spoofeado) verificado en CI.

Esto es el foso. Nuestro mensaje debe ser: *"No emulamos un navegador. Usamos el tuyo."*

## Tácticas aplicables inmediatamente

### 1. Benchmark original (alto impacto, 1-2 días)
Replicar el estilo de cdp-browser-mcp pero con nuestro ángulo:
- Medir **éxito en tareas reales con login** (donde NeoBrowser gana).
- Medir **detección por bot.sannysoft/Cloudflare** (todos igual, honestidad).
- Medir **latencia y tokens** en páginas representativas.
- Publicar como `bench/vs-browser-use-playwright.md` + post HN/dev.to.

### 2. MCP Registry oficial (bloqueado por OAuth usuario)
Canal más importante. Alimenta todos los agregadores. Necesita 1 minuto de OAuth.

### 3. Integraciones 1-click
Ya tenemos badges de VS Code y Cursor en README. Añadir:
- Instalación para Claude Code: `claude mcp add neobrowser -- neobrowser`
- Documentar config para Cursor/Claude Desktop/Windsurf.

### 4. Roundups y listas
Identificar 10 blogs/directorios que publiquen "best MCP servers" y proponer entrada con datos (no hype). Ejemplos encontrados: Sublime Coding, Kiprio, ModelPiper, Skillselion, MCPNest.

### 5. Narrativa "10k o me apagan"
Ningún competidor puede copiar esta historia. Usarla en X/LinkedIn/dev.to. Mostrar métricas reales, fracasos incluidos.

## Qué NO funciona (aprendido de competidores)

- Claims de "evade cualquier bot" — todos los benchmarks serios muestran que Cloudflare/reCAPTCHA paran a todos.
- Posicionarse solo como "más rápido" — Playwright MCP y browser-use ganarán en velocidad pura.
- Vender visión/genericidad — browser-use ya es el agente autónomo; no competimos ahí.

## Recomendación prioritaria

1. Desbloquear MCP Registry oficial (OAuth).
2. Publicar benchmark original con énfasis en "tareas que solo un navegador real puede hacer".
3. Lanzar Product Hunt el martes 25 coordinado con HN + Reddit + X/LinkedIn en 48h.
4. Contactar a 2 influencers por semana con mensaje value-first.
