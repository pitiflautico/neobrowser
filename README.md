# NeoBrowser

[![CI](https://github.com/pitiflautico/neobrowser/actions/workflows/ci.yml/badge.svg)](https://github.com/pitiflautico/neobrowser/actions)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

**Rust browser engine for AI agents.** Raw CDP over WebSocket/Pipe — no Playwright, no chromedriver, no overhead.

- **3.3x faster** than Chrome DevTools MCP, **7.9x less tokens** ([benchmark](#benchmark))
- **22+ MCP tools** — navigate, click, type, extract, send messages, run pipelines
- **Pipe CDP stealth** — `--remote-debugging-pipe` bypasses Cloudflare Turnstile (no TCP port)
- **Polymorphic identity** — unique fingerprint per session (UA, GPU, screen, canvas, audio)
- **Session persistence** — login once, cookies survive restarts via SQLite pre-persistence
- **AI-native output** — semantic text compression, no screenshots needed
- **Zero dependencies** — single binary, just needs Chrome installed

## Quick Start

```bash
# 1. Build
git clone https://github.com/pitiflautico/neobrowser.git
cd neobrowser
cargo build --release
# Binary: target/release/neobrowser_rs

# 2. Setup: opens Chrome, login to your sites, saves session
./target/release/neobrowser_rs setup --sites linkedin.com,chatgpt.com

# 3. Add to Claude Code (~/.claude.json)
```

```json
{
  "mcpServers": {
    "neobrowser": {
      "type": "stdio",
      "command": "/path/to/neobrowser_rs",
      "args": ["mcp"],
      "env": {
        "NEOBROWSER_COOKIES": "~/.cookies/linkedin.json"
      }
    }
  }
}
```

Chrome required. Communicates via CDP WebSocket or Pipe — no chromedriver binary needed.

## Benchmark

Tested against [Chrome DevTools MCP](https://github.com/nichochar/chrome-devtools-mcp) (official Chrome MCP) on the same tasks, same machine. NeoBrowser runs headless with pipe CDP stealth; Chrome MCP uses an open Chrome window.

### Speed

| Test | Chrome MCP | NeoBrowser | Speedup |
|---|---|---|---|
| Navigate + Read (example.com) | 5,200ms | 3,000ms | **1.7x** |
| Extract content (Hacker News) | 13,853ms | 5,000ms | **2.8x** |
| Search form (Google) | 29,224ms | 8,000ms | **3.7x** |
| Multi-step navigate + click (Wikipedia) | 34,144ms | 9,000ms | **3.8x** |
| **Total** | **82,421ms** | **25,000ms** | **3.3x** |

### Token efficiency

| Test | Chrome MCP | NeoBrowser | Reduction |
|---|---|---|---|
| Navigate + Read | 250 chars | 390 chars | 0.6x* |
| Extract content | 38,297 chars | 4,039 chars | **9.5x** |
| Search form | 19,966 chars | 6,518 chars | **3.1x** |
| Multi-step | 1,084,270 chars | 132,845 chars | **8.2x** |
| **Total** | **1,142,783 chars** | **143,792 chars** | **7.9x** |

*\*NeoBrowser includes metadata (page class, revision, action count) in simple pages.*

### Why NeoBrowser is faster

- **One tool call** does navigate + extract (Chrome MCP needs `navigate` + `take_snapshot` = 2 calls minimum)
- **Semantic compression** — WOM extracts only actionable content, not full a11y tree
- **No screenshot needed** — text output is self-contained and AI-optimized
- **Headless autonomous** — doesn't require an open Chrome window

### What Chrome MCP does better

- **Visual debugging** — screenshots and DevTools panel integration
- **Performance profiling** — Lighthouse audits, memory snapshots, performance traces
- **Manual workflow** — great when you need to see what the browser is doing

## Architecture

```
┌─────────────────────────────────────────────────────┐
│                    MCP Server                        │
│                (JSON-RPC / stdio)                    │
├─────────────────────────────────────────────────────┤
│  13 Tools:                                           │
│  Core:     open · observe · act · wait · tabs        │
│  Auth:     auth · session · api                      │
│  New:      state · network · trace · pipeline · pool │
├─────────────────────────────────────────────────────┤
│  Engine (raw CDP)  │  Stealth  │  Reliability        │
│  ┌────────┐ ┌────────┐ ┌──────────┐ ┌───────────┐  │
│  │ Light  │ │ Chrome │ │Polymorph │ │ 4-strategy│  │
│  │reqwest │ │CDP Pipe│ │Identity/ │ │ click     │  │
│  │html5ev │ │  / WS  │ │Canvas/GL │ │ fallback  │  │
│  └────────┘ └────────┘ └──────────┘ └───────────┘  │
├─────────────────────────────────────────────────────┤
│  see_page │ WOM │ Delta │ Vision │ Trace │ Runner   │
└─────────────────────────────────────────────────────┘
```

### Source Files

| File | Purpose |
|---|---|
| `engine.rs` | Chrome CDP session — launch, navigate, click, type, eval, frames, see_page, state export, network capture, reliability |
| `mcp.rs` | MCP server — 13 tools, JSON-RPC loop, pipeline executor |
| `stealth.rs` | Anti-detection: canvas noise, WebGL spoof, AudioContext, plugins, timezone, screen, iframe |
| `trace.rs` | Per-action tracing with timing, outcomes, success rate stats |
| `runner.rs` | Pipeline definitions: steps, retry, assertions, variables |
| `pool.rs` | Multi-context browser pool with isolated profiles |
| `wom.rs` | Web Object Model — DOM to structured nodes with stable IDs |
| `vision.rs` | Page classification — type (article, form, list, app) and state |
| `semantic.rs` | AX-tree text extraction from HTML |
| `delta.rs` | Diff between WOM revisions |
| `cdp.rs` | Raw CDP client — WebSocket + Pipe (fd 3/4) |
| `identity.rs` | Polymorphic browser identity generator (OS-matched) |
| `auth.rs` | Auth profiles, OS keychain, session persistence |
| `main.rs` | CLI + module wiring |

## MCP Tools

### Core Tools

#### browser_open
```json
{"url": "https://example.com", "mode": "chrome"}
```
Modes: `light` (HTTP only), `chrome` (full browser), `auto`.

#### browser_observe
```json
{"format": "see"}
```
Formats: `see` (recommended, ~100ms), `compact`, `content`, `full`, `delta`.

#### browser_act
```json
{"kind": "click", "target": "Sign in"}
{"kind": "type", "text": "hello world"}
{"kind": "eval", "text": "document.title"}
{"kind": "fill_form", "fields": {"user": "john", "pass": "secret"}}
{"kind": "press", "key": "Enter"}
{"kind": "scroll", "direction": "down"}
{"kind": "send_message", "text": "Hello!", "input_selector": "div[contenteditable='true']", "button_selector": "button[type='submit']"}
```
Actions: click, type, focus, press, scroll, back, forward, reload, eval, hover, select, fill_form, send_message.

`send_message` is a universal contenteditable message sender. Works with LinkedIn, Slack, Discord, and any site using contenteditable + send button. Uses `execCommand('insertText')` + InputEvent to activate React/framework bindings. Defaults: `input_selector` = `div[contenteditable='true']`, `button_selector` = auto-detect.

Targets: text match (`"Sign in"`), WOM ID (`"btn_003"`), or empty for auto-focus.

#### browser_wait
```json
{"text_present": "Welcome", "timeout_ms": 10000}
{"seconds": 2}
{"text_absent": "Loading..."}
```

#### browser_api
HTTP from browser context (inherits cookies/session):
```json
{"url": "/api/data", "extract": "json"}
{"url": "/page", "method": "POST", "body": "key=val", "extract": "text"}
```
Extract: `text`, `json`, `html`, `headers`.

#### browser_tabs
```json
{"op": "list"}
{"op": "switch", "index": 1}
{"op": "close", "index": 2}
```

#### browser_auth
OS keychain integration:
```json
{"op": "add_profile", "profile_id": "linkedin", "domains": ["linkedin.com"]}
{"op": "set_credential", "profile_id": "linkedin", "credential_kind": "password", "credential_value": "..."}
{"op": "auto_session", "domain": "linkedin.com"}
{"op": "extract_chrome", "domain": "linkedin.com", "port": 9222}
```

#### browser_session
```json
{"op": "start_capture"}
{"op": "screenshot"}
{"op": "network"}
{"op": "console"}
```

### Extended Tools (v2)

#### browser_state — Session state management
```json
{"op": "export"}
{"op": "export", "file": "/tmp/state.json"}
{"op": "import", "file": "/tmp/state.json"}
{"op": "health"}
```
- **export**: cookies + localStorage + sessionStorage as JSON
- **import**: restore from previous export
- **health**: check login status, captcha, errors, form detection

#### browser_network — Network intelligence
```json
{"op": "start"}
{"op": "read"}
{"op": "har"}
{"op": "intercept", "url_pattern": "*api*"}
```
- **start**: capture all fetch/XHR with headers + response bodies (capped 4KB)
- **read**: get captured requests (clears buffer)
- **har**: export as HAR 1.2 format
- **intercept**: set URL pattern for Fetch.enable interception

#### browser_trace — Action observability
```json
{"op": "start"}
{"op": "read", "last_n": 10}
{"op": "stats"}
{"op": "clear"}
{"op": "stop"}
```
Records every action with: id, action, target, outcome, effect, duration_ms, url, timestamp.

Stats include: total, succeeded, failed, avg_duration_ms, per-action success rates.

#### browser_pipeline — Deterministic runner
```json
{
  "pipeline": {
    "name": "login_flow",
    "steps": [
      {"action": "goto", "target": "https://app.com/login"},
      {"action": "type", "target": "email", "value": "user@test.com"},
      {"action": "type", "target": "password", "value": "secret"},
      {"action": "click", "target": "Sign in", "assert_text": "Dashboard"},
      {"action": "extract", "value": "document.title", "store_as": "title"},
      {"action": "screenshot"}
    ],
    "variables": {}
  }
}
```
Step actions: `goto`, `click` (with 4-strategy fallback), `type`, `press`, `wait`, `eval`, `extract`, `screenshot`.

Each step supports: `timeout_ms`, `max_retries`, `assert_text` (postcondition), `store_as` (variable capture), `on_fail` (abort/skip/continue).

Variables: `{{var_name}}` substitution in target/value fields.

#### browser_pool — Multi-context isolation
```json
{"op": "create", "id": "scraper1"}
{"op": "list"}
{"op": "destroy", "id": "scraper1"}
{"op": "destroy_all"}
```
Each context gets its own profile directory under `~/.neobrowser/pool/`.

## Environment Variables

| Variable | Default | Description |
|---|---|---|
| `NEOBROWSER_PROFILE` | `~/.neobrowser/profile` | Custom Chrome profile directory. Set different values to run multiple instances. |
| `NEOBROWSER_HEADLESS` | `0` | Set to `1` for headless Chrome. Default is headed (visible window). |
| `NEOBROWSER_COOKIES` | (none) | Comma-separated list of cookie JSON files. Pre-persisted to Chrome SQLite before launch. |
| `NEOBROWSER_STEALTH` | `0` | Set to `1` for pipe CDP mode. Bypasses Cloudflare Turnstile — no TCP port, no WebSocket. |

## Session & Auth

NeoBrowser uses Chrome profiles for persistent sessions. The default profile lives at `~/.neobrowser/profile/`.

**How login works:**

1. Start an interactive session: `neobrowser_rs session --url https://site.com`
2. Login manually in the visible Chrome window
3. Cookies and localStorage persist in the profile directory
4. Next launch reuses the same profile — you stay logged in

**Cookie injection (for automation):**

Export cookies from your real Chrome (see [Cookie Pre-Persistence](#cookie-pre-persistence)) and set `NEOBROWSER_COOKIES` to load them before Chrome starts.

**Multiple profiles:**

```bash
NEOBROWSER_PROFILE=~/.neobrowser/work neobrowser_rs mcp    # work account
NEOBROWSER_PROFILE=~/.neobrowser/personal neobrowser_rs mcp # personal account
```

## Stealth

Two modes: **normal** (WebSocket CDP) and **stealth** (pipe CDP, `NEOBROWSER_STEALTH=1`).

### Pipe CDP (stealth mode)

Uses `--remote-debugging-pipe` instead of `--remote-debugging-port`. Chrome communicates via fd 3/4 (Unix pipes) — **no TCP port, no WebSocket**. Cloudflare Turnstile cannot detect pipe-based CDP because there's nothing to scan.

```bash
NEOBROWSER_STEALTH=1 NEOBROWSER_HEADLESS=1 neobrowser_rs mcp
```

### Polymorphic identity

Each session gets a unique browser fingerprint generated from real-world hardware databases:

| Component | Technique |
|---|---|
| User-Agent | OS-matched (TLS fingerprint must match UA — Cloudflare compares JA3 hash) |
| GPU | Real hardware: Apple M1/M2/M3/M4, AMD Radeon Pro, Intel Iris |
| Screen | Realistic resolutions: 1440x900, 1512x982, 1920x1080, 2560x1440 |
| Canvas | Deterministic noise on `toDataURL`/`toBlob` (seeded per session) |
| WebGL | Vendor/renderer from GPU database |
| Audio | Gain variation on oscillator connections (seeded per session) |
| Timezone | Random from realistic pool (Europe/Madrid, America/New_York, etc.) |

### Injection layers

Applied **after** first navigation (not during launch — Cloudflare detects early CDP modifications):

| Layer | Technique |
|---|---|
| WebDriver | `navigator.webdriver` removal + prototype cleanup |
| Chrome runtime | `chrome.runtime` polyfill |
| Plugins | 3 standard Chrome plugins + mimeTypes |
| iframe | `webdriver` removal propagated to iframe `contentWindow` |
| Connection | `navigator.connection` (4g, 50ms RTT) |
| Visibility | `document.hidden=false`, `visibilityState=visible` |
| Permissions | `notifications` permission query passthrough |

## Session Recovery

NeoBrowser automatically recovers from Chrome crashes and zombie processes:

| Layer | Mechanism |
|---|---|
| CDP alive detection | Atomic flag in WebSocket recv loop — `is_alive()` returns `false` instantly when connection drops |
| Fast fail | `send()`/`send_to()` fail immediately on dead connection instead of waiting 30s timeout |
| Auto-recovery | `ensure_session()` detects dead sessions, drops them, and relaunches Chrome transparently |
| Zombie cleanup | `launch_ex()` runs `pkill -f user-data-dir=<profile>` before launch — only kills neobrowser-owned Chrome, never user's personal Chrome |
| Lock cleanup | Removes stale `SingletonLock` files left by crashed Chrome |

Recovery takes ~5s (Chrome launch + CDP connect). All pending CDP commands receive immediate error on disconnect.

## Cookie Pre-Persistence

Set `NEOBROWSER_COOKIES` to pre-persist cookies before Chrome launches:

```bash
NEOBROWSER_COOKIES=~/.cookies/gpt.json,~/.cookies/gemini.json
```

Cookies are written directly to Chrome's SQLite profile database. Chrome loads them from disk — no CDP injection needed, no race conditions.

**Important**: Always use SQLite pre-persistence for authenticated sessions. CDP `Network.setCookies` causes `ERR_TOO_MANY_REDIRECTS` on sites like LinkedIn that do complex redirect chains during auth verification. Pre-persistence writes cookies to Chrome's internal database, which Chrome reads natively — identical to a real user session.

### Extracting Cookies from a Running Chrome

Chrome locks its Cookie database while running. To extract cookies (e.g., `li_at` from LinkedIn):

```python
import shutil, browser_cookie3

# Copy DB + WAL (Chrome stores recent cookies in WAL)
profile = "~/Library/Application Support/Google/Chrome/Profile 24"
shutil.copy2(f"{profile}/Cookies", "/tmp/cookies.db")
for ext in ["-wal", "-shm"]:
    try: shutil.copy2(f"{profile}/Cookies{ext}", f"/tmp/cookies.db{ext}")
    except: pass

cj = browser_cookie3.chrome(domain_name='.linkedin.com', cookie_file='/tmp/cookies.db')
```

Then persist to neobrowser profile before launching:
```python
# Write to ~/.neobrowser/profile/Default/Cookies SQLite
# (same schema as Chrome — see persist_cookies_to_profile in engine.rs)
```

## CLI Commands

| Command | Description |
|---|---|
| `neobrowser_rs fetch <url>` | Light mode — HTTP fetch + HTML parse, no Chrome |
| `neobrowser_rs browse <url>` | Chrome mode — full browser, one-shot render |
| `neobrowser_rs see <url>` | Auto mode — tries light first, falls back to Chrome if JS needed |
| `neobrowser_rs wom <url> [--compact]` | WOM output — structured JSON for AI agents |
| `neobrowser_rs mcp` | MCP server mode (JSON-RPC over stdio) |
| `neobrowser_rs setup [--sites site1,site2]` | First-time setup — opens Chrome, guides login, saves profile |
| `neobrowser_rs session [--url <url>] [--cookies <file>] [--profile <dir>] [--port <n>] [--connect]` | Interactive session with visible Chrome |

### Session flags

| Flag | Effect |
|---|---|
| `--cookies <file>` | Load cookies (pre-persisted to SQLite + CDP injection) |
| `--url <url>` | Navigate to URL on start |
| `--profile <dir>` | Custom Chrome profile directory |
| `--port <n>` | Connect to Chrome already running on this debug port |
| `--connect` | Connect to user's running Chrome (reads DevToolsActivePort) |
| `--lines <n>` | Max content lines per `see` (default: 50) |

## Claude Code Integration

```json
{
  "mcpServers": {
    "neobrowser": {
      "type": "stdio",
      "command": "/path/to/neobrowser_rs",
      "args": ["mcp"],
      "env": {
        "NEOBROWSER_COOKIES": "~/.cookies/linkedin.json,~/.cookies/gpt.json"
      }
    }
  }
}
```

The MCP server auto-launches Chrome on first `browser_open` call and keeps the session alive across tool invocations. If Chrome dies, it auto-recovers on the next call.

## Troubleshooting

**Chrome not found**

NeoBrowser looks for Chrome at standard paths (`/Applications/Google Chrome.app` on macOS). If installed elsewhere, Chrome must be in `PATH`.

**ERR_TOO_MANY_REDIRECTS**

Do not use CDP `Network.setCookies` for auth cookies. Use `NEOBROWSER_COOKIES` env var for SQLite pre-persistence instead. Sites like LinkedIn do redirect-chain auth verification that breaks with CDP-injected cookies.

**Session lost / Chrome zombie**

NeoBrowser kills zombie Chrome processes matching its profile dir on launch. If you see `SingletonLock` errors, delete `~/.neobrowser/profile/SingletonLock` manually or let the auto-recovery handle it on next call.

**LinkedIn contenteditable inputs**

LinkedIn message boxes use `contenteditable` divs, not `<input>` elements. Standard `type` will not work. Use `send_message`:
```json
{"kind": "send_message", "text": "Hello!", "input_selector": "div[contenteditable='true']"}
```

**Headless detection**

Some sites detect headless Chrome despite stealth. Run in headed mode (default) or explicitly set `NEOBROWSER_HEADLESS=0`. For sites behind Cloudflare or reCAPTCHA, headed mode with pre-persisted cookies is the most reliable approach.

## Known Limitations

- **reCAPTCHA v3**: scores 0.1 (bot) in headless mode. Headed mode with real cookies bypasses most checks.
- **Cloudflare Turnstile**: bypassed in stealth pipe mode (`NEOBROWSER_STEALTH=1`). Falls back to headed mode with pre-persisted cookies if needed.
- **One profile per session**: each MCP server instance uses one Chrome profile. Use `NEOBROWSER_PROFILE` to run parallel instances with different profiles.

## License

MIT
