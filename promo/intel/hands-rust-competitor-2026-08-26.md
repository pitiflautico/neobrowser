# Inteligencia de competidor: Hands-Rust MCP/CLI (2026-08-26)

## Qué es
- Show HN: https://news.ycombinator.com/item?id=49405405
- Autor: ryan-b
- Stack: Rust MCP/CLI.
- Propuesta: el agente "mira la pantalla", mueve el ratón real, teclea y clica en Chrome real sin convertirlo en un navegador de automatización.

## Posicionamiento
- Muy cercano a NeoBrowser en el pitch: "Chrome real, sesión real, no headless".
- Diferencia técnica clave: usa visión de escritorio / OS-level automation en lugar de CDP (Chrome DevTools Protocol).
- Enfoque Windows-first (menciona "Windows PC").

## Tradeoffs visibles
| Aspecto | Hands (visión de escritorio) | NeoBrowser (CDP) |
|---|---|---|
| Aplicabilidad | Cualquier app, no solo Chrome | Solo Chrome/Chromium |
| Precisión | Limitada por visión/resolución/focus | Exacta, accede al DOM |
| Velocidad | Más lenta (tiempos de UI) | Más rápida |
| Seguridad | Requiere acceso a pantalla | Mantiene sandbox del renderer |
| Sesiones reales | Sí, al usar Chrome real | Sí, con real-profile o attach mode |
| Multiplataforma | Windows-first | macOS/Linux/Windows |

## Tácticas aplicables
1. **Diferenciación clara en futuros posts**: no competir en "Chrome real vs headless", sino en "CDP preciso + sandbox vs visión de escritorio genérica". El sandbox y la precisión DOM son nuestras ventajas.
2. **Demos técnicos comparativos**: un GIF/video mostrando la misma acción en Hands (lento, depende de focus) y NeoBrowser (rápido, determinista) sería muy convincente, pero hay que hacerlo sin atacar al otro proyecto.
3. **Seguridad como mensaje central**: Hands requiere acceso a pantalla; NeoBrowser mantiene el sandbox de Chrome. Esto es un diferenciador fuerte para entornos enterprise.
4. **Multiplataforma**: subrayar que NeoBrowser funciona en macOS/Linux/Windows, no solo Windows.
5. **Attach mode + Extension Bridge**: el roadmap de Extension Bridge (ejecutar comandos dentro del Chrome real del usuario sin segundo proceso) es la respuesta más directa a Hands: misma experiencia de "Chrome real", pero sin visión ni CDP visible.

## Amenaza
- Baja/moderada. El proyecto es muy nuevo (5 puntos, 2 comentarios en HN) y Windows-only.
- La visión de escritorio es un approach válido pero distinto; no es un reemplazo directo para workflows precisos de navegador.
- Su autor ya está en HN, así que el canal está caliente para este tipo de proyectos.

## Oportunidad
- El hecho de que haya un Show HN similar con tracción mínima valida que el problema (automatización real de navegador) interesa.
- Podemos usar esto como ancla en futuros posts: "Hay varios approaches al browser agent real; aquí explico por qué elegimos CDP".
