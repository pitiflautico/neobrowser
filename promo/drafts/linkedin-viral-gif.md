# Borrador LinkedIn — post viral con GIF comparativo

**Asset:** `promo/assets/neobrowser-vs-headless/neobrowser-vs-headless.gif`
**Cuándo publicar:** martes o miércoles, 8:30-10:00h CET
**Tono:** aprender en público, primera persona, sin exceso de marketing

---

```
He pasado meses peleándome con la misma contradicción: los agentes de IA necesitan navegar la web como humanos, pero casi todos los MCP browsers lanzan un Chrome headless limpio y esperan que pase desapercibido.

Spoiler: cada vez que una web tiene un login real, un bot-check decente o una sesión que importa, ese headless se queda en la puerta.

Así que en lugar de seguir maquillando un navegador fantasma, construí NeoBrowser: un MCP server que controla tu Chrome real (vía Chrome DevTools Protocol) y puede reutilizar tus sesiones ya logueadas.

No falsea el fingerprint. No inventa WebGL. Es literalmente tu navegador, recibiendo comandos.

Tres cosas que me costaron más de lo que pensaba:

1. El descifrado de cookies del perfil real (Keychain en macOS, secret-service en Linux, DPAPI en Windows). Opt-in, nada automático que te desloguee.

2. Detectar muros en lugar de intentar atravesarlos. NeoBrowser reconoce captcha, rate-limit, consent y login gates, y le devuelve al modelo una estrategia en vez de martillear la página.

3. Un benchmark honesto vs Playwright MCP. Playwright gana en velocidad. NeoBrowser gana en sesiones reales y uploads. Los números están en el repo para quien quiera auditarlos.

67 herramientas, binario estático de ~6.4 MB en Rust, MIT.

Si te interesa el ecosistema MCP o la automatización de navegador real, el repo está aquí: https://github.com/pitiflautico/neobrowser

#opensource #rust #aiagents #mcp #browserautomation
```

---

## Notas
- LinkedIn premia que el primer párrafo sea un hook personal. No empezar con "Lanzo...".
- El GIF debe ir adjunto al post, no como link externo.
- Responder a comentarios en las primeras 2 horas para que el algoritmo lo empuje.
- Si la versión en español funciona, traducir al inglés 3-4 días después.
