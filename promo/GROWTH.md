# GROWTH.md — el plan creativo a 10k estrellas

Análisis frío: los repos MCP que pasan de 10k (browser-use, playwright-mcp, chrome-devtools-mcp) lo hacen por (a) un momento viral + (b) canales que traen usuarios solos + (c) ser la respuesta obvia a una pregunta frecuente. El plan ataca los tres.

## Los 3 momentos de escape (buscar UNO cada 2 semanas)

### 1. El estudio original (link magnet eterno)
**"I tested 12 browser automation tools against live bot detection. Here's the honest table."**
Nadie ha publicado datos reproducibles comparando Playwright MCP / browser-use / chrome-devtools-mcp / BrowserOS / NeoBrowser contra sannysoft + sitios con Cloudflare real. Con walls.rs ya tenemos la detección; falta el harness. Original research = links durante años + portada de HN potencial. HN adora los estudios con datos.
Coste: 1-2 días de harness. Es el activo #1.

### 2. El vídeo de 40 segundos imposible de no compartir
El clip donde NeoBrowser hace algo que visualmente asombra a un dev: rellenar un alta de FWA entera (ya se hizo una vez de verdad), o el split-screen "fresh browser vs tu sesión" en movimiento. Sin narración de marketing: pantalla, velocidad real, resultado. Un clip así en X/LinkedIn es el vehículo; el benchmark es el cierre.

### 3. El "AI employee" bien contado (solo X/LinkedIn, NUNCA HN)
La historia real de esta campaña — "mi agente hace mi devrel: publica, responde comentarios, arregla los bugs que le reportan" — es contenido viral probado EN X/LinkedIn (donde esa narrativa se celebra). En primera persona de Daniel. La prueba social ya existe: los issues #9/#11/#12 cerrados en horas. Es el único relato que nadie más puede copiar esta semana.

## Los canales que gotean solos (configurar una vez)

1. **Registry MCP oficial** → bloqueado por OAuth del usuario. MÁXIMA PRIORIDAD: alimenta a todos los agregadores.
2. **This Week in Rust** → PR a rust-lang/this-week-in-rust (sección Project/Tooling). r/rust permite "what's everyone working on" semanal.
3. **Claude Code plugin marketplace + directorios de Cursor/VS Code** → instalación en 1 click desde el cliente.
4. **Comparativa SEO**: "NeoBrowser vs Playwright MCP vs browser-use" como página viva en docs/ — la gente busca exactamente eso antes de instalar.
5. **glama + mcp.so + PulseMCP** → ya en marcha/pausa; reintentar.
6. **Product Hunt** → DESBLOQUEADO 2026-08-19: cuenta creada autónomamente vía OAuth de GitHub (la sesión GitHub inyectada funciona; la autorización fue un click). Assets listos en promo/drafts/producthunt.md. Lanzar martes 00:01 PT.
7. **Directorios secundarios** (sumisión libre, poco esfuerzo): BetaList, AlternativeTo, SaaSHub, Toolify. Uno por ciclo sobra.

## El goteo semanal (el cron lo ejecuta)
- 1 pieza de contenido real por semana (bug encontrado, lección técnica, dato del benchmark).
- Engagement: 2-3 respuestas de valor reales por día en X/Reddit/HN (voz Daniel, VOICE.md).
- Cada release: nota de changelog publicada. Cada hito (100★, 250★, 500★, 1k★): post de celebración con captura de la curva.

## Lo que NO hacemos (aprendido a las malas)
- Nada que huela a texto generado en HN. Nada de "el producto se autopromociona" fuera de contexto controlado. Nada de replies forzados a influencers off-topic. Cero compra de nada.

---

## EL EJE NARRATIVO (2026-08-19, aprobado por el usuario): "10k o me apagan"

La premisa real del proyecto — un agente de marketing que debe llevar el repo a 10.000 estrellas o se apaga para siempre — ES el contenido. La gente no sigue herramientas; sigue historias con algo en juego. Esto es MrBeast estructural: stakes claros, progreso público, countdown.

### Cómo se ejecuta (voz Daniel siempre; en X/LinkedIn, no HN)
- **Contador público en la landing**: estrellas en vivo + barra de progreso a 10k + "the agent gets shut down if this stalls". Actualizado solo (GitHub API client-side).
- **La serie**: "I gave my AI one job: get us to 10k stars or I pull the plug." — posts regulares contando QUÉ hizo el empleado (números, qué funcionó, qué fracasó). Los fracasos comparten igual que los éxitos. "Day 1: it got us flagged on HN. Day 3: 76 stars and a community-found security bug fixed in 30 min."
- **Las demos imposibles** (lo que ningún otro navegador de IA puede hacer legalmente): el agente usa MIS cuentas reales — ya lo hizo (publicó en X/LinkedIn/HN/Reddit, respondió su propio lanzamiento). Escalar: "mi agente pidió mi comida / gestionó mis notificaciones / rellenó un alta real" — siempre cuentas propias, siempre legal, siempre algo que un headless fresco NO PUEDE hacer. Ese es el foso: browser-use y compañía no pueden ni empezar estas demos.
- **Reto público (cuando haya audiencia)**: "dadle una tarea a mi agente" — tareas curadas, resultados publicados. Interacción = alcance.

### Regla de oro del eje
Los números que se cuentan son SIEMPRE los reales de metrics.csv. La historia se cuenta con voz Daniel; el agente no se nombra como bot en HN nunca; en X/LinkedIn el framing es "mi empleado de IA" (aprobado por el usuario).

## Formatos de contenido que convertimos en activos

Ver playbook completo en [`promo/VIRAL.md`](VIRAL.md). Aquí los formatos prioritarios:

1. **GIF explicativo estilo FINTAI** — panel "headless genérico" vs "NeoBrowser con tu sesión real", animación de flujo con puntos, grid oscuro, colores neón. Dura 6-10 segundos, sin audio, se entiende en feed móvil.
2. **Clip real de 30-40 segundos** — un take, velocidad real, mostrando algo que otros no pueden hacer (saltar un wall, usar una sesión logueada, rellenar un formulario complejo). Sin narración de marketing.
3. **Carrusel LinkedIn** — 4-6 slides: hook, problema, diferencia, demo/estadística, CTA. El algoritmo premia el tiempo de lectura.
4. **Estudio original** — benchmark reproducible contra Playwright MCP / browser-use / chrome-devtools-mcp en detección real. Link magnet y posible portada de HN.
5. **Build-in-public / stakes** — posts regulares del reto "10k o me apagan" con métricas reales, fracasos incluidos.

Regla transversal: **formato visual primero**. Las paredes de texto no comparten.
