# PDR v4: Browser Optimizado para IA

## Principio

Un browser para IA no renderiza — COMPRENDE.
No descarga lo que no necesita. Devuelve datos, no HTML.

## Optimizaciones que un browser humano no tiene

### 1. Skip resources que la IA no necesita
- NO descargar imágenes (img src → solo extraer alt text + URL)
- NO descargar CSS (no hay layout)
- NO descargar fonts
- NO descargar videos/audio
- NO ejecutar analytics/tracking scripts
- NO ejecutar ads scripts
- SÍ descargar: HTML, JS de la app, JSON APIs

**Implementación**: en `extract_all_scripts()`, filtrar por URL:
```rust
fn should_skip_resource(url: &str) -> bool {
    url.ends_with(".css") || url.ends_with(".png") || url.ends_with(".jpg") ||
    url.ends_with(".gif") || url.ends_with(".svg") || url.ends_with(".woff") ||
    url.ends_with(".mp4") || url.ends_with(".webm") ||
    url.contains("analytics") || url.contains("tracking") ||
    url.contains("ads") || url.contains("pixel") ||
    url.contains("gtm.js") || url.contains("ga.js")
}
```

### 2. Respuesta AI-native (no HTML)
En vez de devolver HTML + text dump, devolver JSON estructurado:

```json
{
  "url": "https://google.es/search?q=pelotas+rojas",
  "title": "pelotas rojas - Google",
  "intent": "search_results",

  "content": {
    "main": "10 resultados de búsqueda para 'pelotas rojas'",
    "results": [
      {"title": "Pelota roja Amazon", "url": "...", "snippet": "..."},
      {"title": "Pelotas Decathlon", "url": "...", "snippet": "..."}
    ]
  },

  "actions": [
    {"type": "search", "target": "q", "placeholder": "Buscar"},
    {"type": "link", "text": "Siguiente", "url": "/search?q=...&start=10"},
    {"type": "link", "text": "Imágenes", "url": "/search?q=...&tbm=isch"}
  ],

  "forms": [
    {"action": "/search", "method": "GET", "fields": [
      {"name": "q", "type": "text", "value": "pelotas rojas"}
    ]}
  ],

  "meta": {
    "language": "es",
    "render_ms": 450,
    "scripts_executed": 12,
    "cookies_set": 3
  }
}
```

### 3. Compresión semántica
No enviar todo el texto de la página. Comprimir:
- Eliminar whitespace excesivo
- Eliminar texto duplicado (headers, footers, nav repetido)
- Eliminar boilerplate (copyright, privacy policy links)
- Priorizar: contenido principal > sidebar > footer
- Truncar texto largo a N chars con "..."

**Implementación**: `js/compress.js`
```javascript
globalThis.__neo_compress = function(maxChars) {
    // 1. Find main content area
    const main = document.querySelector('main, article, [role="main"], #content, .content')
        || document.body;

    // 2. Extract text blocks with context
    const blocks = [];
    function walk(el, depth) {
        if (depth > 20) return;
        const tag = el.tagName?.toLowerCase();
        if (['script','style','nav','footer','aside','noscript'].includes(tag)) return;

        // Text node
        const text = el.textContent?.trim();
        if (text && text.length > 10) {
            blocks.push({
                tag,
                text: text.slice(0, 500),
                priority: getPriority(el),
            });
        }
        for (const child of el.children || []) walk(child, depth+1);
    }

    function getPriority(el) {
        const tag = el.tagName?.toLowerCase();
        if (['h1','h2','h3'].includes(tag)) return 10;
        if (['p','li','td'].includes(tag)) return 5;
        if (['span','div'].includes(tag)) return 2;
        if (el.closest?.('main,article,[role="main"]')) return 8;
        if (el.closest?.('nav,footer,aside')) return 1;
        return 3;
    }

    walk(main, 0);

    // 3. Sort by priority, truncate to maxChars
    blocks.sort((a,b) => b.priority - a.priority);
    let total = 0;
    const compressed = [];
    for (const b of blocks) {
        if (total + b.text.length > maxChars) break;
        compressed.push(b);
        total += b.text.length;
    }

    return JSON.stringify(compressed);
};
```

### 4. Detección automática de tipo de página
Clasificar la página automáticamente:

```javascript
globalThis.__neo_classify = function() {
    const url = location.href;
    const title = document.title;
    const forms = document.querySelectorAll('form').length;
    const inputs = document.querySelectorAll('input').length;
    const articles = document.querySelectorAll('article').length;
    const tables = document.querySelectorAll('table').length;
    const results = document.querySelectorAll('[class*="result"],[class*="search"]').length;

    if (url.includes('/search') || results > 3) return 'search_results';
    if (url.includes('/login') || url.includes('/signin') ||
        (inputs > 1 && inputs < 5 && forms > 0)) return 'login';
    if (articles > 0 || document.querySelector('article')) return 'article';
    if (tables > 2) return 'data_table';
    if (forms > 0 && inputs > 3) return 'form';
    if (url === '/' || url.endsWith('.com') || url.endsWith('.es')) return 'homepage';
    if (document.querySelectorAll('[class*="product"]').length > 0) return 'product';
    if (document.querySelectorAll('[class*="cart"],[class*="checkout"]').length > 0) return 'checkout';
    return 'content';
};
```

### 5. Smart prefetch
Anticipar qué va a querer la IA:
- En search results → pre-fetch los primeros 3 resultados
- En article → pre-fetch "next page" si hay paginación
- En form → identificar qué campos son obligatorios

### 6. Delta updates
No re-enviar toda la página si solo cambió un poco:
- Después de click/type → enviar SOLO lo que cambió
- MutationObserver → diff compacto
- La IA no necesita re-leer el header y footer

### 7. Auto-skip noise
Automáticamente filtrar:
- Cookie banners (ya tenemos consent auto-accept)
- Newsletter popups
- Chat widgets (Intercom, Drift, etc.)
- Cookie preference modals
- Age verification gates
- Paywalls (detectar y reportar, no bloquear)

### 8. Parallel page intelligence
Mientras la IA procesa la página actual:
- Pre-analizar links (¿son internos o externos?)
- Pre-clasificar forms (¿login, search, contact?)
- Pre-extraer structured data (JSON-LD, microdata)
- Tener ready el siguiente paso probable

### 9. Session intelligence
Aprender del comportamiento:
- Si la IA siempre ignora el footer → no enviarlo
- Si la IA siempre lee articles → priorizar main content
- Si la IA siempre clicka el primer resultado → prefetch automático

### 10. Error as information
En vez de "403 Forbidden" → dar contexto:
```json
{
  "blocked": true,
  "reason": "Cloudflare WAF",
  "suggestions": [
    "Use cookies from Chrome session",
    "Try different IP/proxy",
    "This site requires human verification"
  ]
}
```

## Implementación — archivos a crear

```
js/
├── compress.js     — compresión semántica del contenido
├── classify.js     — detección automática de tipo de página
├── prefetch.js     — smart prefetch de recursos probables
├── noise.js        — auto-skip de popups/banners/widgets

src/neorender/
├── ai_response.rs  — formato de respuesta AI-native JSON
├── resource_filter.rs — skip images/css/fonts/analytics
├── delta.rs        — delta updates (solo cambios)
├── intelligence.rs — page classification + prefetch
```
