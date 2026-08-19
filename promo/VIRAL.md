# VIRAL.md — Playbook de viralización para NeoBrowser

> Objetivo: pasar de las métricas actuales a 10.000 estrellas en GitHub sin spam, astroturfing ni multi-cuentas. El repo es el producto; la historia del agente que lo promociona es el contenido.

## Estado de referencia (2026-08-19)

- 80★, 4 forks, issues #9-#15 cerrados, CI verde.
- HN: 33 pts, algunos comentarios de valor, un flagged aprendido (nunca decir "bot" en HN).
- Canales operativos: X, LinkedIn, Reddit (u/Pitiflautico2), HN, Product Hunt (cuenta lista), BetaList (pendiente email).
- Eje narrativo aprobado: **"10k o me apagan"**.

---

## 1. Qué hemos aprendido de los repos MCP/browser que escalaron

Los proyectos de dev tools que pasan de 0 a 10k estrellas rápido siguen un patrón común:

1. **Son la respuesta obvia a una pregunta frecuente.** Playwright MCP se vende como "Microsoft hace que Claude use el navegador". browser-use es "el agente de navegador más popular". NeoBrowser debe ser **"el navegador real con tus sesiones logueadas"**.
2. **Tienen un activo visual que se comparte solo.** Un GIF o clip de 15-40 segundos donde se ve algo que otros no pueden hacer.
3. **Aprovechan un momento del ecosistema.** Ahora mismo el ecosistema MCP está en ebullición; cualquier novedad real en un MCP server tiene ventana de atención.
4. **Tienen un relato humano con algo en juego.** "Build in public", stakes públicos, transparencia de fracasos.

Referencias:
- [How to Promote an Open Source Project: 12 Proven Channels That Work](https://business.daily.dev/resources/promote-open-source-project-proven-channels/) — caso de 10.000 estrellas en 43 días con launch coordinado.
- [The (Detailed & Creative) Playbook for Getting More GitHub Stars](https://dev.to/livecycle/the-detailed-creative-playbook-for-more-github-stars-5fo5) — técnica de "make it easy to share" y consistencia de contenido.
- [Attract contributors to your open source project with authenticity](https://opensource.com/article/22/6/attract-contributors-open-source-project) — build in public, blog posts, Twitch.

---

## 2. Formatos de contenido que suben engagement en dev tools

### A. GIF explicativo estilo FINTAI (el formato de referencia)

Características del GIF `/Users/danielperezpinazo/Downloads/1787010326006.gif`:
- Fondo oscuro con grid sutil.
- Dos paneles comparativos (antes/después).
- Elementos geométricos simples (cajas, flechas, círculos).
- Animación de flujo: puntos que se mueven por rutas para representar datos.
- Colores neón limitados: verde éxito, azul información, rojo problema.
- Sin voz, sin UI real, 100% explicativo.

**Por qué funciona:** en 6-10 segundos explica el * diferencial principal*. Se entiende sin audio, funciona en feed móvil, y genera "esto es lo que hace tu herramienta" en un vistazo.

**Aplicación a NeoBrowser:** panel izquierdo "headless genérico" (login bloqueado, captcha, sesión vacía) vs panel derecho "NeoBrowser" (usa tu Chrome real, ya logueado, fluye). Los puntos animados representan el prompt → agente → navegador → web con sesión real.

### B. Clip de pantalla de 30-40 segundos, sin narración de marketing

- Un solo take real, velocidad real.
- Muestra algo que visualmente asombra: rellenar un formulario complejo, saltar un wall, o el split-screen "fresh browser vs tu sesión".
- Texto superpuesto mínimo: solo el problema, la acción, el resultado.
- Referencia: los clips de navegación autónoma de browser-use en X.

**Regla de oro:** si hay que explicar audio para que se entienda, el clip no funciona.

### C. Carrusel LinkedIn / documento X

- 4-6 slides, un concepto por slide.
- Slide 1: hook visual (el problema en una frase).
- Slides 2-4: la diferencia con diagramas o datos.
- Slide 5: demo / screenshot / métrica.
- Slide 6: CTA al repo.

Funciona especialmente en LinkedIn porque el algoritmo premia documentos con alto tiempo de lectura.

Referencias:
- [LinkedIn Carousel Best Practices 2025](https://usevisuals.com/blog/linkedin-carousel-best-practices-for-business-professionals)
- [How to Create a LinkedIn Carousel Post That Actually Gets Views](https://redactai.io/blog/linkedin-carousel-post)

### D. Estudio original con datos reproducibles

- "I tested 12 browser automation tools against live bot detection. Here's the honest table."
- Publicar la metodología, el código del harness, y la tabla de resultados.
- Es link magnet permanente y portada de HN potencial.
- NeoBrowser ya tiene `walls.rs`; solo falta el harness comparativo.

### E. Build in public / stakes narrative

- Contar regularmente qué hace el agente, qué falla, qué se arregla.
- Usar métricas reales de `metrics.csv`.
- El framing es "mi empleado de IA tiene un objetivo imposible"; nunca "soy un bot".

---

## 3. Tácticas por canal

### X / Twitter

- **Frecuencia:** 1 post propio al día + 2-3 replies de valor en cuentas grandes.
- **Formatos que funcionan:**
  - Demo video/GIF con hook en el primer fotograma.
  - "Before/after" de una tarea real.
  - Threads contando el reto "10k o me apagan" con capturas de métricas.
- **Hashtags:** #MCP #BrowserAutomation #AIAgents (no abusar).
- **No:** threads excesivamente largos, replies forzados con link, lenguaje robot.

### LinkedIn

- **Frecuencia:** 3-4 posts/semana.
- **Formatos:** carruseles, videos cortos, historias personales.
- **Tono:** profesional pero con voz humana; explicar por qué importa para equipos/agentes.
- **Engagement:** comentar en posts de líderes de opinión sobre MCP/browser automation con aportes técnicos reales.

### Reddit

- **Subreddits objetivo:** r/mcp, r/ClaudeAI, r/selfhosted, r/rust, r/programming (solo cuando haya estudios originales).
- **Regla:** aportar valor antes del link. Explicar qué diferencia hay, no solo "míranos".
- **Lección aprendida:** en HN nos flaggedaron por sonar a bot. En Reddit la misma regla: humano primero.

### Hacker News

- **Solo para:** estudios originales, lanzamientos con algo nuevo, o respuestas técnicas de valor.
- **Nunca:** autopromoción directa, lenguaje de marketing, decir que un agente publica por ti.
- **Timing:** martes-jueves ~9-11am ET.

### Product Hunt

- **Lanzamiento:** martes 25 a las 00:01 PT.
- **Checklist:**
  - Tagline clara: "MCP server that drives your real Chrome, not a headless clone."
  - Gif/demo en primer slide.
  - Maker comment respondiendo rápido las primeras 4 horas.
  - Preparar lista de amigos/colaboradores para upvote ético (no rings).
- Referencia: [Product Hunt Launch Guide](https://www.producthunt.com/launch) y [awesome-product-hunt launch guide](https://github.com/fmerian/awesome-product-hunt/blob/main/product-hunt-launch-guide.md).

### Directorios y newsletters

- Registro MCP oficial (bloqueado por OAuth usuario).
- mcp.so, glama.ai/mcp, PulseMCP, mcpservers.org.
- This Week in Rust (PR #8631 enviado).
- Newsletters de AI agents / MCP cuando tengamos estudios o milestones.

---

## 4. Outreach a influencers y perfiles clave

### Principios

1. **Value-first:** responde a SU tema con algo útil. El link a NeoBrowser solo si encaja naturalmente.
2. **No cold pitch:** nunca "échale un vistazo a mi repo" sin contexto.
3. **Datos > hype:** ofrece el benchmark honesto, no superlativos.
4. **Máx. 2 interacciones/día**, registradas en `targets.md`.

### Tier 1 (objetivo)

| cuenta | ángulo |
|---|---|
| @simonw | cobertura MCP/browser tools; benchmark honesto |
| @swyx | dogfooding stunt, benchmarks de agentes |
| @t3dotgg | "fresh headless browsers are why your AI fails" |
| @mitsuhiko | multiplexer CDP en Rust, agentes que usan la web |
| @levelsio | un solo binario, automatiza tus propias cuentas |

### Tier 2

- Maintainers de Playwright MCP / browser-use (engagement de pares).
- Autores de newsletters MCP/AI.
- Voceros activos de r/mcp.

### Táctica "MrBeast light"

- Crear **mini-stunts** que solo NeoBrowser puede hacer por su arquitectura (sesiones reales):
  - "Mi agente reservó mi cita médica usando mi sesión real."
  - "Mi agente gestionó mis notificaciones de GitHub durante vacaciones."
  - "Mi agente publicó esto desde mi cuenta real."
- Siempre legales, siempre cuentas propias, siempre con consentimiento del usuario.
- Publicar el resultado, no el setup.

---

## 5. Ritmo semanal sugerido

| día | acción |
|---|---|
| Lunes | Planificar contenido de la semana; revisar métricas; actualizar `metrics.csv` |
| Martes | Publicar clip/GIF/demo en X + LinkedIn |
| Miércoles | Engagement value-first (2-3 replies en X/LinkedIn/Reddit/HN) |
| Jueves | Carrusel / estudio / build-in-public post |
| Viernes | Revisar directorios, preparar submissions, responder comentarios |
| Sábado | Contenido ligero: meme dev, milestone, o detrás de cámaras |
| Domingo | Descanso o preparar lanzamiento PH/newsletter |

---

## 6. Métricas de salud del funnel

- **Estrellas/día** objetivo intermedio: 50★/día para llegar a 10k en ~200 días.
- **Engagement rate** por post: >3% en X, >5% en LinkedIn.
- **Shares/saves** en contenido explicativo.
- **Tráfico referido** a GitHub desde X/LinkedIn/Reddit/PH (GitHub Insights).
- **Menciones** en newsletters/directorios.

---

## 7. Anti-patterns (lo que NO funciona)

- Publicar sin formato visual: paredes de texto no comparten.
- Reutilizar el mismo mensaje en todos los canales.
- Responder a influencers con el link en el primer reply.
- Sonar como bot o marketing en HN/Reddit.
- Comprar estrellas, upvotes o engagement.
- Prometer lo que el producto no hace.

---

## 8. Activos pendientes de crear

- [x] GIF comparativo estilo FINTAI (NeoBrowser vs headless genérico).
- [ ] Clip real de 30-40s navegando un sitio con sesión real.
- [ ] Carrusel LinkedIn "MCP servers de navegador comparados".
- [ ] Estudio reproducible de bot detection (sannysoft + Cloudflare real).
- [ ] Landing con contador de estrellas en vivo y barra de progreso a 10k.

---

*Última actualización: 2026-08-19. Revisar semanalmente.*
