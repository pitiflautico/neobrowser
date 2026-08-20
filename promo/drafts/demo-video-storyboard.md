# Storyboard — demo video viral para NeoBrowser

## Video A: "Real Chrome vs Headless" (grabable hoy, sin sesiones reales)

**Duración:** 35-40 segundos  
**Formato:** pantalla real, velocidad real, sin narración  
**Música:** uptempo tech, sin letra

### Escena 1 — Setup (0:00-0:05)
- Terminal limpio, fondo oscuro.
- Texto superpuesto: "Most browser MCPs launch a fresh headless Chrome."
- Se ve el comando: `npx @playwright/mcp@latest --headless`

### Escena 2 — El problema (0:05-0:15)
- Split screen o cortes rápidos:
  - Login page → "sign in required"
  - bot.sannysoft → User Agent check: `HeadlessChrome` ❌
  - File upload form → error
- Texto: "No cookies. Wrong fingerprint. Blocked."

### Escena 3 — NeoBrowser (0:15-0:30)
- Terminal: `neobrowser`
- MCP client recibe la lista de 67 tools.
- Comando de voz/texto del modelo: "log in, upload the file, check bot.sannysoft"
- NeoBrowser ejecuta:
  - Navega a login → "You logged into a secure area!" ✅
  - Sube imagen → server confirma ✅
  - bot.sannysoft → 11/11 pass ✅
- Texto: "Real Chrome. Real fingerprint. Real result."

### Escena 4 — CTA (0:30-0:40)
- Logo NeoBrowser + repo.
- Texto: "github.com/pitiflautico/neobrowser"

---

## Video B: "Mi agente usa MI cuenta" (necesita sesión real)

**Duración:** 25-30 segundos  
**Concepto:** MrBeast light. Mostrar algo que ningún headless puede hacer.

### Opciones de stunt

1. **GitHub notifications**
   - "Agente, ¿qué PRs tengo pendientes?"
   - NeoBrowser abre github.com/notifications usando la sesión real.
   - Lee y resume. El headless se queda en login.

2. **LinkedIn aceptar invitaciones**
   - "Acepta las 5 invitaciones más relevantes."
   - NeoBrowser navega a LinkedIn/mynetwork usando sesión real.
   - Headless: CAPTCHA/login wall.

3. **Reservar cita médica / gestionar subscription**
   - Depende de cuentas personales del usuario.
   - Alto engagement porque es "un agente haciendo mi vida real".

### Storyboard genérico

- 0:00-0:05: Hook. "Tu agente no puede usar tu LinkedIn. El mío sí."
- 0:05-0:10: Pantalla dividida. Izquierda: headless en login wall. Derecha: NeoBrowser ya dentro.
- 0:10-0:25: Time-lapse del agente ejecutando la tarea real.
- 0:25-0:30: CTA + repo.

---

## Notas técnicas de grabación

- Usar Screen Studio, OBS o QuickTime.
- Grabar en 1080p, 60 fps.
- Cursor visible para que se sienta humano.
- Sin narración; texto superpuesto mínimo.
- Velocidad real; no acelerar demasiado.
- Exportar como MP4 < 20 MB para X/LinkedIn.

## Scripts de apoyo

- `rust/scripts/demo.py` ya hace login + upload + bot check.
- Para sesión real: `NEOBROWSER_REAL_PROFILE=Default neobrowser` o `NEOBROWSER_ATTACH_PORT=9222`.
