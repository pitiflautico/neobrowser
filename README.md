# NeoRender

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

**The browser without a rendering engine.** V8 + linkedom + rquest. Built in Rust.

Loads pages, executes JavaScript, hydrates React — at Chrome speed, without Chrome.

```
ChatGPT cached load:  3.1s  (Chrome: 3-5s)
ChatGPT first load:   8.1s
Cloudflare bypass:    19/20 top sites
React Router 7:       Full hydration in V8
Token footprint:      7.9x less than Chrome MCP
```

## Why

AI agents need to browse the web. Current options:

| Tool | Approach | Problem |
|---|---|---|
| Playwright/Puppeteer | Controls Chrome | Heavy, detectable, 100MB+ |
| Chrome headless | Full browser | Cloudflare blocks, expensive |
| HTTP scrapers | curl + parse | No JavaScript, no SPAs |
| **NeoRender** | **V8 + DOM + Chrome TLS** | **Fast, light, invisible** |

NeoRender is a browser that only has the parts AI agents need: HTTP with real Chrome fingerprint, a DOM, and JavaScript execution. No pixels, no GPU, no rendering pipeline.

## How It Works

```
┌──────────────────────────────────────────────────┐
│                   NeoRender                       │
├──────────┬──────────┬──────────┬─────────────────┤
│  rquest  │ linkedom │  V8      │  MCP Server     │
│ Chrome   │ DOM API  │ deno_core│  35+ tools      │
│ 136 TLS  │ No render│ ES modules│ JSON-RPC stdio  │
├──────────┴──────────┴──────────┴─────────────────┤
│ Caches: HTTP module cache │ V8 bytecode cache     │
│ Stubs:  63 heavy modules replaced with proxies    │
│ Polyfills: ViewTransition, getAll, allSettled      │
└──────────────────────────────────────────────────┘
```

**Three engines in one binary:**

| Engine | Use case | JS | Speed |
|---|---|---|---|
| **NeoRender** | SPAs, React, Vue | Full V8 | 3s cached |
| **Light** | Static pages, APIs | None | 200ms |
| **Chrome** | Captchas, login | Full browser | 5s |

NeoRender picks automatically. Most pages work without Chrome.

## Install

```bash
# Build from source
git clone https://github.com/pitiflautico/neobrowser.git
cd neobrowser && cargo build --release
# Binary: target/release/neobrowser_rs
```

## Quick Start

### As MCP server (Claude Code, Cursor, any MCP client)

```json
{
  "mcpServers": {
    "neobrowser": {
      "type": "stdio",
      "command": "/path/to/neobrowser_rs",
      "args": ["mcp"],
      "env": { "NEOBROWSER_HEADLESS": "1" }
    }
  }
}
```

### CLI

```bash
# Browse any page
neobrowser_rs see https://news.ycombinator.com

# Full browser when needed
neobrowser_rs browse https://chatgpt.com

# Login to sites (saves cookies)
neobrowser_rs login https://chatgpt.com
```

## Performance

### ChatGPT (React Router 7 SPA)

| Metric | NeoRender | Chrome | Speedup |
|---|---|---|---|
| Cached load | **3.1s** | 3-5s | **1x (parity)** |
| First load | 8.1s | 5s | 0.6x |
| React hydration | Yes | Yes | - |
| Token cost | ~4K chars | ~140K chars | **35x** |
| Binary size | 15MB | 300MB+ | **20x** |
| Memory | ~100MB | ~500MB | **5x** |

### 20 Top Sites (NeoRender, no Chrome)

| Site | Status | Time | Size |
|---|---|---|---|
| Google | Pass | 0.4s | 354KB |
| Reddit | Pass | 0.5s | 567KB |
| YouTube | Pass | 0.6s | 723KB |
| Wikipedia | Pass | 0.8s | 1MB |
| Amazon | Pass | 0.7s | 1MB |
| Stack Overflow | Pass | 0.3s | 234KB |
| ChatGPT | Pass | 3.1s | 288KB |
| NYT | Pass | 0.9s | 1.2MB |
| LinkedIn | Pass | 1.2s | 8.7MB |
| Apple | Pass | 0.3s | 229KB |
| Netflix | Pass | 0.5s | 538KB |
| Instagram | Pass | 0.5s | 648KB |
| Facebook | Pass | 0.4s | 401KB |
| Notion | Pass | 0.3s | 279KB |
| Google Docs | Pass | 0.9s | 1.1MB |
| Twitch | Pass | 0.2s | 182KB |
| El Pais | Pass | 0.4s | 431KB |
| Microsoft | Pass | 0.3s | 225KB |
| BBC | Timeout | - | Heavy JS |
| HN | Pass | 0.1s | 33KB |

**19/20 pass Cloudflare** with Chrome 136 TLS fingerprint. Zero detection.

## What Makes It Different

### No Rendering Engine

Traditional headless browsers (Playwright, Puppeteer, Chrome headless) run a full rendering pipeline: layout, paint, composite, GPU. NeoRender skips all of that. The DOM exists in memory (linkedom), JavaScript executes in V8, but no pixels are ever computed.

### React Hydration in V8

First known implementation of React Router 7 streaming SSR hydration in a headless V8 runtime without a browser. ChatGPT's React app hydrates completely:

- 47 ES modules loaded and evaluated
- `Promise.allSettled` source-level rewrite (deno_core module scope issue)
- `Object.prototype.getAll` fallback (React Router Early Hints)
- `ReadableStream.pipeThrough` no-op (prevents V8 pipe promise deadlock)
- Inline modules converted to async IIFE (bypasses top-level await blocking)
- `ViewTransition` API polyfill (React 19)

### Chrome 136 TLS Fingerprint

HTTP requests use rquest with `Emulation::Chrome136` — identical TLS fingerprint (JA3/JA4), HTTP/2 settings, and header order to real Chrome. Cloudflare, Akamai, and PerimeterX cannot distinguish NeoRender from a real browser.

### V8 Bytecode Cache

First module load compiles JavaScript and caches V8 bytecode to disk (`~/.neobrowser/cache/v8/`). Subsequent loads skip compilation entirely. This is the same technique Chrome uses internally.

### Module Stubbing

Heavy modules (>1MB) that aren't needed for core functionality are replaced with lightweight Proxy stubs (~200 bytes). ChatGPT loads 362 modules (98MB total) — 63 are stubbed, saving 82MB of V8 parsing time.

## MCP Tools

35+ tools organized by function:

### Core
| Tool | Description |
|---|---|
| `browser_open` | Navigate to URL (modes: neorender, light, chrome, auto) |
| `browser_observe` | Read page content (formats: see, compact, content, delta) |
| `browser_act` | Interact: click, type, scroll, eval, send_message, fill_form |
| `browser_wait` | Wait for text, element, or time |
| `browser_tabs` | Manage browser tabs |

### API & Data
| Tool | Description |
|---|---|
| `browser_fetch` | HTTP via rquest (Chrome 136 TLS, no browser needed) |
| `browser_api` | HTTP from browser context (inherits cookies) |
| `browser_record` | Record HTTP traffic for replay analysis |

### Session & Auth
| Tool | Description |
|---|---|
| `browser_auth` | OS keychain integration, credential management |
| `browser_session` | Screenshots, network capture, console logs |
| `browser_state` | Export/import cookies + localStorage |

### Automation
| Tool | Description |
|---|---|
| `browser_pipeline` | Run multi-step workflows with retry and assertions |
| `browser_pool` | Multi-context browser isolation |
| `browser_learn` | Record and replay web workflows |
| `browser_network` | Network intelligence (HAR, intercept, mock) |
| `browser_trace` | Action observability with timing and stats |

## Architecture

```
38K lines — 30 JS modules, 17 Rust modules

src/
  neorender/          # V8 browser engine
    session.rs        # Navigation pipeline, script execution, caching
    v8_runtime.rs     # V8 runtime, module loader, code cache
    ops.rs            # V8 ops: fetch, timer, crypto, storage
    net/              # Browser-standard HTTP headers
    mod.rs            # HTML parsing, script extraction, ES imports
  engine.rs           # Chrome CDP session (fallback)
  mcp.rs              # MCP server, 35 tools, pipeline executor
  ghost.rs            # Light mode: HTTP + parse
  stealth.rs          # Anti-detection fingerprinting
  identity.rs         # Polymorphic browser identity
  cdp.rs              # Raw CDP client (WebSocket + Pipe)

js/
  bootstrap.js        # Browser environment polyfills
  webapis.js          # Web API stubs (Permissions, etc.)
  layout.js           # Element dimensions, canvas, WebGL
  dynamic_scripts.js  # appendChild interceptor for <script>
  observer.js         # MutationObserver + DOM snapshots
  ... (30 modules)
```

## Caching

Three cache layers for maximum speed:

| Cache | Location | What | Impact |
|---|---|---|---|
| HTTP modules | `~/.neobrowser/cache/modules/` | Pre-fetched JS source | 6.5s → 0.6s |
| V8 bytecode | `~/.neobrowser/cache/v8/` | Compiled V8 bytecode | 25s → 2s |
| Cookies | `~/.neobrowser/storage/cookies.db` | Unified SQLite jar | Persistent auth |

Clear caches: `rm -rf ~/.neobrowser/cache/`

## Environment Variables

| Variable | Default | Description |
|---|---|---|
| `NEOBROWSER_HEADLESS` | `0` | `1` = Chrome offscreen (for fallback mode) |
| `NEOBROWSER_PROFILE` | `~/.neobrowser/profile` | Chrome profile directory |
| `NEOBROWSER_COOKIES` | — | Cookie JSON files to pre-load |
| `NEOBROWSER_STUB_THRESHOLD` | `1000000` | Module stub threshold (bytes). `0` = disabled |

## Comparison with Alternatives

| Feature | NeoRender | Playwright | Firecrawl | Browser Use |
|---|---|---|---|---|
| JS execution | V8 | Chrome | Chrome | Chrome |
| React hydration | Yes | Yes | No | Yes |
| Chrome required | No* | Yes | Yes (cloud) | Yes |
| TLS fingerprint | Chrome 136 | Chrome | Varies | Chrome |
| Binary size | 15MB | 300MB+ | Cloud | 300MB+ |
| Cloudflare bypass | 19/20 | Detectable | Cloud proxy | Detectable |
| Token efficiency | 7.9x less | Baseline | 2-3x less | Baseline |
| Self-hosted | Yes | Yes | Optional | Yes |

*Chrome used only as fallback for captchas/login.

## Known Limitations

- **BBC/heavy JS**: Pages with >500KB of JS in a single bundle may timeout
- **Cloudflare Turnstile**: Bypassed for authenticated accounts via PoW solver. Unauthenticated may need Chrome fallback
- **Canvas/WebGL**: Stubs return consistent values but not pixel-accurate
- **Real-time**: No WebSocket connections (use Chrome mode for chat apps)
- **V8 first load**: 8s for SPA pages (subsequent loads: 3s with cache)

## License

MIT
