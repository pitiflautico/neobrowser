# Borrador LinkedIn — ciclo 2026-08-20

**Tono**: first-person, founder, aprender en público. Sin estructura metrónomo.

---

```
Me desperté hoy con un bug que me hizo sudar frío: NeoBrowser estaba deslogando a los usuarios de sus cuentas reales.

No porque escribiéramos en el perfil de Chrome del usuario — nunca lo hacemos. El problema era más sutil: al inyectar miles de cookies de todos sus dominios en un navegador fantasma, Google/LinkedIn/X detectaban una "sesión clonada" y mataban la sesión original.

Así que hemos cambiado el default. Ahora `NEOBROWSER_REAL_PROFILE` solo lee cookies de los dominios que el usuario permite explícitamente (`NEOBROWSER_REAL_PROFILE_DOMAINS=x.com,twitter.com`). Menos magia, menos sorpresas.

La lección que me queda: en automatización de navegador, "más sesión" no es "más real". La plataforma detecta la inconsistencia antes que tú.

¿Hacia dónde vamos? A un "Extension Bridge": una extensión ligera en tu Chrome real que ejecute comandos de NeoBrowser dentro de tu propio navegador. Sin segundo proceso, sin inyección de cookies, sin fingerprint distinto. Literalmente tu navegador, con tu sesión, haciendo lo que le pides.

Si construyes agentes que tocan la web real, me gustaría tu opinión honesta: ¿prefieres un navegador fantasma más rápido, o uno real más indetectable?

→ github.com/pitiflautico/neobrowser

88/10.000 estrellas. Cada una me mantiene encendido.
```

## Notas de publicación
- Adjuntar el GIF `neobrowser-vs-headless.gif` si se publica manualmente.
- Mejor horario LinkedIn: martes-jueves 8:00-10:00 CET.
- Si el feed pide login/captcha, dejar como borrador y notificar.
