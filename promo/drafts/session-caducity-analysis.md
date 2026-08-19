# Análisis: por qué caducan las sesiones en plataformas duras

## Resumen
- **X** funciona con `NEOBROWSER_REAL_PROFILE=Profile 24`: inyecta ~4974 cookies y la sesión persiste.
- **LinkedIn** no funciona solo con cookies inyectadas, incluso con `NEOBROWSER_INCLUDE_IDENTITY_COOKIES=1` (~5014 cookies). La carga `/feed/` sin redirigir a login, pero el UI no detecta sesión (`feed-identity-module` ausente). Requiere `localStorage`/`sessionStorage` con tokens de sesión que no se inyectan.
- **Reddit** (`old.reddit.com`) funciona inicialmente con cookies, pero el submit del formulario puede forzar re-autenticación/captcha si faltan tokens CSRF o si el perfil Ghost dispara detección de bot.
- **GitHub / Product Hunt OAuth** requieren sesión completa de GitHub (cookies + posiblemente localStorage de github.com).

## Causas raíz
1. **NEOBROWSER_REAL_PROFILE solo inyecta cookies** en un perfil Ghost limpio. No copia `localStorage`, `sessionStorage`, `IndexedDB`, service workers ni tokens en memoria.
2. **Plataformas "duras"** (LinkedIn, GitHub) usan almacenamiento local para tokens y validación de dispositivo. Las cookies solas no bastan.
3. **El perfil real de Chrome del usuario está bloqueado** por `SingletonLock` mientras Chrome nativo corre, así que NeoBrowser no puede lanzar Chrome directamente con ese `user-data-dir`.
4. **Attach mode** (`NEOBROWSER_ATTACH_PORT`) requiere que Chrome ya esté lanzado con `--remote-debugging-port`, algo que el usuario no hace normalmente.

## Soluciones posibles (orden de preferencia)

### A. Copiar perfil real a un perfil NeoBrowser (mejor para automatización)
Pedir al usuario que cierre Chrome un momento, copiar `~/Library/Application Support/Google/Chrome` a `~/.neobrowser/profiles/real`, y lanzar NeoBrowser con `NEOBROWSER_PROFILE=real`. Después el usuario puede volver a abrir su Chrome normal.

Pros: sesión completa, funciona con LinkedIn/GitHub/Reddit.
Contras: requiere cerrar Chrome del usuario una vez; el perfil copiado se desincroniza con el original si cambian credenciales.

### B. Mejorar `save_session` / `restore_session`
Extender la tool `save_session` para capturar `localStorage`, `sessionStorage` e `IndexedDB` de dominios clave (`linkedin.com`, `github.com`, `reddit.com`, `x.com`). Restaurar al navegar a esos dominios.

Pros: no requiere cerrar Chrome; snapshots periódicos mantienen sesión viva.
Contras: más desarrollo; algunas plataformas pueden invalidar tokens por cambio de fingerprint.

### C. Attach mode con Chrome real
Reiniciar Chrome del usuario con `--remote-debugging-port=9222` y usar `NEOBROWSER_ATTACH_PORT=auto`.

Pros: usa el navegador real y logueado directamente.
Contras: invasivo; el usuario pierde sus pestañas si no usa "reabrir pestañas"; no es automático.

### D. Login scripted con credenciales
Usar la tool `login` de NeoBrowser con usuario/contraseña cuando estén disponibles.

Pros: no depende de Chrome del usuario.
Contras: no tenemos credenciales de LinkedIn/GitHub/Reddit; 2FA/captcha lo bloquean.

## Resultado de las pruebas
- `NEOBROWSER_INCLUDE_IDENTITY_COOKIES=1` aumentó las cookies inyectadas de ~4974 a ~5014, pero LinkedIn siguió sin detectar sesión.
- Copiar `Profile 24` a `~/.neobrowser/profiles/real` mientras Chrome del usuario corría no funcionó: las cookies de LinkedIn estaban presentes (`liap`, `bcookie`, etc.) e IndexedDB se copió, pero el login no persistió. Probablemente Chrome invalida tokens al detectar un `user-data-dir` clonado, o faltó `localStorage`/`sessionStorage` consistente.
- `save_session` funciona para capturar cookies + localStorage del dominio actual (X), pero requiere que el dominio objetivo ya esté logueado en la sesión controlada.

## Recomendación inmediata
Para LinkedIn/GitHub/Product Hunt/Reddit se necesita **una de estas dos**:
1. **Copia limpia del perfil real**: pedir al usuario que cierre Chrome, copiar `~/Library/Application Support/Google/Chrome/Profile 24` a `~/.neobrowser/profiles/real/Default`, y lanzar con `NEOBROWSER_PROFILE=real`. Esto evita la inconsistencia de copiar mientras Chrome escribe.
2. **Attach mode**: reiniciar Chrome del usuario con `--remote-debugging-port=9222` y usar `NEOBROWSER_ATTACH_PORT=auto`.

La opción 1 es más robusta para automatización; la 2 es más fácil para un uso puntual.

## Actualización de scripts
Los scripts de promo deben:
- Seguir usando `NEOBROWSER_REAL_PROFILE=Profile 24` para X (suficiente).
- Usar `NEOBROWSER_PROFILE=real` + copia limpia del perfil para LinkedIn/GitHub/Reddit/Product Hunt.
- Usar `NEOBROWSER_INCLUDE_IDENTITY_COOKIES=1` como fallback cuando solo haya cookies.
