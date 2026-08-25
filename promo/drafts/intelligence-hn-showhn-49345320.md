# Inteligencia: análisis del Show HN exitoso #49345320

Post: https://news.ycombinator.com/item?id=49345320  
Título: "NeoBrowser: An MCP server that drives real Chrome with your logged-in sessions"  
Resultado: 34 puntos, 30 comentarios, no flagged.

## Qué funcionó

- Ángulo claro y honesto: "real Chrome + real sessions".
- No overclaim de bot detection; se menciona detección de walls.
- Benchmark neutral visible en el README.
- Single static binary como diferenciador tangible.

## Objeciones principales (ordenadas por frecuencia/importancia)

1. **Seguridad y control (dongkeren)**
   - Piden domain allowlist, human approval, audit record, revocación.
   - Respuesta: ya están en main (policy engine, `NEOBROWSER_REQUIRE_APPROVAL`, `NEOBROWSER_AUDIT`, vault con revocation, `NEOBROWSER_DOMAIN_ALLOWLIST`).
   - Acción: destacar estos controles en la landing y en futuros posts.

2. **"Claude/Chrome remote-debugging ya puede hacer esto" (Icingdeath, dbbk, cute_boi)**
   - Crítica: es un wrapper de CDP.
   - Respuesta: el valor no es "conectar a Chrome", es el contrato de actions verificadas, policy, vault, wall detection y el harness de 67 tools. Hay que comunicar esto más claro.

3. **"It spits out password in logs" (npodbielski)**
   - Crítica grave sobre logs.
   - Respuesta: `trace/redact.rs` y vault mask secrets, pero el GIF/demo puede haber mostrado texto plano. Revisar demos y logs para no exponer credenciales.

4. **Comparación con browser-use / BrowserOS / Playwright MCP**
   - browser-use tiene 80K★ y mindshare.
   - Respuesta: diferenciación por real sessions + verificación + políticas, no por "más stealth".

5. **"Vibe coded slop" (cute_boi)**
   - Crítica a calidad percibida.
   - Respuesta: mantener CI verde, tests herméticos, releases firmados, documentación honesta.

## Tácticas aplicables

- **Landing**: añadir sección "Security & control" visible antes del fold medio con: domain allowlist, approval gates, audit log, secret redaction, vault.
- **README**: mover la tabla de policy/approval más arriba o crear `docs/SECURITY-FOR-AGENTS.md`.
- **Demos**: revisar que no se muestren contraseñas en texto plano en GIFs/vídeos.
- **Futuros posts**: liderar con el problema de seguridad/control, no con "real Chrome". Ej: "An MCP browser with guardrails: domain allowlist, audit log, and human approval gates."
- **Product Hunt**: usar el ángulo "real sessions + control" en lugar de solo "stealth".
