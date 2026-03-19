# PDR: NeoRender — Browser for AI

## Vision

Un browser REAL sin capa gráfica. Hace todo lo que Chrome hace excepto renderizar píxeles. Output = WOM (mapa semántico para IA). La IA navega como un humano: va a webs, lee, hace click, rellena formularios, envía, lee la respuesta.

No es scraping. No es bypass. Es un browser legítimo que habla con webs normalmente — solo que su usuario es una IA en vez de un humano.

## Qué es un browser

| Capa | Chrome | NeoRender | Status |
|------|--------|-----------|--------|
| **Networking** | Chromium net stack | rquest (BoringSSL Chrome131) | ✅ |
| **TLS** | BoringSSL | BoringSSL (misma lib) | ✅ |
| **HTTP/2** | nghttp2 | rquest (hyper) | ✅ |
| **Cookies** | Cookie jar + SQLite | rquest cookie_store + CookieJar | ✅ |
| **DNS** | System resolver | System resolver (vía tokio) | ✅ |
| **HTML Parser** | Blink HTML parser | linkedom (spec-compliant) | ✅ |
| **DOM** | Blink DOM | linkedom DOM | ✅ |
| **JavaScript** | V8 | V8 (misma engine, vía deno_core) | ✅ |
| **ES Modules** | V8 module loader | deno_core module loader | ✅ |
| **CSS Parser** | Blink CSS | ❌ No (no hay layout) | N/A |
| **Layout** | Blink layout engine | ❌ No (no hay píxeles) | N/A |
| **Rendering** | Skia/GPU | ❌ No (output = WOM) | N/A |
| **Events** | Blink event system | linkedom events | ⚠️ Parcial |
| **Forms** | HTML forms + submit | ❌ Falta | **TODO** |
| **Navigation** | Blink navigation | goto() persistente | ✅ |
| **Click** | Input → event dispatch | ❌ Falta | **TODO** |
| **Type** | Input → event dispatch | ❌ Falta | **TODO** |
| **Scroll** | Layout-based | N/A (no layout) | N/A |
| **iframes** | Blink frame tree | ❌ Falta | **TODO** |
| **Web Workers** | V8 isolates | ❌ Falta | **TODO** |
| **Service Workers** | V8 + cache API | ❌ Falta | Bajo prio |
| **localStorage** | LevelDB | SQLite | ✅ |
| **sessionStorage** | Memory | Memory | ✅ |
| **Fetch API** | Blink fetch | rquest + Sec-Fetch-* | ✅ |
| **XMLHttpRequest** | Blink XHR | JS polyfill → fetch | ✅ |
| **WebSocket** | Chromium WS | ❌ Stub | **TODO** |
| **Crypto** | BoringSSL | SHA-256 nativo + stubs | ⚠️ Parcial |
| **Canvas** | Skia | Stub (no-op) | N/A |
| **Consent/GDPR** | User clicks | Auto-accept | **TODO** |

## Lo que NO es

- No es scraping (navega normalmente, ejecuta JS, respeta robots.txt)
- No es bypass de seguridad (usa las mismas libs que Chrome)
- No es headless Chrome (no usa Chrome — es un browser independiente)
- No elude captchas (si hay captcha, pide ayuda al humano)

## Arquitectura

```
┌─────────────────────────────────────────────────┐
│                 NeoSession                       │
│     (persistent browser session for AI)          │
├──────────┬──────────┬──────────┬────────────────┤
│ net/     │ dom/     │ js/      │ interact/      │
│          │          │          │                │
│ rquest   │ linkedom │ V8      │ click(sel)     │
│ Chrome   │ HTML     │ deno    │ type(sel,txt)  │
│ TLS      │ parser   │ core    │ submit(form)   │
│          │          │          │ select(opt)    │
│ Fetch    │ DOM tree │ ES      │ check(box)     │
│ Standard │ Events   │ Modules │ hover(sel)     │
│ CORS     │ Forms    │ Timers  │ focus(sel)     │
│ Cookies  │ WOM gen  │ Fetch   │                │
│          │          │ Storage │                │
├──────────┴──────────┴──────────┴────────────────┤
│                    output/                       │
│                                                  │
│  WOM = { text, links, forms, buttons, inputs,   │
│          headings, images, meta, tables }        │
│                                                  │
│  "AI-friendly page map — what a user sees,       │
│   but structured for machine consumption"        │
└──────────────────────────────────────────────────┘
```

## Completado

### Phase 1: Networking ✅
- `net/mod.rs`: BrowserNetwork con Fetch Standard
- `net/headers.rs`: Sec-Fetch-Site/Mode/Dest
- `net/referrer.rs`: 4 referrer policies
- rquest Chrome131 TLS (BoringSSL)
- Cookie store automático en redirects

### Phase 2: WOM from linkedom ✅
- `js/wom.js`: __wom_extract() — walk DOM in V8
- Extrae text, links, forms, buttons, headings, meta, images
- Sin re-parse html5ever

### Phase 3: Storage ✅
- `storage.rs`: SQLite-backed localStorage
- Per-domain namespace
- 4 V8 ops (get/set/remove/clear)

### Phase 4: Web APIs ✅
- ReadableStream (controller, tee, pipeTo, asyncIterator)
- WritableStream, TransformStream
- MessageChannel + MessagePort (async message passing)
- TextEncoderStream / TextDecoderStream
- SubtleCrypto (SHA-256 nativo)
- 50+ polyfills (Canvas, WebSocket, Range, Selection, etc.)

### Phase 5: Error isolation ✅
- Scripts wrapped in try-catch
- onerror + onunhandledrejection handlers
- Analytics/telemetry auto-skip

## Pendiente

### Phase 6: Interaction (PRÓXIMO)

La pieza que convierte el lector en browser. La IA puede navegar pero no interactuar.

**`interact/mod.rs`** + **`js/interact.js`**

```rust
impl NeoSession {
    /// Click an element by CSS selector or text content
    async fn click(&mut self, target: &str) -> Result<(), String>;

    /// Type text into an input/textarea
    async fn type_text(&mut self, target: &str, text: &str) -> Result<(), String>;

    /// Submit a form (by selector or auto-detect)
    async fn submit(&mut self, target: &str) -> Result<(), String>;

    /// Select an option in a <select>
    async fn select(&mut self, target: &str, value: &str) -> Result<(), String>;

    /// Check/uncheck a checkbox
    async fn check(&mut self, target: &str, checked: bool) -> Result<(), String>;
}
```

Internamente:
1. `click(target)` → find element → dispatch mousedown/mouseup/click events → si es `<a>`, goto(href) → si es `<button type=submit>`, submit form
2. `type_text(target, text)` → find element → focus → dispatch input/change events → set value
3. `submit(form)` → collect form data → POST to action URL → goto response
4. Si click dispara navegación → goto() automático

```javascript
// js/interact.js
globalThis.__neo_click = function(selector) {
    const el = document.querySelector(selector)
        || [...document.querySelectorAll('*')].find(e => e.textContent?.trim() === selector);
    if (!el) return JSON.stringify({ok:false, error:'not found'});

    // Dispatch full event sequence (what a real browser does)
    el.dispatchEvent(new MouseEvent('mousedown', {bubbles:true}));
    el.dispatchEvent(new MouseEvent('mouseup', {bubbles:true}));
    el.dispatchEvent(new MouseEvent('click', {bubbles:true}));
    el.click?.();

    // If it's a link, return the href for navigation
    const href = el.closest('a')?.getAttribute('href');
    if (href) return JSON.stringify({ok:true, navigate:href});

    // If it's a submit button, collect form data
    const form = el.closest('form');
    if (form && (el.type === 'submit' || el.tagName === 'BUTTON')) {
        return JSON.stringify({ok:true, submit:{action:form.action, method:form.method}});
    }

    return JSON.stringify({ok:true, clicked:el.tagName});
};

globalThis.__neo_type = function(selector, text) {
    const el = document.querySelector(selector)
        || document.querySelector(`[placeholder*="${selector}" i]`)
        || document.querySelector(`[name="${selector}"]`);
    if (!el) return JSON.stringify({ok:false, error:'not found'});

    el.focus?.();
    el.value = text;
    el.dispatchEvent(new Event('input', {bubbles:true}));
    el.dispatchEvent(new Event('change', {bubbles:true}));

    return JSON.stringify({ok:true, typed:text.length});
};

globalThis.__neo_submit = function(selector) {
    const form = document.querySelector(selector || 'form');
    if (!form) return JSON.stringify({ok:false, error:'no form'});

    const data = {};
    for (const el of form.querySelectorAll('input,select,textarea')) {
        const name = el.name || el.id;
        if (name) data[name] = el.value || '';
    }

    return JSON.stringify({
        ok:true,
        action: form.action || location.href,
        method: (form.method || 'GET').toUpperCase(),
        data
    });
};
```

**Flujo completo de interacción:**
```
IA: goto("https://google.es")
    → WOM: {forms:[{action:"/search", fields:["q"]}], buttons:["Aceptar todo"]}

IA: click("Aceptar todo")
    → consent accepted, cookie set

IA: type("q", "restaurantes la eliana")
    → input filled

IA: submit("form")
    → POST/GET to /search?q=restaurantes+la+eliana
    → auto-goto response
    → WOM: {links:[...restaurants...], text:"10 resultados"}
```

### Phase 7: iframes

Muchas webs usan iframes (Turnstile, ads, embeds). Necesario para:
- Google reCAPTCHA / Cloudflare Turnstile
- Payment forms (Stripe)
- OAuth popups
- YouTube embeds

Implementación: cada iframe = mini NeoSession con su propio document + postMessage bridge.

### Phase 8: WebSocket

Para apps en tiempo real:
- Chat (Slack, Discord)
- Notifications
- Live updates

Implementación: rquest WebSocket client expuesto como JS WebSocket API.

### Phase 9: Consent auto-accept

Patrones GDPR comunes:
- Cookie banners → detect + click accept
- Google consent → set CONSENT cookie
- Generic: buscar botones con "Accept", "Aceptar", "OK", "Agree"

Implementación: después de cada goto(), scan para consent patterns → auto-click.

## Success Metrics

Un browser para IA que puede:
1. ✅ Navegar a cualquier web (18/20 top sites)
2. ✅ Leer contenido (WOM)
3. ⬜ Hacer click en links/buttons
4. ⬜ Rellenar y enviar formularios
5. ⬜ Aceptar consent dialogs automáticamente
6. ⬜ Buscar en Google sin Chrome
7. ⬜ Enviar mensajes en ChatGPT sin Chrome
8. ⬜ Login en webs con user/password

Sin Chrome para el 95% de las operaciones. Chrome solo para captchas irresolubles.
