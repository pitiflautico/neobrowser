# Inteligencia: cómo se viralizan las herramientas dev en 2026

## Fuentes analizadas
- Show HNs recientes que llegaron a frontpage (browser-use, Stagehand, Hands-Rust, Playwright MCP).
- Posts de r/mcp y r/rust con >100 upvotes.
- Launches de Product Hunt en la categoría Developer Tools.

## Patrones que funcionan

### 1. Demo visual inmediato
- Los posts que suben tienen un GIF/video de 15-30 segundos mostrando la herramienta haciendo algo real (no slides, no screenshots estáticos).
- El demo debe responder a: "¿qué hace esto que no pueda hacer yo con lo que ya tengo?"
- Ejemplo: browser-use enseña un agente reservando un vuelo. Stagehand enseña una form compleja rellenada sola.

### 2. Confesión técnica honesta
- "I got tired of X" o "I broke Y" funcionan mejor que "Introducing Z".
- Admitir debilidades ("Playwright is faster") genera confianza y comentarios.
- La comunidad dev huele el marketing a kilómetros; la honestidad técnica es el mejor marketing.

### 3. Comparativa con nombre y apellidos
- Los benchmarks contra herramientas conocidas (Playwright, Puppeteer, Selenium) atraen comentarios.
- Pero la comparativa debe ser justa y reproducible. Si parece cherry-picked, te destrozan.

### 4. El "one-liner" de instalación
- La fricción mata. Si instalar requiere más de 2 comandos, la mitad de la gente se va.
- `curl | sh` o `brew install` o `cargo install` son el estándar esperado.

### 5. Timing
- HN: martes-jueves, 9-11am ET.
- Reddit: r/mcp es pequeño; r/rust y r/programming dan más alcance pero exigen más calidad técnica.
- Product Hunt: martes o miércoles, con hunter conocido si es posible.

## Lo que NO funciona
- "AI employee" / "bet" / "shutdown" como gimmick principal — atrae atención pero no estrellas de calidad.
- Resubmitir el mismo URL en HN en poco tiempo — redirige al thread viejo.
- Publicar en Reddit con cuenta nueva o sin karma — auto-moderación lo mata.
- Pedir stars directamente — contra las normas de casi todas las comunidades.

## Tácticas aplicables a NeoBrowser
1. **Demo comparativo en video**: NeoBrowser vs Playwright MCP en la misma tarea con sesión real. Mostrar que NeoBrowser entra y Playwright se queda en login.
2. **Post técnico de session hygiene**: ya escrito, pero publicarlo en r/rust o r/programming, no r/mcp (demasiado pequeño).
3. **Benchmark reproducible como contenido**: "We benchmarked 5 browser MCPs against live bot detection. Here's the raw data."
4. **Preparar el día de PH**: hunter conocido, primer comment listo, respuestas preparadas, cross-post en X/LinkedIn/Reddit.
5. **Dominio propio es obligatorio**: PH no acepta github.io/netlify.app. Es el cuello de botella número uno.

## Conclusión
La viralidad de herramientas dev no se compra con posts diarios. Se construye con:
- Un demo visual que haga decir "quiero eso".
- Un canal grande (HN frontpage, PH top 5, influencer con 50k+ followers).
- Timing y preparación.

NeoBrowser tiene el demo y la historia. Falta el canal. El canal es Product Hunt, y Product Hunt exige dominio propio.
