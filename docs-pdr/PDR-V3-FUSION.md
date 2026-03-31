# PDR: NeoRender V3 — Fusion Architecture

## Decisión

Combinar lo mejor de V1 (Chrome fallback, velocidad, vision) con V2 (modularidad, WOM, tracing, MCP).

## Arquitectura V3: Three-Engine Auto-Select

```
navigate(url)
  │
  ├─ Phase 1: LIGHT (HTTP + html5ever parse)
  │   ├─ 200ms typical
  │   ├─ Chrome 145 TLS fingerprint (wreq)
  │   ├─ Extract text + links + forms
  │   └─ IF content > threshold → DONE
  │
  ├─ Phase 2: V8 (deno_core + happy-dom)  [skip for SPAs]
  │   ├─ 2-8s typical
  │   ├─ SSR hydration, inline scripts
  │   ├─ Module loading + evaluation
  │   └─ IF content > threshold → DONE
  │
  └─ Phase 3: CHROME (headless via CDP)
      ├─ 3-8s typical
      ├─ Full SPA rendering (React, Vue, Angular)
      ├─ Cloudflare/Turnstile bypass
      ├─ Real event loop, MessageChannel, etc.
      └─ ALWAYS produces content → DONE
```

## Auto-Detection Logic

```
content_threshold = 50 chars visible text OR 10+ interactive elements

after_light:
  if SSR content (text > threshold) → return light result
  if SPA markers (empty root, __NEXT_DATA__, id="app") → skip V8, go Chrome
  else → try V8

after_v8:
  if content > threshold → return V8 result
  else → fallback to Chrome
```

## What V3 Takes From Each

| From V1 | From V2 |
|---------|---------|
| Chrome fallback (always works) | Clean crate architecture |
| Auto-engine selection | WOM extraction + classification |
| Module stubbing (82MB savings) | Structured tracing |
| V8 bytecode caching | MCP server (13 tools) |
| Vision/semantic analysis | Cross-origin isolation |
| Fast light mode | Cookie persistence (SQLite) |
| Token efficiency (7.9x) | ES module system |

## Implementation: 3 Steps

1. Add `render_with_chrome()` to NeoSession using neo-chrome crate
2. Add auto-detection in pipeline: if V8 result empty → Chrome
3. Extract WOM from Chrome-rendered DOM (reuse neo-extract)
