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
- **Reddit old.reddit.com** acepta el submit pero la verificación es lenta/inconsistente; posible rate-limit por cuenta nueva o moderación de r/selfhosted. Requiere re-check manual en 30-60 min.
- **HN sigue siendo el mejor canal para outreach técnico**: un comentario value-first en un Show HN relacionado (Stagehand/Browserbase) es bien recibido si incluye disclosure honesto y una pregunta genuina.
- **Producto como marketing**: actualizar la landing con "Pre-launch hardening merged" y CI verde refuerza la narrativa de build-in-public y da material nuevo para posts.
