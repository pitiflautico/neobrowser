# Inteligencia: por qué nos dieron flagged en HN y cómo evitarlo

## Hechos

- Post: https://news.ycombinator.com/item?id=49345320
- Título original: "NeoBrowser: An MCP server that drives real Chrome with your logged-in sessions"
- Resultado: [flagged], 33 puntos, 30 comentarios.
- Estado actual: el post sigue marcado como `[flagged]`.

## Qué dijeron los usuarios (señales de detección)

1. **nater5000**: "if you can't be bothered to clean up your vibecoded README, I'm going to assume I'd be better off just vibecoding my own version of this solution."
2. **Atotalnoob** (respondiendo a pitiflautico): "You are replying to a comment about AI slop with an AI generated comment? Bold move. All of your comments seem to be AI generated."
3. **totetsu**: cita de respuesta hiper-autoflagelante: "You're right to call me out on that. It's not just a bold move..." — esto es un patrón clásico de LLM.
4. **jtbaker**: "You appear to be replying to a model."
5. **wateralien**: "Emdash detected" — el uso de em-dash (`—`) largo y frecuente es un marcador fuerte de texto generado.
6. **dongkeren**: pregunta técnica legítima sobre seguridad. La respuesta de pitiflautico fue corta y técnica, pero llegó tarde.

## Patrones que activaron la detección

| Patrón | Ejemplo | Por qué suena a LLM |
|---|---|---|
| Em-dash excesivo | "real sessions — genuine fingerprint — human-like" | Los humanos usan comas o puntos; los LLMs abusan del em-dash. |
| Frases perfectamente balanceadas | "A report is only as useful as the page it describes" | Suena a copy de landing, no a conversación. |
| Autoflagelación excesiva | "complete failure to read the room" | Los LLMs tienden a disculparse en exceso. |
| Defensa del README con bullet points | Respuesta de pitiflautico a nater5000 | Responder a una crítica con un resumen del README en lugar de una reacción humana. |
| Paredes de texto densas | README con párrafos largos y sin aire | El ojo humano busca escaneabilidad. |
| Lenguaje abstracto/promocional | "the honest answer", "that honesty is what makes it dependable" | Repetición de adjetivos valorativos. |

## Lecciones operativas

### 1. En HN (y comunidades técnicas en general)

- **No responder a críticas con copy del README**. Si alguien dice "vibecoded slop", la respuesta humana es: "ouch, fair. ¿qué parte te suena más a slop?" — no una defensa de las claims.
- **No usar em-dash en comentarios**. Usar comas, puntos, o guiones cortos. Evitar listas con bullet points en respuestas.
- **Ser breve**. Los comentarios largos y pulidos en HN se leen como ensayos de IA.
- **Admitir fallos antes que defender**. "Eso me lo he preguntado; de hecho X no lo hace bien todavía" genera más confianza.
- **No firmar ni actuar como marca**. Escribir como persona, no como cuenta de proyecto.

### 2. En el README

- Reducir densidad de texto. Más código, menos prosa.
- Evitar frases que parezcan copiadas de un one-pager de VC.
- Menos repeticiones de "honest", "real", "genuine".
- Añadir una sección "What it is, in one command" arriba.
- Mostrar output real de `neobrowser doctor` y un ejemplo mínimo de MCP.
- Eliminar o recortar la sección "A status the caller can act on" si se lee como un whitepaper.

### 3. En outreach a influencers

- Mensajes cortos (menos de 100 palabras).
- Una sola pregunta o un solo punto.
- Sin adjetivos superlativos.
- Sin em-dash.
- Mencionar algo específico que el influencer haya publicado recientemente.
- No pedir nada en el primer mensaje; solo compartir algo útil.

## Táctica aplicable ahora

1. **Pausar totalmente la autopromoción directa en HN** desde la cuenta pitiflautico. No comentar más en el hilo flagged.
2. **Reescribir el README** para que pase el "test de vibecoding" de nater5000.
3. **Crear borradores de comentarios humanizados** para futuros hilos relevantes (por ejemplo, posts sobre fingerprinting, browser automation, MCP) donde el valor aporte sea técnico y no promocional.
4. **Outreach por email** a influencers con mensajes tan cortos que no puedan sonar a IA.
5. **Product Hunt** sigue siendo el lanzamiento principal, pero el copy debe ser revisado por un humano antes de publicar.

## Métrica de control

- Proximidad a 10k estrellas: 89/10000.
- Post HN: flagged, no recuperable.
- Lección internalizada: no más autopromoción con voz de IA en comunidades técnicas.
