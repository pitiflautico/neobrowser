# MVP v1.0 Test Battery — Browser for AI

## Criterio de éxito: TODOS estos tests deben pasar para v1.0

### A. Navegación (12 tests)
| # | Test | URL | Assertion |
|---|------|-----|-----------|
| A1 | GET básico | httpbin.org/html | text contiene "Herman Melville" |
| A2 | HTTPS | https://example.com | status 200, title "Example Domain" |
| A3 | Redirect 301 | httpbin.org/redirect/1 | URL final ≠ URL original |
| A4 | Redirect 302 | httpbin.org/redirect-to?url=/html | llega a /html |
| A5 | 404 | httpbin.org/status/404 | status 404 o error detectado |
| A6 | Página grande | wikipedia.org/wiki/Spain | html_bytes > 500KB |
| A7 | Navegación secuencial | HN → HN/newest | 2ª nav < 1ª en tiempo |
| A8 | Cross-domain | HN → Wikipedia | ambos con contenido |
| A9 | Query params | google.es/search?q=test | "test" visible en WOM |
| A10 | Hash fragment | wikipedia.org/wiki/Spain#History | URL contiene #History |
| A11 | Location eval | cualquier page → eval location.href | URL coincide |
| A12 | Timeout | URL que no responde | error, no hang infinito |

### B. Cookies y Sesiones (10 tests)
| # | Test | Assertion |
|---|------|-----------|
| B1 | Set-Cookie HTTP → JS visible | httpbin.org/cookies/set → document.cookie tiene la cookie |
| B2 | Cookie persiste entre navs | misma cookie visible después de goto() |
| B3 | document.cookie = "x=1" | visible en siguiente eval |
| B4 | Múltiples cookies | 3+ cookies simultáneas |
| B5 | Cookie de archivo | load from /tmp/*-state.json → sesión funciona |
| B6 | LinkedIn auth | feed con "notificaciones" |
| B7 | Amazon auth | home con "Hola Daniel" |
| B8 | Google con consent cookies | search results visibles |
| B9 | Cookie domain matching | .google.es → www.google.es |
| B10 | Session cache | ~/.neobrowser/sessions/ tiene archivos |

### C. Interacción (10 tests)
| # | Test | Assertion |
|---|------|-----------|
| C1 | type(name, text) | valor asignado al input correcto |
| C2 | type(placeholder, text) | encontrado por placeholder |
| C3 | type(selector, text) | CSS selector funciona |
| C4 | click(exact text) | matchea el link exacto, no parcial |
| C5 | click(link) → navega | nueva URL, nuevo contenido |
| C6 | click(button) | evento disparado |
| C7 | submit GET form | URL tiene query params |
| C8 | submit POST form | respuesta del servidor renderizada |
| C9 | select dropdown | valor cambiado |
| C10 | type + submit combo | Google: type "test" → submit → resultados |

### D. JavaScript (8 tests)
| # | Test | Assertion |
|---|------|-----------|
| D1 | eval "1+1" | resultado "2" |
| D2 | eval document.title | título correcto |
| D3 | eval async (fetch) | Promise resuelta |
| D4 | ES Module loaded | módulo de la page ejecutó |
| D5 | setTimeout ejecuta | callback se llamó |
| D6 | DOM manipulation | createElement + appendChild funciona |
| D7 | Event listener | addEventListener + dispatchEvent |
| D8 | Error no crashea | script roto no mata la sesión |

### E. Extracción (10 tests)
| # | Test | URL | Assertion |
|---|------|-----|-----------|
| E1 | WOM text | HN | text no vacío, contiene "Hacker News" |
| E2 | WOM links | HN | links > 100 |
| E3 | WOM buttons | Google | buttons > 0 |
| E4 | extract_tables | Wikipedia/Spain | ≥ 10 tables con rows |
| E5 | extract_article | Wikipedia/Spain | title="Spain", body > 500 chars |
| E6 | extract_form_schema | Google | form con campo "q" |
| E7 | extract_structured | Wikipedia | JSON-LD presente |
| E8 | dom_tree(3) | HN | JSON válido con tag, children |
| E9 | Next.js __NEXT_DATA__ | notion.so o bbc.com | contenido extraído |
| E10 | Page classify | Google search → "search_results", Wikipedia → "article" |

### F. Stealth (6 tests)
| # | Test | Assertion |
|---|------|-----------|
| F1 | navigator.webdriver | false |
| F2 | navigator.plugins.length | > 0 |
| F3 | screen.width | número realista (1920) |
| F4 | typeof chrome | "object" |
| F5 | TLS fingerprint | Amazon/SO no bloquean (Chrome TLS) |
| F6 | No navigator.webdriver flag | ni undefined ni true |

### G. Rendimiento (5 tests)
| # | Test | Assertion |
|---|------|-----------|
| G1 | HN render < 3s | primera navegación |
| G2 | Session reuse < 1s | segunda navegación misma sesión |
| G3 | Resource filter | analytics/CSS skipped (log muestra "Skipped") |
| G4 | Compress output < 2KB | __neo_compress(2000) devuelve < 2KB |
| G5 | WOM extraction < 100ms | después de render, WOM rápido |

### H. Edge Cases (5 tests)
| # | Test | Assertion |
|---|------|-----------|
| H1 | Consent auto-accept | Google consent → clickeado automáticamente |
| H2 | WAF detection | Amazon sin cookies → "WAF challenge" en error |
| H3 | Rate limit | 10 requests rápidos → no crash |
| H4 | Empty page | about:blank → no crash |
| H5 | Error info | 404 → ErrorInfo con sugerencias |

### I. Sites Reales Complejos (14 tests)
| # | Site | Cookie file | Assertion |
|---|------|-------------|-----------|
| I1 | news.ycombinator.com | — | L>100, text>100 chars |
| I2 | google.es/search?q=test | /tmp/google-state.json | L>10, resultados visibles |
| I3 | reddit.com | — | L>10, contenido |
| I4 | en.wikipedia.org/wiki/Spain | — | L>1000, 10+ tables |
| I5 | stackoverflow.com/questions | — | L>50, questions |
| I6 | linkedin.com/feed | /tmp/linkedin-fresh.json | "notificaciones" |
| I7 | amazon.es | /tmp/amazon-state.json | L>50, authenticated |
| I8 | chatgpt.com | /tmp/chatgpt-state.json | "Chat history" |
| I9 | nytimes.com | — | articles, L>50 |
| I10 | bbc.com | — | content (Next.js) |
| I11 | apple.com | — | products, L>50 |
| I12 | elpais.com | — | noticias, fecha actual |
| I13 | instagram.com | — | "Log into Instagram" |
| I14 | netflix.com | — | "Unlimited movies" |

---

## Total: 80 tests

Para v1.0, objetivo: **≥ 70/80 PASS (87.5%)**

Tests que pueden fallar y aún es MVP:
- ChatGPT conversation (Turnstile)
- Facebook/Twitch (React CSR)
- Redirect edge cases
- Some cookie edge cases

Tests que DEBEN pasar para v1.0:
- Toda la navegación (A1-A11)
- Cookies básicas (B1-B5)
- Auth sites (B6-B8)
- Toda la interacción (C1-C10)
- JS básico (D1-D8)
- Toda la extracción (E1-E10)
- Stealth (F1-F6)
- Sites reales principales (I1-I8)
