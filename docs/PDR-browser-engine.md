# PDR: NeoRender — Browser Engine for AI

## Vision

Un browser real construido a piezas de bajo nivel. Sin parte visual. Output = WOM (Web Object Model), no píxeles. Cada pieza es un módulo reemplazable.

## Validated (v0.5.0)

| Pieza | Implementación | Status |
|-------|---------------|--------|
| JS Engine | deno_core (V8) | ✅ ES modules, eval, event loop |
| DOM | linkedom | ✅ parseHTML, outerHTML, querySelector |
| TLS | rquest (BoringSSL Chrome131) | ✅ Pasa Amazon, LinkedIn, SO |
| HTTP | rquest + cookie_store | ✅ Redirects, cookies auto |
| Session | NeoSession | ✅ Persistent across navigations |
| WOM | html5ever re-parse | ⚠️ Debería generarse desde linkedom directo |

## Architecture Target

```
┌─────────────────────────────────────────────┐
│              NeoSession                      │
│  (persistent runtime, owns all modules)     │
├─────────────┬───────────┬──────────────────┤
│ net/        │ dom/      │ web/              │
│ ┌─────────┐ │ ┌───────┐ │ ┌──────────────┐ │
│ │ Client  │ │ │Linkedom│ │ │ Fetch Std    │ │
│ │ (rquest)│ │ │ DOM   │ │ │ CORS, Origin │ │
│ │ Chrome  │ │ │ Parser│ │ │ Sec-Fetch-*  │ │
│ │ TLS     │ │ └───────┘ │ │ Referrer     │ │
│ └─────────┘ │ ┌───────┐ │ └──────────────┘ │
│ ┌─────────┐ │ │ WOM   │ │ ┌──────────────┐ │
│ │ Cookie  │ │ │ Gen   │ │ │ Storage      │ │
│ │ Store   │ │ │(dirct)│ │ │ localStorage │ │
│ │ (auto)  │ │ └───────┘ │ │ sessionStore │ │
│ └─────────┘ │           │ │ (SQLite)     │ │
│ ┌─────────┐ │           │ └──────────────┘ │
│ │ Session │ │           │ ┌──────────────┐ │
│ │ Cache   │ │           │ │ Crypto       │ │
│ │ /domain │ │           │ │ SubtleCrypto │ │
│ └─────────┘ │           │ │ POW (native) │ │
├─────────────┤           │ └──────────────┘ │
│ js/         │           │ ┌──────────────┐ │
│ ┌─────────┐ │           │ │ Observers    │ │
│ │ V8      │ │           │ │ Mutation     │ │
│ │ deno    │ │           │ │ Intersection │ │
│ │ core    │ │           │ │ Resize       │ │
│ └─────────┘ │           │ └──────────────┘ │
│ ┌─────────┐ │           │ ┌──────────────┐ │
│ │ Module  │ │           │ │ Events       │ │
│ │ Loader  │ │           │ │ EventTarget  │ │
│ │ (HTTP)  │ │           │ │ DOM Events   │ │
│ └─────────┘ │           │ │ Custom       │ │
│             │           │ └──────────────┘ │
└─────────────┴───────────┴──────────────────┘
         ↓ output
    ┌──────────┐
    │   WOM    │  Web Object Model
    │ (actions │  - text, links, forms
    │  + map)  │  - interactive elements
    └──────────┘  - API endpoints discovered
```

## Modules to Build

### Phase 1: Networking (replace manual header hacks)

**`src/neorender/net/mod.rs`** — Fetch Standard implementation

```rust
pub struct BrowserNetwork {
    client: rquest::Client,      // Chrome TLS
    origin: String,              // Current page origin
    referrer_policy: ReferrerPolicy,
}

impl BrowserNetwork {
    // Standard fetch() with automatic browser headers
    pub async fn fetch(&self, request: FetchRequest) -> FetchResponse;

    // CORS preflight when needed
    fn needs_preflight(&self, request: &FetchRequest) -> bool;

    // Compute Sec-Fetch-* headers per spec
    fn sec_fetch_headers(&self, url: &str, mode: RequestMode) -> HeaderMap;

    // Referrer policy computation
    fn compute_referrer(&self, url: &str) -> Option<String>;
}
```

No more manual header injection in ops.rs. The network module handles everything.

### Phase 2: Storage (persist across sessions)

**`src/neorender/storage/mod.rs`**

```rust
pub struct BrowserStorage {
    db: rusqlite::Connection,    // SQLite (already a dependency)
    domain: String,
}

impl BrowserStorage {
    pub fn local_storage(&self) -> LocalStorage;      // persists to disk
    pub fn session_storage(&self) -> SessionStorage;  // memory only
    pub fn cookies(&self) -> CookieJar;               // syncs with network
}
```

Bridge to JS via ops:
- `op_storage_get(domain, key)` → reads from SQLite
- `op_storage_set(domain, key, value)` → writes to SQLite
- No more injecting localStorage via JS eval

### Phase 3: WOM from linkedom (no re-parse)

Currently: linkedom renders DOM → export as HTML → re-parse with html5ever → extract WOM.

Target: linkedom renders DOM → extract WOM directly from V8.

```javascript
// In V8: walk linkedom's DOM tree, output WOM JSON
globalThis.__wom_extract = function() {
    const nodes = [];
    function walk(el, depth) {
        if (depth > 100) return;
        const tag = el.tagName?.toLowerCase();
        if (!tag || ['script','style','noscript','svg'].includes(tag)) return;

        const node = { tag };
        if (el.id) node.id = el.id;
        if (el.textContent?.trim()) node.text = el.textContent.trim().slice(0, 200);

        // Interactive elements
        if (tag === 'a' && el.href) node.href = el.href;
        if (tag === 'input') { node.type = el.type; node.name = el.name; node.placeholder = el.placeholder; }
        if (tag === 'button') node.text = el.textContent?.trim();
        if (tag === 'form') { node.action = el.action; node.method = el.method; }

        // Visible text at block level
        if (['h1','h2','h3','h4','h5','h6','p','li','td','th','label','span'].includes(tag)) {
            node.visible_text = el.textContent?.trim()?.slice(0, 500);
        }

        nodes.push(node);
        for (const child of el.children || []) walk(child, depth + 1);
    }
    walk(document.body, 0);
    return JSON.stringify(nodes);
};
```

This eliminates the html5ever re-parse step entirely.

### Phase 4: Web APIs (real implementations, not stubs)

Replace stubs with real implementations where linkedom provides them:

| API | Current | Target |
|-----|---------|--------|
| MutationObserver | stub (no-op) | linkedom's real implementation |
| EventTarget | stub/linkedom | linkedom's (already working) |
| IntersectionObserver | stub | smart stub (mark visible/not) |
| ResizeObserver | stub | no-op (no layout) |
| ReadableStream | minimal | functional (for SSE, streaming) |
| WebSocket | stub | rquest websocket (for live data) |
| Service Worker | stub | skip (not needed for rendering) |

### Phase 5: Error Isolation

Currently one script error can cascade. Target:

```rust
// Each script runs in a try-catch at the V8 level
for script in scripts {
    match execute_with_catch(&mut runtime, script) {
        Ok(()) => {},
        Err(e) => {
            errors.push(e);
            // Continue — don't stop the render
        }
    }
}
```

Also: separate analytics/tracking scripts from app scripts. Skip analytics entirely.

## File Structure

```
src/neorender/
├── mod.rs              # render_page (legacy, keep as fallback)
├── session.rs          # NeoSession (persistent browser)
├── v8_runtime.rs       # V8 + linkedom + module loader
├── ops.rs              # JS ↔ Rust bridge ops
├── dom_export.rs       # DOM → HTML (legacy)
├── net/
│   ├── mod.rs          # BrowserNetwork (Fetch Standard)
│   ├── cors.rs         # CORS preflight
│   ├── referrer.rs     # Referrer policy
│   └── headers.rs      # Sec-Fetch-*, Origin, etc.
├── storage/
│   ├── mod.rs          # BrowserStorage
│   ├── local.rs        # localStorage (SQLite)
│   └── session.rs      # sessionStorage (memory)
└── wom/
    └── extract.rs      # WOM generation from linkedom

js/
├── linkedom.js         # DOM engine (477KB, vendored)
├── bootstrap.js        # Browser globals + polyfills
└── wom.js              # WOM extraction (in-V8)
```

## Priority Order

1. **net/ module** — eliminates header hacks, fixes ChatGPT and all sites that check browser behavior
2. **WOM from linkedom** — eliminates re-parse overhead, cleaner architecture
3. **storage/ module** — persistent localStorage, real cookie management
4. **Error isolation** — makes more sites work without patching each one
5. **Web APIs** — progressive, driven by which sites need what

## Success Metric

All 20 top sites render with content via NeoSession, including:
- ChatGPT (send/receive messages)
- Amazon (authenticated, orders)
- LinkedIn (authenticated, feed + messaging)
- Facebook (at least login page, ideally feed)

Zero Chrome dependency for normal browsing. Chrome only for initial auth + WAF resolution.
