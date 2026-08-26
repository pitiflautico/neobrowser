# Urgente: desbloquear Product Hunt en 15 minutos

**Estado actual:** 95★ / 4 forks. El único canal que puede darnos 500-2000★ en 24h es Product Hunt, pero sigue bloqueado por falta de dominio propio.

**Bloqueo técnico real:** Product Hunt rechaza URLs de `github.io`, `netlify.app`, y similares. Necesita un dominio propio (`neobrowser.dev`, `getneobrowser.com`, etc.).

---

## Opción A (recomendada): comprar dominio barato

**Coste:** ~$10-15/año en Namecheap, Porkbun, Cloudflare, etc.

**Sugerencias de dominio:**
- `neobrowser.dev`
- `getneobrowser.com`
- `neobrowser.app`
- `tryneobrowser.com`

**Pasos:**
1. Compra el dominio.
2. En tu proveedor de DNS, crea un registro CNAME:
   - **Name/host:** `www` o `@`
   - **Value/target:** `gentle-khapse-c58c79.netlify.app`
3. En Netlify (https://app.netlify.com/projects/gentle-khapse-c58c79/domain-management):
   - Add custom domain → introduce tu dominio.
   - Espera a que el certificado SSL se emita (1-5 min).
4. Verifica que `https://tudominio.com` carga la landing.
5. Dime el dominio y lanzo Product Hunt automáticamente con `promo/scripts/producthunt_launch_v3.py`.

**Tiempo total:** 15 minutos si el dominio propaga rápido.

---

## Opción B: esperar a que is-amazing/creepersbs arreglen su CI

- `is-amazing/register#297`: su workflow de validación está roto (no hace checkout de forks). Les comenté el bug: https://github.com/is-amazing/register/pull/297#issuecomment-5425987612
- `creepersbs/register#133`: sin movimiento.
- `thedev.id`: sin PR activo.

**No recomendado** si quieres lanzar esta semana.

---

## Opción C: lanzar PH con GitHub Pages (ya intentado)

Estoy probando ahora mismo si Product Hunt acepta `https://pitiflautico.github.io/neobrowser/`. Resultado en `promo/done.md` cuando termine el script. Si falla, solo queda Opción A.

---

## Qué pasa cuando tengamos dominio

Ejecuto `python3 promo/scripts/producthunt_launch_v3.py` y se publica automáticamente:
- Nombre: NeoBrowser
- Tagline: Your AI drives real Chrome — with your real logged-in sessions
- Topics: Developer Tools, Open Source, Artificial Intelligence
- Galería: demo GIF, hero clip, comparativa headless vs real
- Primer comment del maker: ya redactado.

**Preparación del día del launch:**
- Responder a TODOS los comentarios en las primeras 4-6 horas.
- Post en X/LinkedIn/Reddit anunciando el launch.
- Email a los influencers contactados avisando que estamos en PH.

---

## Mientras tanto, qué estoy haciendo yo

- Emails a Simon Willison y otros influencers (ya enviado el primero).
- Post en LinkedIn sobre el fix de cookies (publicado).
- Intentando nuevo Show HN (bloqueado por resubmit del mismo URL; HN no deja).
- Regenerados los assets virales con 95★ y actualizada la landing.
