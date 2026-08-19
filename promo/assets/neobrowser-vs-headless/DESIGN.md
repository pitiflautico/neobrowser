# DESIGN.md — NeoBrowser vs Headless (GIF viral)

## Style name
Neon Grid — inspirado en el GIF de referencia de FINTAI: diagrama comparativo con flujo de datos animado sobre fondo oscuro tipo grid.

## Mood
Técnico, limpio, revelador. No agresivo: el contraste lo hace el mensaje, no los colores chillones.

## Colors

| role | hex | usage |
|---|---|---|
| background | `#0B0F14` | canvas general |
| grid | `#1A2330` | líneas de grid sutil |
| foreground | `#EAF2FF` | texto principal |
| accent success | `#00F0A8` | NeoBrowser, check, output |
| accent danger | `#FF4D6D` | headless fail, login wall, cross |
| accent info | `#38BDF8` | flujo genérico, CPU |
| panel bg | `#111827` | fondo de tarjetas |
| panel border | `#1F2937` | borde sutil de tarjetas |

## Typography

- **Headlines / labels:** `Space Grotesk`, sans-serif, 700-900.
- **Technical / code labels:** `JetBrains Mono`, monospace, 400-700.
- Tamaños: título 72px, panel title 42px, cajas 22px, micro labels 18px.

## Motion rules

- Entradas: `power3.out` y `expo.out`, snappy.
- Líneas: dibujo vía `stroke-dashoffset` a 1.2s.
- Puntos de flujo: movimiento lineal por segmentos, 0.4s por salto.
- Iconos: `back.out(1.7)` para pop.
- Sin exit animations excepto fade final del CTA.

## What NOT to do

- No gradients de fondo completos (banding en GIF).
- No dos sans-serifs (usamos Space Grotesk + JetBrains Mono).
- No texto pequeño (< 18px).
- No animar propiedades no visuales.
