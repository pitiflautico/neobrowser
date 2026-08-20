# Estrategia creativa: usar el perfil real de Chrome sin ser detectado

## El problema real

Inyectar cookies de un perfil real en un navegador Ghost limpio es técnicamente fácil, pero las plataformas duras (Google, LinkedIn, Microsoft, GitHub, X) detectan la discrepancia:

- **Fingerprint distinto**: WebGL, canvas, fonts, viewport, timezone, hardwareConcurrency.
- **Cookie timing**: miles de cookies aparecen de golpe en un navegador recién nacido.
- **Doble sesión**: el proveedor ve dos dispositivos con la misma cookie de sesión y mata ambos.
- **Headless tells**: aunque los ocultemos, el stack de red/idle es distinto al de un usuario real.

El resultado: el usuario pierde el login en su Chrome real.

## La estrategia ganadora: "Extension Bridge" (puente de extensión)

En lugar de clonar el perfil o lanzar un segundo Chrome, **NeoBrowser actúa como cerebro remoto del Chrome real del usuario**.

### Cómo funciona

1. El usuario instala una extensión ligera de Chrome (`NeoBrowser Bridge`).
2. La extensión se conecta a NeoBrowser vía WebSocket local (o native messaging).
3. Cuando NeoBrowser quiere actuar, no abre un navegador fantasma: **envía comandos a la extensión**.
4. La extensión ejecuta esos comandos en el Chrome real del usuario:
   - Abre una pestaña.
   - Hace scroll/clic/typing real.
   - Lee el DOM.
   - Devuelve el resultado a NeoBrowser.

### Por qué es indetectable

- No hay segundo proceso Chrome.
- No hay inyección de cookies: la extensión usa la sesión ya activa del usuario.
- El fingerprint es el **real**: es literalmente el navegador del usuario.
- Las acciones son eventos reales (`isTrusted`), no síntesis CDP.
- El proveedor ve exactamente lo mismo que vería si el usuario hiciera clic manualmente.

### Ventaja competitiva brutal

Ningún otro navegador de IA ofrece esto hoy. Todos lanzan un Chrome aparte y pelean contra la detección. NeoBrowser podría decir: *"No imitamos un navegador. Usamos el tuyo."*

## Variantes y niveles de madurez

### Nivel 1: Native Messaging Bridge (más robusto)

- La extensión se registra con un host nativo (`neobrowser-bridge` binario).
- Chrome inicia el host como proceso hijo; el host habla con NeoBrowser por TCP/WebSocket.
- No requiere que NeoBrowser esté escuchando en un puerto expuesto.
- Funciona con Chrome cerrado: Chrome se lanza solo cuando la extensión recibe un mensaje.

### Nivel 2: WebSocket Bridge (más rápido de implementar)

- NeoBrowser lanza un servidor WebSocket en `ws://127.0.0.1:PORT`.
- La extensión se conecta y recibe comandos JSON.
- Más fácil de depurar, pero requiere que el usuario mantenga NeoBrowser corriendo.

### Nivel 3: Attach Mode con reinicio elegante

- NeoBrowser reinicia el Chrome real del usuario con `--remote-debugging-port=9222`.
- Preserva pestañas con `--restore-last-session`.
- NeoBrowser se ata vía CDP al Chrome real.
- Menos elegante que la extensión, pero funciona hoy sin desarrollar una extensión.

## Estrategia intermedia (mientras llega la extensión): "Cold Profile Mirror"

1. Pedir al usuario que cierre Chrome por completo.
2. Copiar todo `~/Library/Application Support/Google/Chrome/Profile 24` a `~/.neobrowser/profiles/real/Default`.
3. Lanzar NeoBrowser con `NEOBROWSER_PROFILE=real`.
4. El Ghost Chrome arranca con el perfil exacto del usuario.
5. **Nunca** correr el Chrome real y el Ghost simultáneamente (el SingletonLock de Chrome lo impide de todos modos).

Pros: sesión completa, funciona hoy.
Contras: no puede usar el navegador real mientras NeoBrowser trabaja.

## Qué hacer ahora

1. **Parar de inyectar cookies por defecto** (fix en curso: `NEOBROWSER_REAL_PROFILE_DOMAINS`).
   - Solo inyectar dominios explícitamente permitidos por el usuario.
   - Esto evita el deslogeo accidental mientras desarrollamos la extensión.

2. **Diseñar e implementar el Extension Bridge** como killer feature.
   - MVP: extensión que abre URL, hace clic y lee DOM vía WebSocket.
   - Integrar con el registry de tools de NeoBrowser para que `find`, `click`, `type`, etc. puedan ejecutarse en el Chrome real.

3. **Documentar el Attach Mode** como workaround inmediato.
   - Script de reinicio elegante de Chrome con `--remote-debugging-port`.

## Riesgos y mitigaciones

| Riesgo | Mitigación |
|--------|-----------|
| Chrome Web Store rechaza la extensión | Distribuir como `.crx`/`load unpacked` para desarrolladores/early adopters. |
| La extensión requiere permisos amplios | Pedir solo `<all_urls>` cuando el usuario lo autorice; auditar código abierto. |
| Latencia vs CDP local | WebSocket en localhost es sub-milisegundo; comparable a CDP. |
| Usuario no quiere instalar extensión | Fallback a Cold Profile Mirror y Attach Mode. |

## Narrativa de marketing

- "NeoBrowser no emula un navegador. Se convierte en el copiloto de tu navegador real."
- "Tu sesión de Google/LinkedIn/GitHub nunca sale de tu Chrome."
- "La única herramienta de IA que no pelea contra la detección de bots porque no hay nada que detectar."

---

*Estrategia diseñada para resolver el deslogeo crónico y diferenciar a NeoBrowser del resto de navegadores de IA.*
