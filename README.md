# NeoBrowser

Rust browser for AI agents. Sees the web as structured data, acts on it via MCP.

## Install

**Prerequisites**: Rust toolchain and Google Chrome (or Chromium).

```bash
# Install Rust (if not installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Build
git clone https://github.com/pitiflautico/neobrowser.git
cd neobrowser
cargo build --release

# Binary at target/release/neobrowser_rs
```

**Chrome**: NeoBrowser launches Chrome via CDP. It looks for Chrome at standard paths:
- macOS: `/Applications/Google Chrome.app/Contents/MacOS/Google Chrome`
- Linux: `google-chrome`, `chromium-browser`, or `chromium`

No chromedriver needed — communicates directly via Chrome DevTools Protocol (WebSocket).

## What it does

NeoBrowser gives AI agents a browser they can control via the [Model Context Protocol](https://modelcontextprotocol.io/). Instead of parsing raw HTML, agents get a **WOM (Web Object Model)** — a compressed, AI-friendly representation of the page with stable IDs for every interactive element.

Key capabilities:
- **Dual engine**: Light mode (HTTP + html5ever, no Chrome) and Chrome mode (full CDP browser)
- **Frame-aware**: Automatically detects framesets and targets the frame with actual content
- **WOM output**: Pages compressed 5-20x into structured nodes with stable IDs (`btn_001`, `fld_003`, `lnk_012`)
- **Delta tracking**: Only shows what changed between observations
- **Session management**: Cookie injection, Chrome profile reuse, OS keychain auth
- **browser_api**: HTTP requests from inside browser context — inherits cookies/session, 10x faster than navigating

## Architecture

```
┌──────────────────────────────────────────────┐
│                  MCP Server                   │
│              (JSON-RPC / stdio)               │
├──────────────────────────────────────────────┤
│  8 Tools:                                     │
│  browser_open  · browser_observe · browser_act│
│  browser_wait  · browser_tabs    · browser_auth│
│  browser_session · browser_api                │
├──────────────────────────────────────────────┤
│           Engine (raw CDP over WebSocket)      │
│  ┌─────────┐  ┌─────────┐  ┌──────────────┐  │
│  │  Light   │  │ Chrome  │  │ Frame-aware  │  │
│  │ reqwest  │  │   CDP   │  │ ACTIVE_DOC_JS│  │
│  │html5ever │  │WebSocket│  │  auto-detect │  │
│  └─────────┘  └─────────┘  └──────────────┘  │
├──────────────────────────────────────────────┤
│  WOM Builder → Vision → Delta → Compression   │
└──────────────────────────────────────────────┘
```

### Source files

| File | Purpose |
|---|---|
| `engine.rs` | Chrome CDP session — launch, navigate, click, type, scroll, eval, frame detection |
| `mcp.rs` | MCP server — 8 tool definitions + handlers, JSON-RPC loop |
| `wom.rs` | Web Object Model — DOM → structured nodes with stable IDs |
| `vision.rs` | Page classification — detects type (article, form, list, app) and state |
| `semantic.rs` | AX-tree-like text extraction from HTML |
| `delta.rs` | Diff between WOM revisions — only sends changes |
| `cdp.rs` | Raw CDP WebSocket client |
| `auth.rs` | Auth profiles, OS keychain credentials, session persistence |
| `main.rs` | CLI: fetch, browse, see, wom, session, mcp |

## MCP Tools

### browser_open
Open a URL. Returns WOM representation of the page.

```json
{ "url": "https://example.com", "mode": "chrome" }
```

Modes: `light` (HTTP only, fast), `chrome` (full browser), `auto` (try light, fall back to Chrome).

### browser_observe
See the current page state.

```json
{ "format": "content" }
```

Formats:
- `compact` — minimal JSON for fast loops
- `content` — readable text with stable IDs (default for agents)
- `full` — complete WOM JSON
- `delta` — only changes since last observation

### browser_act
Interact with the page.

```json
{ "kind": "click", "target": "Sign in" }
{ "kind": "click", "target": "btn_003" }
{ "kind": "type", "target": "fld_001", "text": "hello" }
{ "kind": "fill_form", "fields": {"username": "john", "password": "secret"} }
{ "kind": "eval", "text": "document.title" }
{ "kind": "select", "target": "Country", "value": "ES" }
{ "kind": "press", "key": "Enter" }
{ "kind": "scroll", "direction": "down" }
```

Target can be:
- **Text**: `"Sign in"` — fuzzy matches visible text, scored by relevance
- **WOM ID**: `"btn_003"` — direct DOM targeting from observe output

### browser_wait
Wait for conditions.

```json
{ "text_present": "Welcome" }
{ "seconds": 2 }
{ "text_absent": "Loading..." }
```

### browser_api
HTTP requests from inside the browser context. Inherits all cookies, session, and auth. Much faster than navigating.

```json
{ "url": "/api/data.json", "extract": "json" }
{ "url": "/page.html", "method": "POST", "body": "key=value", "extract": "text" }
```

Extract modes:
- `text` — parse HTML response, return innerText
- `json` — return parsed JSON
- `html` — return raw HTML
- `headers` — return response headers

This is the key tool for turning a browser session into an API client. Login once via browser, then make direct HTTP calls.

### browser_tabs
Manage tabs: `list`, `switch`, `close`.

### browser_session
Session management: `load_cookies`, `screenshot`, `reset`, `start_capture`, `network`, `console`, `dialogs`.

### browser_auth
Authentication with OS keychain integration: `profiles`, `add_profile`, `set_credential`, `login`, `resume_challenge`, `auto_session`, `extract_chrome`.

## Frame Support

Legacy sites using `<frameset>` (like Service Box PSA) render content inside `<frame>` elements. The top document is just the frameset — no interactive elements.

NeoBrowser automatically detects this: every DOM query (click, focus, observe, tag) uses `ACTIVE_DOC_JS` which resolves the frame with the most interactive content. No configuration needed.

## CLI Usage

```bash
# Light mode — HTTP only, no browser
neobrowser_rs fetch https://example.com

# Chrome mode — full browser
neobrowser_rs browse https://example.com

# Auto — tries light, falls back to Chrome
neobrowser_rs see https://news.ycombinator.com

# WOM output for AI
neobrowser_rs wom https://example.com --compact

# MCP server for Claude Code / AI agents
neobrowser_rs mcp

# Interactive session
neobrowser_rs session --url https://example.com
neobrowser_rs session --cookies cookies.json --url https://site.com
neobrowser_rs session --connect  # attach to running Chrome
neobrowser_rs session --profile /path/to/chrome/profile
```

## Claude Code Integration

Add to `~/.claude.json`:

```json
{
  "mcpServers": {
    "neobrowser": {
      "type": "stdio",
      "command": "/path/to/neobrowser_rs",
      "args": ["mcp"]
    }
  }
}
```

Then use from Claude Code:
```
browser_open url="https://example.com"
browser_observe format="content"
browser_act kind="click" target="Login"
browser_act kind="type" target="fld_001" text="user@email.com"
browser_api url="/api/search?q=test" extract="json"
```

## Dependencies

- **reqwest** — HTTP client (light mode)
- **html5ever** — HTML parser (Servo's, spec-compliant)
- **tokio-tungstenite** — WebSocket for CDP
- **tokio** — async runtime
- **clap** — CLI
- **keyring** — OS keychain (macOS Keychain, Linux Secret Service)
- **rusqlite** — Chrome cookie DB reading
- **totp-rs** — TOTP 2FA support

## License

MIT
