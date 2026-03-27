# PDR: V3 Real Fusion — Single Rust Binary

## Decision

V3 = single Rust binary. No Python. No V1 CLI calls.
All engines, importers, and tools built-in.

## Architecture

```
neorender v3 mcp
  │
  ├── Engine 1: FAST (HTTP + html5ever)
  │   ├── wreq Chrome 145 TLS
  │   ├── Chrome headers (Sec-Ch-Ua, Sec-Fetch-*)
  │   ├── Cookie import from Chrome profile (Rust)
  │   ├── Cookie store (SQLite, persistent)
  │   ├── html5ever DOM parse
  │   ├── WOM extraction + compact view
  │   └── ~500ms-3s
  │
  ├── Engine 2: CHROME (CDP neomode)
  │   ├── neo-chrome launcher (Rust)
  │   ├── CDP WebSocket client (Rust)
  │   ├── Neomode patches (5 JS properties)
  │   ├── Cookie injection from store
  │   ├── Page.navigate + wait
  │   ├── Runtime.evaluate for click/type/find/extract
  │   └── ~3-10s
  │
  └── Auto-select:
      if HTTP gives >200 chars text → FAST
      if empty or CF challenge → CHROME
```

## What exists (crates ready):

| Crate | Status | What it does |
|-------|--------|-------------|
| neo-http | ✅ | wreq Chrome TLS + headers |
| neo-http/cookies | ✅ | Chrome cookie decrypt (Rust) |
| neo-dom | ✅ | html5ever parse |
| neo-extract | ✅ | WOM + compact view |
| neo-chrome | ⚠️ | CDP client (WebSocket hangs in V8 context) |
| neo-mcp | ✅ | MCP server + 15 tools |
| neo-engine | ✅ | Pipeline orchestrator |

## What needs fixing:

1. **neo-chrome WebSocket**: hangs when called from V8 tokio context
   Fix: run Chrome in separate thread with own tokio runtime
   (already proven to work in ghost.py — just port to Rust)

2. **neo-chrome neomode**: add 5 JS patches after launch
   Fix: Page.addScriptToEvaluateOnNewDocument via CDP

3. **MCP ghost tool**: currently calls ghost.py
   Fix: call neo-chrome directly from Rust

## Implementation order:

1. Fix neo-chrome WebSocket (separate thread + tokio)
2. Add neomode patches to neo-chrome session
3. Wire ghost tool → neo-chrome instead of ghost.py
4. Test: LinkedIn, Factorial, ChatGPT
5. Remove Python dependency
