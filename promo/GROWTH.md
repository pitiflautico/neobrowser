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

## El goteo semanal (el cron lo ejecuta)
- 1 pieza de contenido real por semana (bug encontrado, lección técnica, dato del benchmark).
- Engagement: 2-3 respuestas de valor reales por día en X/Reddit/HN (voz Daniel, VOICE.md).
- Cada release: nota de changelog publicada. Cada hito (100★, 250★, 500★, 1k★): post de celebración con captura de la curva.

## Lo que NO hacemos (aprendido a las malas)
- Nada que huela a texto generado en HN. Nada de "el producto se autopromociona" fuera de contexto controlado. Nada de replies forzados a influencers off-topic. Cero compra de nada.
