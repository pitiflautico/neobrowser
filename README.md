# NeoBrowser

A real browser for AI agents. Single Python file, no Selenium, no Playwright — direct Chrome DevTools Protocol over WebSocket.

NeoBrowser is an MCP (Model Context Protocol) server. It gives Claude, ChatGPT, or any MCP-compatible agent a persistent headless Chrome instance with your real browser session already loaded.

---

## What it does

When you connect NeoBrowser to your AI client:

- A headless Chrome starts in the background, isolated per MCP process
- Your cookies and localStorage sync from your real Chrome profile (Google domains excluded to avoid invalidating your login)
- The agent gets 19 tools: navigate, click, fill, extract, screenshot, search, and more
- Session persists across the entire conversation — no re-login per operation

---

## Architecture

| Component | Detail |
|---|---|
| Entry point | `tools/v3/neo-browser.py` (~1500 lines) |
| Transport | MCP over stdin/stdout JSON-RPC |
| Chrome control | Raw CDP via WebSocket (`websockets` only) |
| Session isolation | `~/.neorender/ghost-{pid}/` per process |
| Cookie sync | SQLite read from real Chrome (WAL-safe, read-only) |
| Dependencies | Python 3.10+, Chrome, `websockets` |

No Selenium. No Playwright. No chromedriver. One file, one dependency.

---

## Tools (19)

### HTTP — no Chrome required

| Tool | Description | Speed |
|---|---|---|
| `browse` | Fast HTTP fetch + smart parse | ~0.1–0.8s |
| `search` | DuckDuckGo web search | ~1s |

### Chrome browsing

| Tool | Description |
|---|---|
| `open` | Navigate to URL in Ghost Chrome |
| `read` | Extract content: markdown, accessibility tree, tweets, posts, comments, products, tables |
| `find` | Find element by text, CSS, XPath, or ARIA role |
| `click` | Click by text or CSS selector |
| `type` | Type in input — finds field by label, placeholder, name, or aria-label |
| `fill` | Smart fill: inputs, textareas, selects, checkboxes, radios |
| `submit` | Submit form |
| `scroll` | Scroll page |
| `wait` | Wait for element or text to appear |
| `login` | Automated email + password login |
| `extract` | Extract tables or links |
| `screenshot` | Capture PNG |
| `js` | Execute arbitrary JavaScript |

### AI Chat — dedicated persistent tabs

| Tool | Description |
|---|---|
| `gpt` | ChatGPT: `send`, `read_last`, `is_streaming`, `history` |
| `grok` | Grok: `send`, `read_last`, `is_streaming`, `history` |

### Meta

| Tool | Description |
|---|---|
| `plugin` | Run, list, or create YAML automation pipelines |
| `status` | Chrome state, open tabs, PIDs |

---

## Benchmarks

Measured wall-clock on macOS, Chrome headless. All overhead included.

```
21 tests, 21 passed — 2026-03-30

Cold start:      1.6s   (Chrome launch + cookie sync, one-time per session)
19 operations:   7.1s   (0.37s/op average)
ChatGPT:        33.0s   (mostly LLM server response time)
```

Per operation, warm:

```
browse (HTTP)        0.44s avg    ~1000 tokens/page
open (Chrome)        1.00s avg    ~1000 tokens/page
read                 0.10s avg    ~1025 tokens/page
fill / find / click  <0.01s       instant
screenshot           0.12s
search (DDG)         0.94s
```

`browse` vs `open` on the same URL:

```
browse example.com    0.11s     93 tokens   (HTTP fast path)
open   example.com    0.48s     32 tokens   (Chrome CDP)
browse hacker news    0.77s   1073 tokens   (HTTP, raw content)
open   hacker news    1.32s   1000 tokens   (Chrome, structured output)
```

Notes:
- Cold start is one-time per MCP session
- ChatGPT time is ~95% server response, not NeoBrowser overhead
- `open` times include readyState polling — no fixed sleeps
- Use `browse` for static content, `open` for SPAs and Cloudflare-protected pages

---

## Installation

```bash
# Install from npm
npx neobrowser

# Or install globally
npm install -g neobrowser
```

Add to your MCP config:

**Claude Code** (`~/.claude/mcp.json`):
```json
{
  "neo-browser": {
    "command": "npx",
    "args": ["-y", "neobrowser"]
  }
}
```

**Claude Desktop** (`~/Library/Application Support/Claude/claude_desktop_config.json`):
```json
{
  "mcpServers": {
    "neo-browser": {
      "command": "npx",
      "args": ["-y", "neobrowser"]
    }
  }
}
```

**Manual install (alternative):** Clone the repo and run `python3 tools/v3/neo-browser.py` directly.

Requirements: Node.js, Google Chrome installed.

---

## Session sync

On startup, NeoBrowser reads cookies from your real Chrome profile (SQLite, read-only, WAL-safe) and injects them into the Ghost Chrome instance. This means sites where you are already logged in will also be logged in for the agent — without you doing anything.

Google domains are excluded by default to avoid invalidating active Google sessions.

---

## YAML Plugins

Reusable automation pipelines defined in YAML. The `plugin` tool runs them by name.

Example: a pipeline that logs into a site, navigates to a dashboard, and extracts a table can be saved once and reused across sessions.

---

## Limitations

- **ChatGPT / Grok dedicated tabs**: `type` (insertText via CDP) does not work in these tabs due to how the editors handle input. The `gpt` and `grok` tools use a different injection method that does work.
- **ChatGPT response times**: highly variable (5–60s+) depending on server load. The 33s benchmark is typical but not guaranteed.
- **Cookie sync scope**: only cookies that Chrome has stored in SQLite at startup time are synced. Cookies set after startup in your real browser are not reflected.
- **Single Chrome instance per MCP process**: running multiple heavy parallel operations on the same tab will serialize. Multiple MCP processes each get their own isolated Chrome.
- **Cloudflare and anti-bot**: NeoBrowser uses a real Chrome UA with automation flags disabled. Most Cloudflare challenges pass. Some enterprise WAFs may still block.
- **macOS tested**: developed and benchmarked on macOS. Linux should work. Windows not tested.

---

## License

MIT
