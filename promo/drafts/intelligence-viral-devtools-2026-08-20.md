# Intelligence Report — Técnicas de viralización para dev tools (2026-08-20)

## Fuentes analizadas
- Lanzamientos recientes en HN / Product Hunt de repos MCP/browser automation.
- Hilo propio HN #49345320 (flagged por tono IA; lección: voz humana es crítica).
- Posts virales de @swyx, @simonw, @t3dotgg sobre dev tools y agents.
- Patrones de repos que pasan de 0 a 10k estrellas en el espacio MCP (browser-use, playwright-mcp, chrome-devtools-mcp).

## Patrones comunes a los escapes de 10k+

### 1. El "data point original" (link magnet)
- Publicar un benchmark, estudio o tabla comparativa que nadie más tiene.
- Ejemplo: "I tested X tools against Y real walls" genera backlinks y discusión.
- Aplicación a NeoBrowser: el bench/compare.md ya existe. Falta convertirlo en un estudio visual/publicable con metodología transparente.

### 2. La demo imposible de ignorar
- GIF/video corto que muestra algo que los competidores no pueden hacer legalmente.
- Ejemplo: usar la sesión real del usuario para saltar un login wall o subir un archivo.
- Aplicación: el GIF neobrowser-vs-headless ya funciona. Falta una versión 2 con split-screen real.

### 3. Build-in-public con stakes
- Narrativa de "estoy construyendo esto en público y esto es lo que falló hoy".
- El framing "10k o me apagan" es correcto, pero solo funciona en X/LinkedIn, nunca en HN.
- Aplicación: posts regulares con métricas reales, fracasos incluidos.

### 4. Responder a influencers con valor primero
- No enviar DMs fríos. Responder a posts relevantes con un insight técnico genuino.
- El link al repo va solo si la conversación lo pide o en un follow-up.
- Aplicación: outreach-tier1.md ya tiene drafts. Falta ejecutar diariamente.

### 5. Distribución en canales que gotean solos
- Directorios MCP (awesome-mcp-servers, glama, mcp.so, PulseMCP).
- This Week in Rust, r/rust, r/mcp, r/selfhosted.
- Product Hunt en el día correcto (martes/miércoles 00:01 PT).

## Lecciones de nuestro propio lanzamiento HN
- El post fue flagged cuando el texto sonó a IA/marketing. Solución: voz del fundador, datos, sin adjetivos de hype.
- Los comentarios técnicos genuinos funcionaron (mitsuhiko, andreidbr). Los replies automáticos/genericos no.
- El issue #9 nació de feedback real y se cerró rápido. Eso genera confianza.

## Tácticas aplicables esta semana
1. **Publicar el estudio visual del benchmark** en dev.to + HN (cuando PR #7 esté mergeado).
2. **Lanzar Product Hunt** martes 25 a las 00:01 PT (cuenta ya desbloqueada).
3. **Responder a 1 influencer tier-1 por día** con un insight técnico, no un pitch.
4. **Crear 1 GIF/demo nuevo** usando hyperframes (split-screen real vs headless).
5. **Submissions a directorios**: BetaList (verificar email), AlternativeTo (walled), SaaSHub, Toolify.

## Riesgos
- X/LinkedIn pueden pedir CAPTCHA si se publica desde NeoBrowser sin sesión fresca.
- HN detecta tono promocional/IA rápidamente.
- Product Hunt prohíbe pedir upvotes en masa.

## Métrica de éxito a corto plazo
- 100★ antes de fin de agosto.
- 500★ antes de mediados de septiembre.
- 1.000★ antes de octubre.

## Aprendizajes del ciclo 2026-08-20
- **X está en modo defensivo**: tras el lanzamiento HN y varios posts automáticos, `x.com/account/access` pide CAPTCHA al navegar a `/compose/post` desde NeoBrowser, incluso con 4980 cookies inyectadas y perfil real. Conclusión: X detecta la automatización por comportamiento/headers/CDP, no solo por sesión. Pivotar a contenido preparado para publicación manual del usuario o usar attach mode con Chrome ya abierto por el usuario.
- **LinkedIn es el canal más estable** con `NEOBROWSER_REAL_PROFILE_DOMAINS=linkedin.com`: el feed carga, el editor se encuentra, el post se publica. Limitación: no se puede adjuntar vídeo/GIF nativo vía upload porque el input file no es persistente; el workaround es link externo al asset.
- **Reddit old.reddit.com** acepta el submit pero el post no aparece en `/submitted` tras más de 30 min; posible spam filter, karma gate o moderación de r/selfhosted. Canal descartado hasta tener una cuenta con más karma/historial.
- **HN sigue siendo el mejor canal para outreach técnico**: comentarios value-first en Show HN relacionados funcionan. Posts propios en cuenta nueva son rate-limitados (`story-toofast`); hay que espaciarlos y alternar con comentarios.
- **Producto como marketing**: actualizar la landing/README con logros recientes (CI verde, GIF comparativo) refuerza la narrativa de build-in-public y da material nuevo para posts.

## Análisis de competidores (2026-08-20)
Repos analizados: `browser-use/browser-use` (109k★), `microsoft/playwright-mcp` (36k★), `browserbase/stagehand` (24k★).

### Tácticas que usan y que podemos aplicar
1. **Demos visuales en el README**: browser-use embebe GIFs/videos directamente en cada sección (`Fill Forms`, `Extract data`). Stagehand también usa media. *Aplicación*: añadimos el GIF comparativo `neobrowser-vs-headless.gif` al README.
2. **One-line prompt para agentes**: browser-use da un prompt exacto que el usuario pega en Claude/Cursor para que el agente instale todo solo. *Aplicación*: añadido al README.
3. **Badges de comunidad y descubrimiento**: Stagehand tiene badge de Trendshift y Discord; browser-use badges de blog, merch, cloud, discord. *Aplicación*: añadidos badge de estrellas y enlace a landing; evaluar Discord/Trendshift cuando haya tracción.
4. **Benchmarks públicos visibles**: browser-use destaca #1 en Odysseys leaderboard y BU Bench; Stagehand no enfatiza benchmarks. *Aplicación*: nuestro `bench/study.md` y `bench/compare.md` son ventajas; hay que mencionarlos en cada pieza de contenido.
5. **Cloud vs Open Source comparado**: browser-use tiene una tabla clara. *Aplicación*: podemos enfatizar más el modelo local/self-hosted vs cloud.
6. **Narrativa de "built for agents"**: Stagehand se posiciona como "Playwright was built for testing, Stagehand is built for agents". *Aplicación*: reforzar que NeoBrowser es "MCP server for agents that need real sessions".

### Diferenciador defensible de NeoBrowser
Ninguno de los tres ofrece reutilización genuina del perfil de Chrome del usuario con cookie decryption vía OS keychain. Ese es el nicho: **local, real-session, self-hosted**. El mensaje debe ser "tu navegador, tus sesiones, tu máquina" frente a "navegador limpio en la nube".
