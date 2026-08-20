# Track de contacto proactivo — gente que puede ayudar

**Por qué ahora**: ya hay prueba social (18 pts HN, 35★ en 90 min, benchmark público honesto). El mensaje ya no es "mira mi proyecto" sino "hemos lanzado esto con números raros de honestidad — ¿nos echas un ojo experto?".

## Escalera de contacto (prioridad)

### Nivel 1 — Los que ya interactuaron (tibios, máxima conversión)
- **Johnny_Bonk** (HN): usa sesiones firmadas a diario y construyó una skill para ello. Pedirle que pruebe NeoBrowser y nos diga qué le falta — early adopter natural.
- **Icingdeath** (HN): usa `--remote-debugging-port` a mano. Es attach-mode user nato; su feedback técnico vale oro.
- **dmix** (HN): usuario de BrowserOS. Comparativa honesta interesado.
Vía: responder en el propio hilo de HN (natural, no invasivo). Si su perfil tiene email público, email opcional.

### Nivel 2 — Pares del ecosistema MCP/browser (feedback técnico + cross-pollination)
- Maintainers de **browser-use**, **BrowserOS**, **Playwright MCP**: compartir el benchmark y pedir crítica. Los pares respetan datos honestos.
- Autores de **MCP newsletters** (PulseMCP newsletter, etc.): con números en la mano, proponer mención.

### Nivel 3 — Figuras (solo cuando haya un ángulo personalizado fuerte)
- simonw, swyx, t3dotgg, mitsuhiko (ver targets.md). Regla: reply público con valor > DM frío. DM solo si ya hubo interacción previa o tienen DMs abiertos y el mensaje es impecable.

## Mensaje tipo (adaptar SIEMPRE a la persona — nunca copiar tal cual)

**Para early adopters técnicos (Nivel 1)**:
"Vi tu comentario en HN — tú haces exactamente el caso de uso para el que lo construí. NeoBrowser drivea tu Chrome real con tus sesiones (opt-in, keychain del SO). Si lo pruebas, lo que más me interesa es qué te falla primero: los bugs que reportó la comunidad esta semana ya van 2 arreglados. https://github.com/pitiflautico/neobrowser"

**Para pares (Nivel 2)**:
"Maintainer de NeoBrowser (MCP server, drivea Chrome real via CDP). Publicamos un benchmark neutral contra Playwright MCP — sin tunear nada a nuestro favor, y ellos ganan en velocidad. Me interesa vuestra crítica a la metodología: bench/compare.md. Si veis algo mal medido, lo corregimos."

**Para newsletters**:
"Lanzamos NeoBrowser en HN esta semana (18+ puntos primeras horas): MCP server que drivea Chrome real con sesiones reales, stealth genuino verificado en CI, y un benchmark honesto donde reconocemos dónde perdemos. Si encaja en la newsletter: github.com/pitiflautico/neobrowser"

## Registro de contactos
| fecha | cuenta | vía | tema | estado | notas |
|---|---|---|---|---|---|
| 2026-08-20 | @simonw | email/Bluesky/Mastodon | benchmark honesto browser MCPs | borrador listo | pendiente envío del usuario; draft en `outreach-simonw-2026-08-20.md` |

## Reglas
- Máx 1-2 contactos/día, siempre personalizados. Nada de plantillas en masa.
- Pedir feedback > pedir difusión. La difusión viene sola si el producto impresiona.
- Registrar cada contacto en targets.md (quién, vía, tema, respuesta).
- Nunca inventar relación ("como hablamos...") si no existió.
