# Log de acciones

## 2026-08-13 — ciclo 0 (setup)
- Repo metadata: descripción, homepage (landing Pages), 12 topics añadidos.
- Workspace promo/ creado (estrategia, backlog, métricas).
- Cron de promoción programado 2×/día (id d77462a9).
- Release v0.1.3: workflow OK (binarios multiplataforma publicados), CI verde, landing desplegada en Pages.
- PR abierto a punkpeye/awesome-mcp-servers: https://github.com/punkpeye/awesome-mcp-servers/pull/12089 (sección Browser Automation, fast-track 🤖). Pendiente de merge.

## 2026-08-13 — ciclo 1
- Métricas: 0★ / 0 forks (sin cambio aún — esperado, PR #12089 aún OPEN).
- appcypher/awesome-mcp-servers: **archivado**, no acepta PRs. Descartado.
- wong2/awesome-mcp-servers: **PRs desactivados** (404 en /pulls). Descartado. El ecosistema awesome-list se ha consolidado en punkpeye — buena noticia: un solo merge cubre el canal.
- glama.ai: NeoBrowser NO indexado (solo hay "NexBrowser", homónimo ajeno). Pendiente; los topics nuevos pueden disparar auto-indexación.
- README: badges de instalación 1-click para VS Code y Cursor (commit cbaf80b, pusheado).
## 2026-08-13 — ciclo 3
- PR #12089 (punkpeye): sigue OPEN sin merge. Estrellas: 0.
- mcp.so: submission creada como issue en chatmcp/mcpso (#3546), formato estándar del directorio.
- Borrador Show HN completo en promo/drafts/show-hn.md (2 títulos, texto, respuestas preparadas a objeciones típicas).
- Investigación de directorios: el MCP Registry oficial es el canal canónico 2026 (alimenta downstream); sigue bloqueado por OAuth interactivo del usuario.

## 2026-08-13 — ciclo 2
- demo.gif generado: grabación asciinema (venv /tmp/promo-tools) del demo con wrapper streaming (/tmp/demo_live.py, mismos pasos MCP que rust/scripts/demo.py), convertido a GIF con pyte+Pillow (/tmp/cast2gif.py). 14 frames, 931x448, 89 KB. Embebido en README.md y docs/index.html.

## 2026-08-13 — ciclo 4
- PR #12089 y issue mcp.so #3546: ambos siguen OPEN. Estrellas: 0.
- Borradores listos para el usuario: r/mcp, hilo Twitter/X (con demo.gif adjunto), LinkedIn (ES, tono first-person).

## 2026-08-13 — ciclo 5
- PulseMCP: no estamos listados; **submissions pausadas hasta mediados de agosto** (rework de su ingestion). Reintentar en próximos ciclos. Bonus: la comprobación se hizo con el propio NeoBrowser — curl recibía 403 de Cloudflare, NeoBrowser pasó, detectó el captcha y leyó la página. Dogfooding + anécdota de marketing.
- Smithery: no estamos listados; el submit requiere cuenta/CLI auth → pendiente credenciales del usuario.
- Mantenimiento: actualizado el binario instalado ~/.local/bin/neobrowser a v0.1.3 (había un server viejo vivo de otra sesión con un Chrome fugado; limpiado). Ojo macOS: al copiar el binario a mano, quitar el xattr com.apple.provenance o Gatekeeper lo mata (SIGKILL) en el primer exec.

## 2026-08-14 — ciclo 6
- MCP Registry oficial: `server.json` creado en la raíz y **validado contra el schema oficial 2025-12-11** (variante websiteUrl, descripción 89 chars). Instrucciones exactas para el usuario en `promo/drafts/registry-publish.md` — solo falta `mcp-publisher login github` (OAuth interactivo).
- glama.ai: sigue sin indexar (solo el homónimo NexBrowser). Tener server.json en la raíz puede ayudar a su crawler; re-check en próximos ciclos.
- Estrellas: 0. PR #12089 y issue mcp.so #3546 siguen OPEN.

## 2026-08-14 — ciclo 7
- Borrador artículo dev.to (`promo/drafts/devto.md`): ángulo "benchmark honesto vs Playwright MCP", con front matter listo y nota sobre la API de dev.to (si el usuario pasa DEV API key, el agente puede publicar drafts).
- Prep completa de Product Hunt (`promo/drafts/producthunt.md`): tagline, descripción, galería, maker comment y checklist del día de launch.
- Estrellas: 0. PR #12089 y mcp.so #3546 siguen OPEN.

## 2026-08-14 — ciclo 8
- Monitor de menciones: 0 reales (la única en HN es un artículo de 2022 sobre "neobrowsers" como categoría, sin relación). Estrellas: 0; PR #12089 OPEN.
- Newsletters: research hecho — no hay forms públicos de submission relevantes que no requieran cuenta/email del usuario; canal aplazado a cuando haya tracción que enseñar.
- Asset nuevo: `docs/assets/og.png` (1200×630, captura real de la landing hecha con el propio NeoBrowser) + meta tags og:image/twitter:card en la landing — los links compartidos en X/LinkedIn/PH ahora salen con imagen.

## 2026-08-14 — ciclo 9
- PulseMCP re-check (con NeoBrowser): submissions SIGUEN pausadas. Reintentar próximos ciclos.
- Nuevo contenido: `promo/drafts/tech-cookies.md` — deep-dive "How to decrypt Chrome cookies on macOS/Linux/Windows without being evil", verificado contra cookies.rs real. Para publicar 1-2 semanas después del primer artículo.
- Estrellas: 0. PR #12089 y mcp.so #3546 OPEN.

## 2026-08-14 — ciclo 10
- Nuevo contenido: `promo/drafts/tech-cdp.md` — deep-dive "Multiplexing CDP in Rust: one reader, zero races", verificado contra cdp.rs real. Cierra la serie técnica (benchmark → cookies → CDP).
- Estrellas: 0. PR #12089 y mcp.so #3546 OPEN.
- El backlog autónomo de contenido queda agotado; próximos ciclos = mantenimiento (re-checks directorios, monitor, mejoras de assets si hacen falta).

## 2026-08-14 — ciclo 11
- Nuevo asset: `docs/assets/architecture.png` (diagrama AI client → NeoBrowser → real Chrome → web, dark theme acorde a la landing), embebido en la landing y referenciado como imagen 5 de la galería de Product Hunt. Todos los assets de PH quedan completos.
- Estrellas: 0. PR #12089 y mcp.so #3546 OPEN.

## 2026-08-14 — ciclo 12
- PR #12089: el bot glama-check pidió requisito nuevo — listing en glama.ai + badge de score en la entrada. Hecho: `glama.json` añadido a la raíz del repo (mecanismo de auto-descubrimiento de glama, ~24h de crawl) y badge añadido a la entrada del PR (commit 514a4e8 en el fork, PR actualizado automáticamente). El badge dará 404 hasta que glama nos indexe.
- Pendiente en glama: para que sus checks pasen piden añadir un Dockerfile "directly to Glama" — evaluar cuando aparezca el listing.
- glama: sigue sin indexar (esperado, crawl ~24h). Estrellas: 0. mcp.so #3546 OPEN.

## 2026-08-14 — ciclo 13 (mantenimiento)
- Sin movimiento: 0★, PR #12089 OPEN, mcp.so #3546 OPEN, glama aún sin indexar (crawl ~24h desde el glama.json; re-check en próximos ciclos).

## 2026-08-14 — ciclo 14: PRIMER POST REAL PUBLICADO (dogfooding total)
- El usuario autorizó usar sus sesiones reales. Mapeo de perfiles Chrome: la sesión viva de X está en "Profile 24" (@perez_pina28188). HN: sin login en ningún perfil. LinkedIn (P3/P15/P24) y Reddit (P24): sesiones caducadas → requieren re-login manual del usuario.
- **Publicado en X con NeoBrowser itself + sesión real** (Profile 24, NEOBROWSER_HOME=/tmp/nbpromo): post de 264 chars verificado en el perfil con tarjeta de GitHub. Flujo: navigate → find/click textbox (backend_node_id) → type human=true → click Post → redirect a home → verificado en @perez_pina28188.
- Aprendizaje de tooling: `click` usa backend_node_id/selector (no intent); los drafts del composer no sobreviven reload — todo el flujo en una sola sesión de servidor.
- Pendiente: hilo completo de X (replies 2-5 de drafts/twitter.md) en próximos ciclos para no gatillar anti-bot; HN/LinkedIn/Reddit esperan re-login del usuario.

## 2026-08-14 — ciclo 15: HILO DE X COMPLETO
- Hilo publicado y verificado: post principal + 5 replies encadenados (stealth genuino, sesiones opt-in, honestidad ante muros, benchmark, y cierre dogfooding "este hilo lo publicó NeoBrowser mismo"). 6 tweets en la conversación: https://x.com/perez_pina28188/status/2087972229699043609
- Aprendizaje tooling: las replies NO aparecen en la tab Posts del perfil (hay que verificar en la conversación); el modal de reply wedga la página — usar la caja inline + tweetButtonInline.
- Estado cuentas: LinkedIn bloqueado por diseño propio (li_at/JSESSIONID excluidos, cookies.rs:159) → attach mode cuando Chrome arranque con --remote-debugging-port=9222. HN y Reddit: el usuario no tiene cuenta en ninguna — se le ha pedido crearlas manualmente (Reddit tiene reCAPTCHA en signup; HN conviene cuenta con algo de rodaje). Yo NO tengo acceso a sus credenciales ni debo sacarlas del almacén de Chrome.

## 2026-08-14 — ciclo 16/17: la máquina de hype
- Engagement del hilo X: 18 views, 2 replies en el principal (primeras horas, cuenta pequeña — normal).
- PLAYBOOK.md creado: 5 estrategias (stunts de dogfooding recurrentes, outreach a figuras con replies de valor, cadencia de contenido, engagement comunitario, launches secuenciados) con reglas anti-spam intactas.
- targets.md: tier 1 (simonw, swyx, t3dotgg, mitsuhiko, Anthropic, levelsio) + tier 2, con ángulos por cuenta.
- Cron actualizado: d77462a9 eliminado → 7c2c75a4 (3×/día: 9:23, 14:23, 20:23) con prompt de hype que incluye el playbook, targets y el flujo X probado.

## 2026-08-13 — dogfooding: alta real en FWA → 4 bugs encontrados y arreglados
- Stunt de dogfooding: dar de alta moneyincheck.org en thefwa.com de punta a punta con NeoBrowser. Enviado (case 19085, £0 con voucher del 100%). El formulario React de 4 pasos destapó 4 bugs, los 4 con la misma raíz: **la tool reportaba éxito por despachar la acción, sin comprobar el efecto**.
- `find_and_click` clicaba nodos invisibles: cogía el primer match textual aunque estuviera en un acordeón colapsado, así que todos los "Continue" iban al paso cerrado y el paso abierto no se enviaba nunca. Provocó un diagnóstico entero en falso (culpé al voucher del usuario). Ahora filtra por visibilidad, devuelve `matched_total`/`matched_visible` y usa el click de ratón real (antes era `.click()` de JS, contradiciendo la promesa de isTrusted del propio MCP).
- `click` no hacía scroll ni verificaba impacto: `"Clicked"` con el target fuera del viewport o bajo un banner de cookies. Ahora `scrollIntoViewIfNeeded` → relee la caja → hit-test con `DOM.getNodeForLocation` → enum `ClickOutcome` con `Obscured{by}` que nombra al elemento que tapa.
- `SingletonLock` huérfano mataba el arranque con error mudo (stderr iba a /dev/null). Ahora se limpia solo si el PID está muerto, y el stderr de Chrome (`~/.neobrowser/logs/chrome-{port}.log`) viaja dentro del error.
- `login` daba falso negativo en páginas con campos de cambio de contraseña, y submitía el primer form del documento (el panel del header) en vez del que contiene el password.
- Cobertura nueva: `rust/tests/multistep_forms.rs`, 7 tests herméticos (fixture `data:` URL) que comprueban **efectos**, no valores de retorno. Total 94 en verde, clippy limpio. Doc: `docs/BUGS-formularios-multipaso.md`.
- v0.1.4. Descripciones de `click`/`find_and_click` y la guía del server actualizadas: el contrato nuevo hay que contarlo al agente, no solo implementarlo.
