# Plan: Browser Foundations

## El problema

Estamos parcheando webs individuales en vez de construir un browser.
Un browser tiene fundamentos que hacen que TODO funcione.

## Fundamento 1: Cookie Container unificado

Ahora: 3 cookie jars separados que no se hablan.
- `rquest::Client::cookie_store` — HTTP level
- `ghost::CookieJar` — manual, inyectado como header
- `document.cookie` en linkedom — JS level, no conectado

Un browser real tiene UN cookie jar:
- HTTP response `Set-Cookie` → jar
- JS `document.cookie = "..."` → jar
- HTTP request → jar envía cookies matching
- Todo sincronizado, todo el rato

### Implementación
```
UnifiedCookieJar {
    // Source of truth: SQLite (domain, name, value, path, secure, httponly, expires)
    db: rusqlite::Connection,

    // On Set-Cookie header → write to DB
    store_from_header(domain, header)

    // On document.cookie set → write to DB (via V8 op)
    store_from_js(domain, name, value)

    // On HTTP request → read from DB, build Cookie header
    header_for(url) → String

    // On document.cookie get → read from DB (via V8 op)
    get_for_js(domain) → String
}
```

V8 ops:
- `op_cookie_get()` → lee del UnifiedCookieJar
- `op_cookie_set(value)` → escribe al UnifiedCookieJar

bootstrap.js:
```javascript
Object.defineProperty(document, 'cookie', {
    get() { return Deno.core.ops.op_cookie_get(); },
    set(val) { Deno.core.ops.op_cookie_set(val); }
});
```

Resultado: cuando Google setea SOCS via Set-Cookie, JS lo puede leer.
Cuando JS setea una cookie, la siguiente HTTP request la envía.

## Fundamento 2: Navigation como un browser

Ahora: goto() hace un fetch HTTP aparte del DOM.
El DOM vive en V8, el HTTP vive en Rust. No se hablan.

Un browser real:
- Click `<a href="/page2">` → HTTP GET /page2 → Set-Cookie procesado → DOM reemplazado
- Submit `<form method="POST">` → HTTP POST → Set-Cookie → DOM reemplazado
- `window.location = "/page3"` → HTTP GET → Set-Cookie → DOM reemplazado
- Redirect 302 → sigue automáticamente con cookies

### Implementación
Todas las navegaciones pasan por el mismo pipeline:
```
navigate(url, method, body, headers) {
    1. HTTP request (rquest client con cookies del jar)
    2. Procesar Set-Cookie → UnifiedCookieJar
    3. Si 3xx redirect → navigate() recursivo
    4. Parsear HTML → linkedom
    5. Ejecutar scripts
    6. Syncronizar cookies JS↔jar
    7. Auto-consent si detectado
    8. Extraer WOM
}
```

click("Aceptar") que dispara un form submit:
1. JS event → form submit → collect FormData
2. → navigate(action, POST, formdata)
3. → cookies actualizadas
4. → página nueva renderizada

## Fundamento 3: Test Battery (no per-site, per-feature)

### Cookie tests
```
test_cookie_from_http:
    navigate to httpbin.org/cookies/set?name=value
    assert document.cookie contains "name=value"
    navigate to httpbin.org/cookies
    assert response contains "name=value"

test_cookie_from_js:
    navigate to any page
    eval: document.cookie = "test=123"
    navigate to same domain
    assert Cookie header contains "test=123"

test_cookie_httponly:
    navigate to site that sets HttpOnly cookie
    assert document.cookie does NOT contain it
    assert next HTTP request DOES contain it

test_cookie_persistence:
    navigate to site, get cookies
    create new session with same cookie file
    navigate again
    assert cookies still present
```

### Navigation tests
```
test_link_click:
    navigate to page with links
    click("link text")
    assert URL changed
    assert new page content visible

test_form_submit_get:
    navigate to page with form
    type("input_name", "value")
    submit()
    assert URL contains ?input_name=value
    assert response page rendered

test_form_submit_post:
    navigate to page with POST form
    type fields
    submit()
    assert new page rendered (not the form)

test_redirect:
    navigate to URL that 302 redirects
    assert final URL is the redirect target
    assert cookies from redirect are stored

test_consent_flow:
    navigate to google.es (shows consent)
    assert consent auto-accepted
    assert SOCS cookie set
    navigate to google.es/search?q=test
    assert search results visible (not blocked)
```

### Interaction tests
```
test_type_input:
    navigate to page with input
    type("input_name", "text")
    assert input.value === "text"
    assert input event fired

test_click_button:
    navigate to page with button
    click("button text")
    assert click event fired

test_select_option:
    navigate to page with select
    select("select_name", "option_value")
    assert select.value === "option_value"
```

### Extraction tests
```
test_extract_tables:
    navigate to Wikipedia article
    extract_tables()
    assert at least 1 table with headers and rows

test_extract_article:
    navigate to news article
    extract_article()
    assert title, body non-empty

test_dom_tree:
    navigate to any page
    dom_tree(3)
    assert JSON tree with tag, children, text
```

## Prioridad

1. **UnifiedCookieJar** — esto desbloquea Google consent y todas las webs con cookies
2. **Navigation pipeline** — click/submit/redirect pasan por el mismo pipeline
3. **Test battery** — tests automáticos que verifican que el browser funciona

## Archivos a crear/modificar

```
src/neorender/
  cookie_jar.rs      — NEW: UnifiedCookieJar (SQLite, HTTP sync, JS sync)
  session.rs         — MODIFY: usar UnifiedCookieJar, pipeline de navegación
  ops.rs             — MODIFY: op_cookie_get/set

js/
  bootstrap.js       — MODIFY: document.cookie → ops

tests/
  test_browser.rs    — NEW: test battery
```
