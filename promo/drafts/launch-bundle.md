# Launch bundle — NeoBrowser

Todo el material de promoción listo para ejecutar. Usar como checklist de lanzamiento coordinado.

## Métrica actual

- **GitHub:** 88★ / 4 forks
- **Target:** 10.000★
- **Landing:** https://pitiflautico.github.io/neobrowser/
- **Repo:** https://github.com/pitiflautico/neobrowser

---

## Contenido social (listo para publicar)

| plataforma | archivo | asset | estado |
|---|---|---|---|
| X | `promo/drafts/x-viral-gif.md` | `promo/assets/neobrowser-vs-headless/neobrowser-vs-headless.gif` | listo; bloqueado por CAPTCHA |
| LinkedIn | `promo/drafts/linkedin-viral-gif.md` | GIF comparativo | listo; bloqueado por sesión real |
| Reddit r/mcp | `promo/drafts/reddit-rmcp-v2.md` | — | listo; bloqueado por sesión real |
| Reddit r/selfhosted | `promo/drafts/reddit-selfhosted-v2.md` | — | listo; bloqueado por sesión real |
| Reddit r/rust | `promo/drafts/reddit-rust-v2.md` | — | listo; bloqueado por sesión real |
| HN (estudio) | `promo/drafts/show-hn-study.md` | link a `bench/study.md` | listo; bloqueado por sesión real |
| dev.to (estudio) | `promo/drafts/devto-bot-detection-study.md` | — | listo; requiere cuenta dev.to |

## Outreach (listo para enviar)

| tipo | archivo |
|---|---|
| Influencers Tier 1 | `promo/drafts/outreach-tier1.md` |
| Campaign tracker | `promo/drafts/outreach-campaign-tracker.md` |
| Newsletter pitches | `promo/drafts/newsletter-pitches.md` |
| Press kit | `promo/drafts/press-kit.md` |

## Distribución (listo para submit)

| canal | archivo | bloqueo |
|---|---|---|
| BetaList | `promo/drafts/betalist-submission.md` | cuenta pendiente |
| Directorios varios | `promo/drafts/directory-submissions-pack.md` | cuentas |
| PR awesome-mcp-servers | PR #12089 | merge pendiente |
| mcp.so | issue #3546 | revisión pendiente |
| MCP Registry oficial | `promo/drafts/registry-publish.md` | OAuth usuario |

## Product Hunt

| asset | ubicación |
|---|---|
| Ficha completa | `promo/drafts/producthunt.md` |
| Schedule launch day | `promo/drafts/producthunt-launch-day.md` |
| GIF | `promo/assets/neobrowser-vs-headless/neobrowser-vs-headless.gif` |

## Producto/docs

| asset | ubicación |
|---|---|
| Guía clientes MCP | `promo/drafts/mcp-clients-guide.md` |
| Storyboard video | `promo/drafts/demo-video-storyboard.md` |
| Fixes CI PR #7 | worktree `/tmp/neobrowser-pr7` |

## Inteligencia

| reporte | ubicación |
|---|---|
| Competencia | `promo/drafts/intelligence-report-2026-08-19.md` |

---

## Plan de lanzamiento coordinado (48h)

**Día -1:**
- Publicar HN "honest table" por la mañana ET.
- Publicar dev.to artículo del estudio.

**Día 0 (Product Hunt):**
- 00:01 PT / 09:01 CET: Product Hunt live.
- 09:15 CET: X + LinkedIn.
- 09:30-13:00 CET: responder comentarios PH en tiempo real.
- 11:00-13:00 CET: Reddit r/mcp + r/selfhosted.

**Día +1:**
- Agradecimiento en redes.
- Outreach a 1-2 influencers con el momentum del launch.
- Submissions a directorios secundarios.

---

## Bloqueos que requieren acción del usuario

1. **X CAPTCHA** — resolver manualmente en `x.com/account/access`.
2. **LinkedIn/Reddit/PH sesiones** — ejecutar `python3 promo/scripts/attach_mode_helper.py` o cerrar Chrome para `cold_profile_mirror.py`.
3. **MCP Registry OAuth** — `mcp-publisher login github && mcp-publisher publish`.
4. **PR #7 fixes** — autorizar git push desde `/tmp/neobrowser-pr7`.
5. **Product Hunt cuenta** — confirmar login y lanzamiento martes 26 00:01 PT.
